//! The whole daemon in-process with mock pads, driven by a browser-like
//! client over a real WebSocket on localhost, next to a UDP phone.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use proto::frame::{ErrorCode, Frame};
use proto::{Button, PadState, report};
use stonesd::config::Config;
use stonesd::hid::MockSink;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

struct Daemon {
    udp: SocketAddr,
    web: SocketAddr,
    pads: Vec<MockSink>,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Daemon {
    async fn start() -> Daemon {
        let config = Config { mdns: false, ..Config::default() };
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (udp, web) = (socket.local_addr().unwrap(), listener.local_addr().unwrap());
        let pads: Vec<MockSink> = (0..config.pad_count as usize).map(|_| MockSink::default()).collect();
        let (stop, stopped) = oneshot::channel();
        let sinks = pads.clone();
        let task = tokio::spawn(async move {
            stonesd::serve(&config, socket, Some(listener), sinks, async { let _ = stopped.await; }).await;
        });
        Daemon { udp, web, pads, stop: Some(stop), task }
    }

    fn last(&self, pad: usize) -> (Instant, PadState) {
        let (at, last) = *self.pads[pad].written.lock().unwrap().last().expect("at least one report");
        (at, report::decode(&last).unwrap())
    }

    async fn stop(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        self.task.await.unwrap();
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Browser {
    ws: Socket,
    token: u64,
    seq: u16,
}

impl Browser {
    async fn open(daemon: &Daemon) -> Browser {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", daemon.web)).await.unwrap();
        Browser { ws, token: 0, seq: 0 }
    }

    async fn send(&mut self, frame: &Frame) {
        self.ws.send(Message::Binary(frame.encode().into())).await.unwrap();
    }

    async fn recv(&mut self) -> Frame {
        loop {
            let message = timeout(Duration::from_secs(1), self.ws.next()).await.expect("reply within 1 s");
            if let Message::Binary(bytes) = message.unwrap().unwrap() {
                return Frame::decode(&bytes).unwrap();
            }
        }
    }

    async fn hello(&mut self, slot: u8) -> Result<(), ErrorCode> {
        self.send(&Frame::Hello { slot, token: self.token, pin: None }).await;
        match self.recv().await {
            Frame::Welcome { token, .. } => {
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
}

async fn udp_status(daemon: &Daemon) -> serde_json::Value {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(&Frame::StatusReq.encode(), daemon.udp).await.unwrap();
    let mut buf = [0u8; 4096];
    let (n, _) = timeout(Duration::from_secs(1), socket.recv_from(&mut buf)).await.unwrap().unwrap();
    match Frame::decode(&buf[..n]).unwrap() {
        Frame::Status(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected STATUS, got {other:?}"),
    }
}

fn pressed(button: Button) -> PadState {
    PadState::NEUTRAL.with_button(button, true)
}

async fn settle() {
    sleep(Duration::from_millis(30)).await;
}

#[tokio::test]
async fn a_browser_is_one_more_phone() {
    let daemon = Daemon::start().await;
    let mut browser = Browser::open(&daemon).await;
    browser.hello(1).await.unwrap();
    browser.state(pressed(Button::A)).await;

    let phone = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    phone.send_to(&Frame::Hello { slot: 1, token: 0, pin: None }.encode(), daemon.udp).await.unwrap();
    let mut buf = [0u8; 64];
    let (n, _) = timeout(Duration::from_secs(1), phone.recv_from(&mut buf)).await.unwrap().unwrap();
    assert_eq!(Frame::decode(&buf[..n]).unwrap(), Frame::Error(ErrorCode::SlotTaken), "same slot rules");
    settle().await;

    assert_eq!(daemon.last(0).1, pressed(Button::A));
    let status = udp_status(&daemon).await;
    assert_eq!(status["slots"][0]["via"], "web");
    assert_eq!(status["slots"][0]["state"], "active");
    daemon.stop().await;
}

#[tokio::test]
async fn ping_and_status_answer_over_the_websocket() {
    let daemon = Daemon::start().await;
    let mut browser = Browser::open(&daemon).await;
    browser.send(&Frame::StatusReq).await;
    let Frame::Status(text) = browser.recv().await else { panic!("expected STATUS") };
    assert!(text.contains("\"pad_count\":4"));

    browser.hello(2).await.unwrap();
    let ping = proto::Ping { token: browser.token, id: 7, phone_time_ms: 1234 };
    browser.send(&Frame::Ping(ping)).await;
    assert_eq!(browser.recv().await, Frame::Pong(ping));
    browser.send(&Frame::State { token: 99, seq: 1, state: PadState::NEUTRAL }).await;
    assert_eq!(browser.recv().await, Frame::Error(ErrorCode::UnknownToken));
    daemon.stop().await;
}

#[tokio::test]
async fn closing_the_page_goes_neutral_at_once_and_keeps_the_slot() {
    let daemon = Daemon::start().await;
    let mut browser = Browser::open(&daemon).await;
    browser.hello(1).await.unwrap();
    browser.state(pressed(Button::B)).await;
    settle().await;
    let (pressed_at, _) = daemon.last(0);

    browser.ws.close(None).await.unwrap();
    settle().await;
    let (neutral_at, state) = daemon.last(0);
    assert_eq!(state, PadState::NEUTRAL);
    let gap = neutral_at.duration_since(pressed_at);
    assert!(gap < Duration::from_millis(200), "neutral after {gap:?}, before the 300 ms fail-safe");

    // A reloaded page sends its saved token and gets the same slot back.
    let mut reloaded = Browser::open(&daemon).await;
    reloaded.token = browser.token;
    reloaded.hello(1).await.unwrap();
    assert_eq!(reloaded.token, browser.token);
    reloaded.state(pressed(Button::Y)).await;
    settle().await;
    assert_eq!(daemon.last(0).1, pressed(Button::Y));
    daemon.stop().await;
}

#[tokio::test]
async fn a_page_from_another_site_is_refused() {
    let daemon = Daemon::start().await;
    let mut request = format!("ws://{}/ws", daemon.web).into_client_request().unwrap();
    request.headers_mut().insert("Origin", "http://example.com".parse().unwrap());
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403),
        other => panic!("expected 403, got {:?}", other.map(|(_, r)| r.status())),
    }

    let mut own = format!("ws://{}/ws", daemon.web).into_client_request().unwrap();
    own.headers_mut().insert("Origin", format!("http://{}", daemon.web).parse().unwrap());
    assert!(tokio_tungstenite::connect_async(own).await.is_ok(), "the board's own page");
    daemon.stop().await;
}

#[tokio::test]
async fn an_oversized_message_closes_the_connection() {
    let daemon = Daemon::start().await;
    let mut browser = Browser::open(&daemon).await;
    browser.ws.send(Message::Binary(vec![0u8; 1000].into())).await.unwrap();
    let end = timeout(Duration::from_secs(1), browser.ws.next()).await.expect("closed within 1 s");
    assert!(matches!(end, None | Some(Err(_)) | Some(Ok(Message::Close(_)))), "got {end:?}");
    daemon.stop().await;
}
