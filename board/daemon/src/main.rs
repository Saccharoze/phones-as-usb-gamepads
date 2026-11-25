use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use stonesd::config::{self, Config};

#[derive(Parser)]
#[command(name = "stonesd", version, about = "Sending Stones board daemon")]
struct Args {
    #[arg(long, default_value = config::DEFAULT_PATH)]
    config: PathBuf,
    /// Record reports instead of writing /dev/hidgN (works on any OS)
    #[arg(long)]
    mock: bool,
    #[arg(long)]
    port: Option<u16>,
    /// TCP port of the browser pad page; 0 turns it off
    #[arg(long)]
    web_port: Option<u16>,
    #[arg(long)]
    no_mdns: bool,
    /// Write the pad's HID report descriptor to FILE and exit (install.sh does this)
    #[arg(long, value_name = "FILE")]
    write_descriptor: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        // journald shows escape codes literally; colour only on a terminal.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    if let Some(out) = &args.write_descriptor {
        return write_descriptor(out);
    }
    let mut config = Config::load(&args.config)?;
    config.mock |= args.mock;
    config.mdns &= !args.no_mdns;
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(port) = args.web_port {
        config.web_port = port;
    }
    stonesd::run(config, shutdown_signal()).await
}

/// The file `setup.sh` hands to configfs. `proto::descriptor::PAD` stays the only copy.
fn write_descriptor(out: &Path) -> Result<()> {
    std::fs::write(out, proto::descriptor::PAD).with_context(|| format!("writing {}", out.display()))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down: pads to neutral");
}
