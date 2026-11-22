//! Report output. The writer task holds only the newest state: if the host
//! is slow, intermediate states are skipped, never queued.

use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::{PadState, REPORT_LEN, report};
use tokio::sync::watch;
use tokio::time::{Instant, sleep};

use crate::metrics::PadMetrics;

pub type Report = [u8; REPORT_LEN];

pub trait PadSink: Send + 'static {
    fn write(&mut self, report: &Report) -> impl Future<Output = io::Result<()>> + Send;
}

/// What the network side publishes for one pad.
#[derive(Debug, Clone, Copy)]
pub struct PadUpdate {
    pub state: PadState,
    /// When the phone packet behind this state arrived. None for states the
    /// daemon makes itself (startup, fail-safe, release): they have no packet,
    /// so they stay out of the receive-to-write metric.
    pub received_at: Option<Instant>,
}

impl PadUpdate {
    pub fn neutral() -> PadUpdate {
        PadUpdate { state: PadState::NEUTRAL, received_at: None }
    }
}

/// Records every report with its time; used by `--mock` and the tests.
#[derive(Clone, Default)]
pub struct MockSink {
    pub written: Arc<Mutex<Vec<(Instant, Report)>>>,
    pub log: bool,
    pub pad: usize,
}

impl PadSink for MockSink {
    async fn write(&mut self, report: &Report) -> io::Result<()> {
        if self.log {
            tracing::info!(pad = self.pad + 1, report = %hex(report), "mock report");
        }
        self.written.lock().expect("mock lock").push((Instant::now(), *report));
        Ok(())
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const MAX_BACKOFF: Duration = Duration::from_secs(1);

/// Writes the newest state whenever it changes. On a write error (PC off,
/// cable out) it logs once, retries with backoff up to 1 s, and always
/// retries with the newest state rather than the one that failed.
pub async fn run_writer<S: PadSink>(
    pad: usize,
    mut updates: watch::Receiver<PadUpdate>,
    mut sink: S,
    metrics: Arc<PadMetrics>,
) {
    let mut last_written: Option<Report> = None;
    loop {
        let update = *updates.borrow_and_update();
        let report = report::encode(&update.state);
        if last_written != Some(report) {
            match write_until_ok(pad, &mut sink, &mut updates, &metrics, update, report).await {
                Some(written) => last_written = Some(written),
                None => return,
            }
        }
        if updates.changed().await.is_err() {
            return;
        }
    }
}

/// Returns the report actually written (newer than `report` if the state
/// changed during retries), or None once the network side has shut down.
async fn write_until_ok<S: PadSink>(
    pad: usize,
    sink: &mut S,
    updates: &mut watch::Receiver<PadUpdate>,
    metrics: &PadMetrics,
    mut update: PadUpdate,
    mut report: Report,
) -> Option<Report> {
    let mut backoff = Duration::from_millis(50);
    let mut failing = false;
    loop {
        match sink.write(&report).await {
            Ok(()) => {
                metrics.record_write(update.received_at.map(|at| at.elapsed()));
                if failing {
                    tracing::info!(pad = pad + 1, "pad writes recovered");
                }
                return Some(report);
            }
            Err(e) => {
                metrics.record_error();
                if !failing {
                    tracing::warn!(pad = pad + 1, error = %e, "pad write failed; retrying quietly");
                    failing = true;
                }
            }
        }
        tokio::select! {
            _ = sleep(backoff) => {}
            changed = updates.changed() => if changed.is_err() { return None },
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
        update = *updates.borrow_and_update();
        report = report::encode(&update.state);
    }
}

#[cfg(target_os = "linux")]
pub use linux::HidgSink;

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::File;
    use std::io::{self, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::PathBuf;

    use tokio::io::unix::AsyncFd;

    use super::{PadSink, Report};

    /// `/dev/hidgN`, opened non-blocking and polled for writability, so a
    /// host that stops reading parks this task instead of a runtime thread.
    pub struct HidgSink {
        path: PathBuf,
        fd: Option<AsyncFd<File>>,
    }

    impl HidgSink {
        pub fn new(path: PathBuf) -> HidgSink {
            HidgSink { path, fd: None }
        }

        fn open(&self) -> io::Result<AsyncFd<File>> {
            let file = File::options().write(true).custom_flags(libc::O_NONBLOCK).open(&self.path)?;
            // Deprecated because a type's fd could change while registered; a
            // File owns one fd for life, so this stays sound. Its replacement,
            // `register`, is `unsafe`, which this project forbids.
            #[allow(deprecated)]
            AsyncFd::new(file)
        }

        async fn write_once(fd: &AsyncFd<File>, report: &Report) -> io::Result<()> {
            loop {
                let mut guard = fd.writable().await?;
                match guard.try_io(|inner| inner.get_ref().write(report)) {
                    Ok(Ok(n)) if n == report.len() => return Ok(()),
                    Ok(Ok(n)) => return Err(io::Error::other(format!("short write: {n} bytes"))),
                    Ok(Err(e)) => return Err(e),
                    Err(_would_block) => continue,
                }
            }
        }
    }

    impl PadSink for HidgSink {
        async fn write(&mut self, report: &Report) -> io::Result<()> {
            if self.fd.is_none() {
                self.fd = Some(self.open()?);
            }
            let fd = self.fd.as_ref().expect("opened above");
            let result = Self::write_once(fd, report).await;
            if result.is_err() {
                // Reopen next time: the gadget may have been rebuilt underneath us.
                self.fd = None;
            }
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Button;

    struct FlakySink {
        fail_first: usize,
        inner: MockSink,
    }

    impl PadSink for FlakySink {
        async fn write(&mut self, report: &Report) -> io::Result<()> {
            if self.fail_first > 0 {
                self.fail_first -= 1;
                return Err(io::Error::other("host gone"));
            }
            self.inner.write(report).await
        }
    }

    fn pressed(button: Button) -> PadUpdate {
        PadUpdate { state: PadState::NEUTRAL.with_button(button, true), received_at: Some(Instant::now()) }
    }

    #[tokio::test(start_paused = true)]
    async fn writes_on_change_only() {
        let (tx, rx) = watch::channel(PadUpdate::neutral());
        let sink = MockSink::default();
        let metrics = Arc::new(PadMetrics::default());
        let task = tokio::spawn(run_writer(0, rx, sink.clone(), metrics.clone()));
        tokio::task::yield_now().await;
        tx.send(pressed(Button::A)).unwrap();
        tokio::task::yield_now().await;
        tx.send(pressed(Button::A)).unwrap();
        tokio::task::yield_now().await;
        drop(tx);
        task.await.unwrap();
        let written = sink.written.lock().unwrap();
        assert_eq!(written.len(), 2, "neutral, then A once");
        assert_eq!(metrics.reports_written(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_with_the_newest_state_after_errors() {
        let (tx, rx) = watch::channel(pressed(Button::A));
        let inner = MockSink::default();
        let metrics = Arc::new(PadMetrics::default());
        let sink = FlakySink { fail_first: 3, inner: inner.clone() };
        let task = tokio::spawn(run_writer(0, rx, sink, metrics.clone()));
        tokio::task::yield_now().await;
        tx.send(pressed(Button::B)).unwrap();
        sleep(Duration::from_secs(5)).await;
        drop(tx);
        task.await.unwrap();
        let written = inner.written.lock().unwrap();
        let reports: Vec<Report> = written.iter().map(|(_, r)| *r).collect();
        assert_eq!(reports, vec![report::encode(&pressed(Button::B).state)], "A was skipped, B written once");
        assert_eq!(metrics.write_errors(), 3);
    }
}
