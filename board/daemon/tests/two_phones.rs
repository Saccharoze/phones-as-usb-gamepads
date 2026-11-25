//! The whole daemon in-process with mock pads, driven by two fake phones
//! over real UDP on localhost. Real clock: the fail-safe is measured.

use std::net::SocketAddr;
use std::time::Duration;

use proto::frame::{ErrorCode, Frame};
use proto::{Button, Hat, PadState, report};
use stonesd::config::Config;
use stonesd::hid::{MockSink, Report};
use tokio::net::UdpSocket;
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep, timeout};

struct Daemon {
    addr: SocketAddr,
    pads: Vec<MockSink>,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Daemon {
    async fn start(config: Config) -> Daemon {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        let pads: Vec<MockSink> = (0..config.pad_count as usize).map(|_| MockSink::default()).collect();
        let (stop, stopped) = oneshot::channel();
        let sinks = pads.clone();
        let task = tokio::spawn(async move {
            stonesd::serve(&config, socket, None, sinks, async { let _ = stopped.await; }).await;
        });
        Daemon { addr, pads, stop: Some(stop), task }
    }

    fn reports(&self, pad: usize) -> Vec<(Instant, Report)> {
        self.pads[pad].written.lock().unwrap().clone()
    }

    fn last_state(&self, pad: usize) -> PadState {
        let (_, last) = *self.reports(pad).last().expect("at least one report");
        report::decode(&last).unwrap()
    }

    async fn stop(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        self.task.await.unwrap();
    }
}

struct Phone {
    socket: UdpSocket,
    board: SocketAddr,
    token: u64,
    seq: u16,
}

impl Phone {
    async fn new(board: SocketAddr) -> Phone {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        Phone { socket, board, token: 0, seq: 0 }
    }

    async fn send(&self, frame: &Frame) {
        self.socket.send_to(&frame.encode(), self.board).await.unwrap();
    }

    async fn recv(&self) -> Frame {
        let mut buf = [0u8; 2048];
        let (n, _) = timeout(Duration::from_secs(1), self.socket.recv_from(&mut buf))
            .await
            .expect("reply within 1 s")
            .unwrap();
        Frame::decode(&buf[..n]).unwrap()
    }

    async fn hello(&mut self, slot: u8, pin: Option<u16>) -> Result<(), ErrorCode> {
        self.send(&Frame::Hello { slot, token: self.token, pin }).await;
        match self.recv().await {
            Frame::Welcome { slot: s, token } => {
                assert_eq!(s, slot);
                self.token = token;
                Ok(())
            }
            Frame::Error(code) => Err(code),
            other => panic!("unexpected reply {other:?}"),
        }
    }

    async fn state(&mut self, state: PadState) {
        self.seq = self.seq.wrapping_add(1);
        self.send(&Frame::State { token: self.token, seq: self.seq, state }).await;
    }

