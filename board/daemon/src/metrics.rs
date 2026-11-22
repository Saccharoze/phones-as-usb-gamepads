use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Enough samples for a stable p95 while staying a few KB per pad.
const WINDOW: usize = 1024;

#[derive(Default)]
pub struct PadMetrics {
    reports_written: AtomicU64,
    write_errors: AtomicU64,
    receive_to_write_us: Mutex<VecDeque<u32>>,
}

impl PadMetrics {
    pub fn record_write(&self, receive_to_write: Option<Duration>) {
        self.reports_written.fetch_add(1, Ordering::Relaxed);
        let Some(receive_to_write) = receive_to_write else {
            return;
        };
        let us = u32::try_from(receive_to_write.as_micros()).unwrap_or(u32::MAX);
        let mut window = self.receive_to_write_us.lock().expect("metrics lock");
        if window.len() == WINDOW {
            window.pop_front();
        }
        window.push_back(us);
    }

    pub fn record_error(&self) {
        self.write_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn reports_written(&self) -> u64 {
        self.reports_written.load(Ordering::Relaxed)
    }

    pub fn write_errors(&self) -> u64 {
        self.write_errors.load(Ordering::Relaxed)
    }

    /// (p50, p95) in microseconds, None before the first write.
    pub fn receive_to_write_us(&self) -> Option<(u32, u32)> {
        let window = self.receive_to_write_us.lock().expect("metrics lock");
        let mut sorted: Vec<u32> = window.iter().copied().collect();
        drop(window);
        Some((percentile(&mut sorted, 50)?, percentile(&mut sorted, 95)?))
    }
}

/// Nearest-rank percentile; sorts in place.
pub fn percentile(values: &mut [u32], pct: usize) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let rank = (pct * values.len()).div_ceil(100).max(1);
    Some(values[rank - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_made_reports_count_but_have_no_latency() {
        let m = PadMetrics::default();
        m.record_write(None);
        assert_eq!(m.reports_written(), 1);
        assert_eq!(m.receive_to_write_us(), None);
    }

    #[test]
    fn nearest_rank_percentiles() {
        let mut v: Vec<u32> = (1..=100).collect();
        assert_eq!(percentile(&mut v, 50), Some(50));
        assert_eq!(percentile(&mut v, 95), Some(95));
        assert_eq!(percentile(&mut [7], 95), Some(7));
        assert_eq!(percentile(&mut [], 50), None);
    }

    #[test]
    fn window_keeps_the_latest_samples() {
        let m = PadMetrics::default();
        for us in 0..(WINDOW as u64 + 10) {
            m.record_write(Some(Duration::from_micros(us)));
        }
        assert_eq!(m.reports_written(), WINDOW as u64 + 10);
        let (p50, _) = m.receive_to_write_us().unwrap();
        assert!(p50 >= 10 + WINDOW as u32 / 2 - 1, "oldest samples dropped");
    }
}
