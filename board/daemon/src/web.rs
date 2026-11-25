//! The browser side. A WebSocket carries the same frames as UDP, one frame
//! per binary message, byte for byte. Each connection forwards its frames to
//! the task that owns the slots, so a browser is one more phone: same slots,
//! PIN, tokens and fail-safe.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::timeout;

use crate::peer::Peer;

/// One per player is the normal case; the cap bounds what a stray client can hold.
const MAX_CONNECTIONS: usize = 16;
/// The longest phone-to-board frame is STATE, 27 bytes.
const MAX_MESSAGE: usize = 64;
/// A page sends a PING every second while playing and a STATUS_REQ every
/// 1.5 s while picking a player; a connection silent for longer is dropped.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const REPLY_QUEUE: usize = 16;

/// What the connections tell the task that owns the slots.
#[derive(Debug)]
pub enum WebEvent {
    Open { id: u64, replies: mpsc::Sender<Vec<u8>> },
    Received { peer: Peer, bytes: Vec<u8> },
    Closed { id: u64 },
}

#[derive(Clone)]
struct App {
    events: mpsc::Sender<WebEvent>,
    connections: Arc<Semaphore>,
    next_id: Arc<AtomicU64>,
}

pub async fn bind(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    TcpListener::bind(addr).await.with_context(|| format!("binding TCP {addr}"))
}

/// Serves until the task is aborted.
pub async fn serve(listener: TcpListener, events: mpsc::Sender<WebEvent>) {
    let app = App {
        events,
        connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        next_id: Arc::new(AtomicU64::new(1)),
    };
    let router = Router::new().route("/ws", get(upgrade)).with_state(app);
    let service = router.into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service).await {
        tracing::warn!(error = %e, "web server stopped");
    }
}

async fn upgrade(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !same_origin(&headers) {
        tracing::info!(%addr, "WebSocket from another site refused");
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = app.connections.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.max_message_size(MAX_MESSAGE)
        .max_frame_size(MAX_MESSAGE)
        .on_upgrade(move |socket| connection(socket, addr, app, permit))
}

/// A browser sends `Origin` with every WebSocket handshake. Only the board's
/// own page may open one: a page from another site, open in a player's
/// browser, must not drive a pad. A client without `Origin` isn't a browser,
/// and could send UDP anyway.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    match (origin.to_str().ok().and_then(|o| o.strip_prefix("http://")), host) {
        (Some(origin_host), Some(host)) => origin_host.eq_ignore_ascii_case(host),
        _ => false,
    }
}

async fn connection(socket: WebSocket, addr: SocketAddr, app: App, _permit: OwnedSemaphorePermit) {
    let id = app.next_id.fetch_add(1, Ordering::Relaxed);
    let peer = Peer::Web { id, addr };
    let (replies, mut outgoing) = mpsc::channel(REPLY_QUEUE);
    if app.events.send(WebEvent::Open { id, replies }).await.is_err() {
        return;
    }
    tracing::debug!(%peer, "WebSocket open");
    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            incoming = timeout(IDLE_TIMEOUT, stream.next()) => match incoming {
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    let event = WebEvent::Received { peer, bytes: bytes.to_vec() };
                    if app.events.send(event).await.is_err() {
                        break;
                    }
                }
                // The library answers pings itself; text frames mean nothing here.
                Ok(Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_)))) => {}
                Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => break,
            },
            reply = outgoing.recv() => match reply {
                Some(bytes) => {
                    if sink.send(Message::Binary(bytes.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    tracing::debug!(%peer, "WebSocket closed");
    let _ = app.events.send(WebEvent::Closed { id }).await;
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(origin: Option<&'static str>, host: &'static str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static(host));
        if let Some(origin) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_static(origin));
        }
        h
    }

    #[test]
    fn only_the_boards_own_page_opens_a_websocket() {
        assert!(same_origin(&headers(Some("http://192.0.2.20:8080"), "192.0.2.20:8080")));
        assert!(same_origin(&headers(Some("http://Stones.local"), "stones.local")));
        assert!(same_origin(&headers(None, "192.0.2.20:8080")), "not a browser");
        assert!(!same_origin(&headers(Some("http://example.com"), "192.0.2.20:8080")));
        assert!(!same_origin(&headers(Some("http://192.0.2.20:9999"), "192.0.2.20:8080")));
        assert!(!same_origin(&headers(Some("null"), "192.0.2.20:8080")));
    }
}
