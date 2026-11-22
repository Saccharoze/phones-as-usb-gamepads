//! A slot whose phone goes quiet for the fail-safe window gets a neutral
//! report, so a dropped phone never leaves a button held.

use std::time::Duration;

use tokio::time::Instant;

/// One slot as the fail-safe sees it: when its phone was last heard, and
/// whether the pad currently shows anything other than neutral.
#[derive(Debug, Clone, Copy)]
pub struct Watch {
    pub last_seen: Instant,
    pub active_input: bool,
}

/// Indices of slots whose deadline has passed while input is still held.
pub fn due(slots: &[Option<Watch>], now: Instant, window: Duration) -> Vec<usize> {
    slots
        .iter()
        .enumerate()
        .filter_map(|(i, w)| w.filter(|w| w.active_input && now >= w.last_seen + window).map(|_| i))
        .collect()
}

/// The earliest moment one of the slots could become due, if any.
pub fn next_deadline(slots: &[Option<Watch>], window: Duration) -> Option<Instant> {
    slots
        .iter()
        .flatten()
        .filter(|w| w.active_input)
        .map(|w| w.last_seen + window)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_millis(300);

    #[tokio::test(start_paused = true)]
    async fn fires_exactly_at_the_window() {
        let t0 = Instant::now();
        let slots = [Some(Watch { last_seen: t0, active_input: true }), None];
        assert_eq!(next_deadline(&slots, WINDOW), Some(t0 + WINDOW));
        tokio::time::sleep(Duration::from_millis(299)).await;
        assert!(due(&slots, Instant::now(), WINDOW).is_empty());
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(due(&slots, Instant::now(), WINDOW), vec![0]);
    }

    #[test]
    fn neutral_pads_need_no_deadline() {
        let t0 = Instant::now();
        let slots = [Some(Watch { last_seen: t0, active_input: false })];
        assert_eq!(next_deadline(&slots, WINDOW), None);
        assert!(due(&slots, t0 + WINDOW * 10, WINDOW).is_empty());
    }

    #[test]
    fn picks_the_earliest_deadline() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_millis(100);
        let slots = [
            Some(Watch { last_seen: later, active_input: true }),
            Some(Watch { last_seen: t0, active_input: true }),
        ];
        assert_eq!(next_deadline(&slots, WINDOW), Some(t0 + WINDOW));
        assert_eq!(due(&slots, t0 + WINDOW, WINDOW), vec![1]);
    }
}