    async fn status(&self) -> serde_json::Value {
        self.send(&Frame::StatusReq).await;
        match self.recv().await {
            Frame::Status(text) => serde_json::from_str(&text).unwrap(),
            other => panic!("expected STATUS, got {other:?}"),
        }
    }
}

fn pressed(button: Button) -> PadState {
    PadState::NEUTRAL.with_button(button, true)
}

/// Lets the daemon's writer tasks catch up with what was just sent.
async fn settle() {
    sleep(Duration::from_millis(30)).await;
}

#[tokio::test]
async fn two_phones_drive_their_own_pads() {
    let daemon = Daemon::start(Config { mdns: false, ..Config::default() }).await;
    let mut p1 = Phone::new(daemon.addr).await;
    let mut p2 = Phone::new(daemon.addr).await;
    p1.hello(1, None).await.unwrap();
    p2.hello(2, None).await.unwrap();
    assert_ne!(p1.token, p2.token);

    p1.state(pressed(Button::A)).await;
    let stick = PadState { left_x: 32767, hat: Hat::Left, ..pressed(Button::B) };
    p2.state(stick).await;
    settle().await;

    assert_eq!(daemon.last_state(0), pressed(Button::A));
    assert_eq!(daemon.last_state(1), stick);
    let pad1: Vec<Report> = daemon.reports(0).into_iter().map(|(_, r)| r).collect();
    assert_eq!(pad1, vec![report::encode(&PadState::NEUTRAL), report::encode(&pressed(Button::A))]);
    daemon.stop().await;
}

#[tokio::test]
async fn silent_phone_goes_neutral_within_300_ms() {
    let daemon = Daemon::start(Config { mdns: false, ..Config::default() }).await;
    let mut p1 = Phone::new(daemon.addr).await;
    p1.hello(1, None).await.unwrap();
    p1.state(pressed(Button::A)).await;
    settle().await;
    let pressed_at = daemon.reports(0).last().unwrap().0;

    sleep(Duration::from_millis(450)).await;
    let (neutral_at, last) = *daemon.reports(0).last().unwrap();
    assert_eq!(last, report::encode(&PadState::NEUTRAL), "fail-safe released A");
    let gap = neutral_at.duration_since(pressed_at);
    assert!(gap >= Duration::from_millis(290) && gap <= Duration::from_millis(340), "neutral after {gap:?}");
    daemon.stop().await;
}

#[tokio::test]
async fn heartbeats_keep_input_held() {
    let daemon = Daemon::start(Config { mdns: false, ..Config::default() }).await;
    let mut p1 = Phone::new(daemon.addr).await;
    p1.hello(1, None).await.unwrap();
    p1.state(pressed(Button::X)).await;
    for _ in 0..6 {
        sleep(Duration::from_millis(100)).await;
        p1.seq += 1;
        p1.send(&Frame::Heartbeat { token: p1.token, seq: p1.seq }).await;
    }
    assert_eq!(daemon.last_state(0), pressed(Button::X), "held for 600 ms");
    daemon.stop().await;
}

#[tokio::test]
async fn stale_and_foreign_packets_are_ignored() {
    let daemon = Daemon::start(Config { mdns: false, ..Config::default() }).await;
    let mut p1 = Phone::new(daemon.addr).await;
    p1.hello(1, None).await.unwrap();
    p1.seq = 10;
    p1.state(pressed(Button::Y)).await;
    p1.send(&Frame::State { token: p1.token, seq: 5, state: pressed(Button::B) }).await;
    settle().await;
    assert_eq!(daemon.last_state(0), pressed(Button::Y), "older seq dropped");

    let stranger = Phone::new(daemon.addr).await;
    stranger.send(&Frame::State { token: 12345, seq: 1, state: pressed(Button::A) }).await;
    assert_eq!(stranger.recv().await, Frame::Error(ErrorCode::UnknownToken));
    stranger.send(&Frame::State { token: 12345, seq: 2, state: pressed(Button::A) }).await;
    stranger.socket.send_to(b"garbage", daemon.addr).await.unwrap();
    settle().await;
    assert_eq!(daemon.last_state(0), pressed(Button::Y));

    let status = p1.status().await;
    assert_eq!(status["stale_frames"], 1);
    assert_eq!(status["bad_frames"], 1);
    daemon.stop().await;
}

#[tokio::test]
async fn slot_rules_and_status_over_udp() {
    let config = Config { mdns: false, pin: Some(4321), ..Config::default() };
    let no_such_slot = config.pad_count + 1;
    let daemon = Daemon::start(config).await;
    let mut owner = Phone::new(daemon.addr).await;
    let mut intruder = Phone::new(daemon.addr).await;
    assert_eq!(owner.hello(1, None).await, Err(ErrorCode::BadPin));
    owner.hello(1, Some(4321)).await.unwrap();
    assert_eq!(intruder.hello(1, Some(1111)).await, Err(ErrorCode::BadPin));
    assert_eq!(intruder.hello(no_such_slot, Some(4321)).await, Err(ErrorCode::NoSuchSlot));

    let status = intruder.status().await;
    let text = status.to_string();
    assert!(!text.contains(&owner.token.to_string()), "STATUS must never leak tokens");
    assert_eq!(status["pin_required"], true);
    assert_eq!(status["slots"][0]["state"], "active");
    assert_eq!(status["slots"][1]["state"], "free");

    owner.state(pressed(Button::Menu)).await;
    owner.send(&Frame::Release { token: owner.token }).await;
    settle().await;
    assert_eq!(daemon.last_state(0), PadState::NEUTRAL, "release goes neutral");
    assert_eq!(intruder.status().await["slots"][0]["state"], "free");
    daemon.stop().await;
}

#[tokio::test]
async fn resume_from_new_address_keeps_slot() {
    let daemon = Daemon::start(Config { mdns: false, ..Config::default() }).await;
    let mut phone = Phone::new(daemon.addr).await;
    phone.hello(1, None).await.unwrap();
    let token = phone.token;

    // Same session, new socket: what a phone sees after a Wi-Fi reconnect.
    let mut moved = Phone::new(daemon.addr).await;
    moved.token = token;
    moved.hello(1, None).await.unwrap();
    assert_eq!(moved.token, token);
    moved.state(pressed(Button::LB)).await;
    settle().await;
    assert_eq!(daemon.last_state(0), pressed(Button::LB));

    let mut other = Phone::new(daemon.addr).await;
    assert_eq!(other.hello(1, None).await, Err(ErrorCode::SlotTaken));
    daemon.stop().await;
}

#[tokio::test]
async fn shutdown_leaves_pads_neutral() {
    let daemon = Daemon::start(Config { mdns: false, ..Config::default() }).await;
    let mut phone = Phone::new(daemon.addr).await;
    phone.hello(2, None).await.unwrap();
    phone.state(pressed(Button::RB)).await;
    settle().await;
    let pads = daemon.pads.clone();
    daemon.stop().await;
    let (_, last) = *pads[1].written.lock().unwrap().last().unwrap();
    assert_eq!(last, report::encode(&PadState::NEUTRAL));
}
