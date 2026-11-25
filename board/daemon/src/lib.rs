//! Sending Stones board daemon: phones in over UDP or WebSocket, HID reports out.

pub mod config;
pub mod discovery;
pub mod failsafe;
pub mod hid;
pub mod metrics;
pub mod net;
pub mod peer;
pub mod slots;
pub mod web;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, watch};

use crate::config::Config;
use crate::hid::{MockSink, PadSink, PadUpdate};
use crate::metrics::PadMetrics;
use crate::net::{PadOutput, Server};
use crate::slots::Slots;

/// Frames waiting between the WebSocket connections and the slot task.
const WEB_EVENT_QUEUE: usize = 256;

pub async fn bind(port: u16) -> Result<UdpSocket> {
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    UdpSocket::bind(addr).await.with_context(|| format!("binding UDP {addr}"))
}

/// Runs the daemon with the sinks the config asks for.
pub async fn run(config: Config, shutdown: impl Future<Output = ()>) -> Result<()> {
    let socket = bind(config.port).await?;
    let web = match config.web_port {
        0 => None,
        port => web::bind(port)
            .await
            .map_err(|e| tracing::warn!(error = %e, "browser pad unavailable"))
            .ok(),
    };
    let _advert = if config.mdns {
        discovery::advertise(config.port, config.pad_count, config.pin.is_some())
            .map_err(|e| tracing::warn!(error = %e, "mDNS unavailable; phones need the address"))
            .ok()
    } else {
        None
    };
    if config.mock {
        let sinks = (0..config.pad_count as usize).map(|pad| MockSink { log: true, pad, ..MockSink::default() });
        serve(&config, socket, web, sinks.collect(), shutdown).await;
        return Ok(());
    }
    run_hidg(config, socket, web, shutdown).await
}

#[cfg(target_os = "linux")]
async fn run_hidg(
    config: Config,
    socket: UdpSocket,
    web: Option<TcpListener>,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    let sinks = (0..config.pad_count as usize).map(|pad| hid::HidgSink::new(config.hid_path(pad)));
    serve(&config, socket, web, sinks.collect(), shutdown).await;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn run_hidg(_: Config, _: UdpSocket, _: Option<TcpListener>, _: impl Future<Output = ()>) -> Result<()> {
    anyhow::bail!("real pads need the Linux USB gadget; use --mock on this OS")
}

/// Spawns one writer per sink, and the web server when `web` is given, then
/// serves until `shutdown`. Public so tests can run the whole daemon
/// in-process against mock sinks.
pub async fn serve<S: PadSink>(
    config: &Config,
    socket: UdpSocket,
    web: Option<TcpListener>,
    sinks: Vec<S>,
    shutdown: impl Future<Output = ()>,
) {
    let mut pads = Vec::new();
    let mut writers = Vec::new();
    for (pad, sink) in sinks.into_iter().enumerate() {
        let (updates, rx) = watch::channel(PadUpdate::neutral());
        let metrics = Arc::new(PadMetrics::default());
        writers.push(tokio::spawn(hid::run_writer(pad, rx, sink, metrics.clone())));
        pads.push(PadOutput { updates, metrics });
    }
    let slots = Slots::new(pads.len(), config.pin, config.reservation());
    if let Ok(addr) = socket.local_addr() {
        tracing::info!(%addr, pads = pads.len(), mock = config.mock, "stonesd listening");
    }
    // Without a web server the sender is dropped here, and the server stops
    // waiting for web events.
    let (events, web_events) = mpsc::channel(WEB_EVENT_QUEUE);
    let web_server = web.map(|listener| {
        if let Ok(addr) = listener.local_addr() {
            tracing::info!(%addr, "browser pad listening");
        }
        tokio::spawn(web::serve(listener, events))
    });
    Server::new(socket, web_events, slots, pads, config.failsafe()).run(shutdown).await;
    if let Some(web_server) = web_server {
        web_server.abort();
    }
    for writer in writers {
        let _ = writer.await;
    }
}
