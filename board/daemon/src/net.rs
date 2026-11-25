//! The phone side. One task owns the UDP socket, the browsers' connections,
//! the slots and the fail-safe timer, so session state needs no locks; pads
//! are fed through `watch`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use proto::frame::{ErrorCode, Frame, Ping, seq_is_newer};
use proto::PadState;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, sleep, sleep_until};

use crate::failsafe::{self, Watch};
use crate::hid::PadUpdate;
use crate::metrics::PadMetrics;
use crate::peer::Peer;
use crate::slots::{Claim, Slots};
use crate::web::WebEvent;

const ERROR_REPLY_INTERVAL: Duration = Duration::from_secs(1);
const EXPIRY_CHECK: Duration = Duration::from_secs(1);
/// Bounds the per-source rate limiter if something sprays addresses at us.
const MAX_TRACKED_SOURCES: usize = 256;

pub struct PadOutput {
    pub updates: watch::Sender<PadUpdate>,
    pub metrics: Arc<PadMetrics>,
}

pub struct Server {
    socket: UdpSocket,
    web: mpsc::Receiver<WebEvent>,
    /// Reply queue of each open WebSocket, by connection id.
    web_replies: HashMap<u64, mpsc::Sender<Vec<u8>>>,
    slots: Slots,
    pads: Vec<PadOutput>,
    failsafe_window: Duration,
    last_error_reply: HashMap<Peer, Instant>,
    bad_frames: u64,
    stale_frames: u64,
    started: Instant,
}

impl Server {
    pub fn new(
        socket: UdpSocket,
        web: mpsc::Receiver<WebEvent>,
        slots: Slots,
        pads: Vec<PadOutput>,
        failsafe_window: Duration,
    ) -> Server {
        assert_eq!(slots.count(), pads.len(), "one pad output per slot");
        Server {
            socket,
            web,
            web_replies: HashMap::new(),
            slots,
            pads,
            failsafe_window,
            last_error_reply: HashMap::new(),
            bad_frames: 0,
            stale_frames: 0,
            started: Instant::now(),
        }
    }

