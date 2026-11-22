//! `cargo xtask build | deploy [--dev] | logs`, run from the repo root.
//!
//! The board is reached as `$STONES_SSH` (default `root@10.55.0.1`, the USB
//! bringup link). Needs `cargo-zigbuild` and `zig` on PATH for the build.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const TARGET: &str = "aarch64-unknown-linux-musl";
const STAGING: &str = "/tmp/sending-stones-staging";
const BINARY: &str = "stonesd";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dev = args.iter().any(|a| a == "--dev");
    match args.first().map(String::as_str) {
        Some("build") => build().map(|_| ()),
        Some("deploy") => deploy(dev),
        Some("logs") => logs(),
        _ => bail!("usage: cargo xtask build | deploy [--dev] | logs"),
    }
}

/// The board's Rust workspace, two levels up from dev/xtask.
fn board_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../board")
}

/// Every file in board/scripts, sorted so each deploy copies the same list.
fn scripts() -> Result<Vec<PathBuf>> {
    let dir = board_dir().join("scripts");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        files.push(entry?.path());
    }
    files.sort();
    Ok(files)
}

fn board() -> String {
    std::env::var("STONES_SSH").unwrap_or_else(|_| "root@10.55.0.1".to_owned())
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("running {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed: {status}");
    }
    Ok(())
}

fn build() -> Result<PathBuf> {
    let mut cargo = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cargo.current_dir(board_dir()).args(["zigbuild", "--release", "--target", TARGET, "-p", BINARY]);
    run(&mut cargo)?;
    Ok(board_dir().join("target").join(TARGET).join("release"))
}

fn deploy(dev: bool) -> Result<()> {
    let out = build()?;
    let board = board();
    run(Command::new("ssh").args([&board, &format!("rm -rf {STAGING} && mkdir -p {STAGING}")]))?;
    let mut files = vec![out.join(BINARY)];
    files.extend(scripts()?);
    run(Command::new("scp").arg("-q").args(&files).arg(format!("{board}:{STAGING}/")))?;
    // The gadget is only started, never restarted: over the USB link, a
    // restart would cut the SSH session running this very command.
    let script = format!(
        "set -e; {STAGING}/install.sh{flag} \
         && systemctl start sending-stones-gadget.service \
         && systemctl restart sending-stones.service \
         && sleep 1 && systemctl --no-pager --lines=0 status sending-stones.service | head -3",
        flag = if dev { " --dev" } else { "" }
    );
    run(Command::new("ssh").args([&board, &script]))
}

fn logs() -> Result<()> {
    run(Command::new("ssh").args(["-t", &board(), "journalctl -fu sending-stones -u sending-stones-gadget"]))
}