    /// Serves until `shutdown` resolves, then leaves every pad neutral.
    pub async fn run(mut self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        let mut buf = [0u8; 1500];
        let mut expiry = tokio::time::interval(EXPIRY_CHECK);
        // False once the web server is gone (or never started).
        let mut web_open = true;
        loop {
            let deadline = failsafe::next_deadline(&self.watches(), self.failsafe_window);
            tokio::select! {
                received = self.socket.recv_from(&mut buf) => match received {
                    Ok((len, from)) => self.handle(&buf[..len], Peer::Udp(from)).await,
                    // Windows reports ICMP "port unreachable" from an earlier send here.
                    Err(e) => tracing::debug!(error = %e, "recv error ignored"),
                },
                event = self.web.recv(), if web_open => match event {
                    Some(event) => self.on_web(event).await,
                    None => web_open = false,
                },
                _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.fire_failsafe();
                }
                _ = expiry.tick() => self.expire_reservations(),
                _ = &mut shutdown => break,
            }
        }
        for pad in 0..self.pads.len() {
            self.publish(pad, PadState::NEUTRAL, None);
        }
        // Give the writers a moment to flush the neutral reports.
        sleep(Duration::from_millis(100)).await;
    }

    async fn on_web(&mut self, event: WebEvent) {
        match event {
            WebEvent::Open { id, replies } => {
                self.web_replies.insert(id, replies);
            }
            WebEvent::Received { peer, bytes } => self.handle(&bytes, peer).await,
            WebEvent::Closed { id } => {
                self.web_replies.remove(&id);
                // The page is gone: release its input now rather than after the
                // fail-safe window. The slot stays reserved for a resume.
                for pad in 0..self.pads.len() {
                    let held = self.slots.get(pad).is_some_and(|s| matches!(s.peer, Peer::Web { id: i, .. } if i == id));
                    if held && self.current_state(pad) != PadState::NEUTRAL {
                        tracing::info!(slot = pad + 1, "browser closed: neutral");
                        self.publish(pad, PadState::NEUTRAL, None);
                    }
                }
            }
        }
    }

    async fn handle(&mut self, bytes: &[u8], from: Peer) {
        let now = Instant::now();
        let frame = match Frame::decode(bytes) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::debug!(%from, error = %e, "bad frame");
                return self.bad_frame(from, now).await;
            }
        };
        match frame {
            Frame::Hello { slot, token, pin } => self.on_hello(slot, token, pin, from, now).await,
            Frame::State { token, seq, state } => self.on_state(token, seq, state, from, now).await,
            Frame::Heartbeat { token, seq } => self.on_heartbeat(token, seq, from, now).await,
            Frame::Ping(ping) => self.on_ping(ping, from, now).await,
            Frame::Stats { token, rtt_p50_ms, rtt_p95_ms } => {
                match self.slots.touch(token, from, now) {
                    Some((_, session)) => {
                        session.rtt_p50_ms = Some(rtt_p50_ms);
                        session.rtt_p95_ms = Some(rtt_p95_ms);
                    }
                    None => self.unknown_token(from, now).await,
                }
            }
            Frame::Release { token } => {
                if let Some(pad) = self.slots.release(token) {
                    tracing::info!(slot = pad + 1, %from, "slot released");
                    self.publish(pad, PadState::NEUTRAL, None);
                }
            }
            Frame::StatusReq => {
                let status = Frame::Status(self.status_json(now).to_string());
                self.send(&status, from).await;
            }
            // Board-to-phone frames have no business arriving here.
            Frame::Welcome { .. } | Frame::Error(_) | Frame::Pong(_) | Frame::Status(_) => {
                self.bad_frame(from, now).await
            }
        }
    }

    async fn on_hello(&mut self, slot: u8, token: u64, pin: Option<u16>, from: Peer, now: Instant) {
        match self.slots.claim(slot, token, pin, from, now, new_token()) {
            Ok((token, claim)) => {
                tracing::info!(slot, %from, ?claim, "slot claimed");
                if matches!(claim, Claim::New | Claim::TakenOver) {
                    self.publish(slot as usize - 1, PadState::NEUTRAL, None);
                }
                self.send(&Frame::Welcome { slot, token }, from).await;
            }
            Err(code) => {
                tracing::info!(slot, %from, ?code, "claim refused");
                self.send(&Frame::Error(code), from).await;
            }
        }
    }

    async fn on_state(&mut self, token: u64, seq: u16, state: PadState, from: Peer, now: Instant) {
        match self.accept_sequenced(token, seq, from, now) {
            Accepted::Fresh(pad) => self.publish(pad, state, Some(now)),
            Accepted::Stale => self.stale_frames += 1,
            Accepted::UnknownToken => self.unknown_token(from, now).await,
        }
    }

    async fn on_heartbeat(&mut self, token: u64, seq: u16, from: Peer, now: Instant) {
        match self.accept_sequenced(token, seq, from, now) {
            Accepted::Fresh(_) => {}
            Accepted::Stale => self.stale_frames += 1,
            Accepted::UnknownToken => self.unknown_token(from, now).await,
        }
    }

    async fn on_ping(&mut self, ping: Ping, from: Peer, now: Instant) {
        if self.slots.touch(ping.token, from, now).is_some() {
            self.send(&Frame::Pong(ping), from).await;
        } else {
            self.unknown_token(from, now).await;
        }
    }

    /// STATE and HEARTBEAT share one sequence counter per session.
    fn accept_sequenced(&mut self, token: u64, seq: u16, from: Peer, now: Instant) -> Accepted {
        let Some((pad, session)) = self.slots.touch(token, from, now) else {
            return Accepted::UnknownToken;
        };
        if session.last_seq.is_some_and(|last| !seq_is_newer(seq, last)) {
            return Accepted::Stale;
        }
        session.last_seq = Some(seq);
        Accepted::Fresh(pad)
    }

    fn publish(&mut self, pad: usize, state: PadState, received_at: Option<Instant>) {
        self.pads[pad].updates.send_replace(PadUpdate { state, received_at });
    }

    fn current_state(&self, pad: usize) -> PadState {
        self.pads[pad].updates.borrow().state
    }

    fn watches(&self) -> Vec<Option<Watch>> {
        (0..self.pads.len())
            .map(|pad| {
                self.slots.get(pad).map(|s| Watch {
                    last_seen: s.last_seen,
                    active_input: self.current_state(pad) != PadState::NEUTRAL,
                })
            })
            .collect()
    }

    fn fire_failsafe(&mut self) {
        let now = Instant::now();
        for pad in failsafe::due(&self.watches(), now, self.failsafe_window) {
            tracing::info!(slot = pad + 1, "phone silent: fail-safe neutral");
            self.publish(pad, PadState::NEUTRAL, None);
        }
    }

    fn expire_reservations(&mut self) {
        let now = Instant::now();
        for pad in self.slots.expire(now) {
            tracing::info!(slot = pad + 1, "reservation expired: slot free");
            self.publish(pad, PadState::NEUTRAL, None);
        }
        self.last_error_reply.retain(|_, at| now.duration_since(*at) < ERROR_REPLY_INTERVAL);
    }

    async fn unknown_token(&mut self, from: Peer, now: Instant) {
        self.rate_limited_error(ErrorCode::UnknownToken, from, now).await;
    }

    async fn bad_frame(&mut self, from: Peer, now: Instant) {
        self.bad_frames += 1;
        self.rate_limited_error(ErrorCode::BadFrame, from, now).await;
    }

    async fn rate_limited_error(&mut self, code: ErrorCode, from: Peer, now: Instant) {
        let recent = self
            .last_error_reply
            .get(&from)
            .is_some_and(|at| now.duration_since(*at) < ERROR_REPLY_INTERVAL);
        if recent || self.last_error_reply.len() >= MAX_TRACKED_SOURCES {
            return;
        }
        self.last_error_reply.insert(from, now);
        self.send(&Frame::Error(code), from).await;
    }

    async fn send(&self, frame: &Frame, to: Peer) {
        match to {
            Peer::Udp(addr) => {
                if let Err(e) = self.socket.send_to(&frame.encode(), addr).await {
                    tracing::debug!(%to, error = %e, "send failed");
                }
            }
            Peer::Web { id, .. } => {
                // A full queue means the page isn't reading: the reply is
                // lost, as a UDP packet could be.
                if let Some(replies) = self.web_replies.get(&id) {
                    let _ = replies.try_send(frame.encode());
                }
            }
        }
    }

    /// Never includes tokens: anyone on the LAN may ask for this.
    fn status_json(&self, now: Instant) -> serde_json::Value {
        let slots: Vec<_> = (0..self.pads.len()).map(|pad| self.slot_json(pad, now)).collect();
        serde_json::json!({
            "version": proto::frame::VERSION,
            "pad_count": self.pads.len(),
            "pin_required": self.slots.pin_required(),
            "uptime_s": now.duration_since(self.started).as_secs(),
            "bad_frames": self.bad_frames,
            "stale_frames": self.stale_frames,
            "slots": slots,
        })
    }

    fn slot_json(&self, pad: usize, now: Instant) -> serde_json::Value {
        let metrics = &self.pads[pad].metrics;
        let latency = metrics.receive_to_write_us().map(|(p50, p95)| serde_json::json!({ "p50": p50, "p95": p95 }));
        let mut slot = serde_json::json!({
            "slot": pad + 1,
            "state": "free",
            "reports_written": metrics.reports_written(),
            "write_errors": metrics.write_errors(),
            "receive_to_write_us": latency,
        });
        if let Some(session) = self.slots.get(pad) {
            let silent = now.duration_since(session.last_seen);
            slot["state"] = if silent < self.failsafe_window { "active" } else { "reserved" }.into();
            slot["phone"] = session.peer.addr().to_string().into();
            slot["via"] = session.peer.via().into();
            slot["silent_ms"] = (silent.as_millis() as u64).into();
            slot["rtt_p50_ms"] = session.rtt_p50_ms.into();
            slot["rtt_p95_ms"] = session.rtt_p95_ms.into();
        }
        slot
    }
}

enum Accepted {
    Fresh(usize),
    Stale,
    UnknownToken,
}

/// Zero means "new session" on the wire, so it is never handed out.
fn new_token() -> u64 {
    loop {
        let token: u64 = rand::random();
        if token != 0 {
            return token;
        }
    }
}
