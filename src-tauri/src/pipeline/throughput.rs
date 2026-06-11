use std::collections::VecDeque;

/// Sliding-window throughput tracker.
///
/// Replaces the warmup-anchored per-batch EMA: instead of an exponential
/// moving average seeded on the slow first batch, this keeps a fixed-duration
/// window of recent `(completion_time_secs, image_count)` observations and
/// reports their aggregate rate. Stale batches automatically fall out of the
/// window, so the reported value converges to steady-state within a few batches
/// regardless of how slow the initial warmup was.
pub struct ThroughputWindow {
    window_secs: f32,
    entries: VecDeque<(f32, usize)>,
}

impl ThroughputWindow {
    pub fn new(window_secs: f32) -> Self {
        Self { window_secs, entries: VecDeque::new() }
    }

    /// Record that `count` images completed at `now_secs` seconds since some fixed epoch.
    pub fn record(&mut self, count: usize, now_secs: f32) {
        self.entries.push_back((now_secs, count));
        let cutoff = now_secs - self.window_secs;
        while self.entries.front().map_or(false, |(t, _)| *t < cutoff) {
            self.entries.pop_front();
        }
    }

    /// Current rate in images/sec. Returns 0.0 when fewer than two entries are
    /// present (not enough span to compute a meaningful rate).
    pub fn rate(&self, now_secs: f32) -> f32 {
        let cutoff = now_secs - self.window_secs;
        let in_window: Vec<_> = self.entries.iter().filter(|(t, _)| *t >= cutoff).collect();
        if in_window.len() < 2 {
            return 0.0;
        }
        let total: usize = in_window.iter().map(|(_, n)| n).sum();
        let oldest = in_window.first().unwrap().0;
        let span = (now_secs - oldest).max(1e-3);
        total as f32 / span
    }
}

/// Returns `raw` when it is a usable (> 0) rate, else falls back to the last
/// published value (`prev`). A transient "not enough samples" 0 from the sliding
/// window must never blank the displayed speed while processing is active.
pub fn effective_rate(raw: f32, prev: f32) -> f32 {
    if raw > 0.0 { raw } else { prev }
}

#[cfg(test)]
mod tests {
    use super::ThroughputWindow;

    #[test]
    fn slow_first_batch_does_not_suppress_subsequent_fast_rate() {
        let mut w = ThroughputWindow::new(15.0);
        // Batch 1: slow warmup — 12 images completing at t=4s (ONNX cold start)
        w.record(12, 4.0);
        // Batches 2–6: fast steady-state — 12 images per second
        w.record(12, 5.0);
        w.record(12, 6.0);
        w.record(12, 7.0);
        w.record(12, 8.0);
        w.record(12, 9.0);
        // Window holds all 6 entries; span = 9 - 4 = 5s → 72/5 = 14.4 img/s.
        // Old EMA (0.3/0.7) seeded at 3 img/s would only reach ~10.5 here.
        let rate = w.rate(9.0);
        assert!(rate >= 12.0, "expected rate >= 12 img/s after fast batches, got {rate:.1}");
    }

    #[test]
    fn empty_window_returns_zero() {
        let w = ThroughputWindow::new(15.0);
        assert_eq!(w.rate(0.0), 0.0);
    }

    #[test]
    fn single_entry_returns_zero() {
        let mut w = ThroughputWindow::new(15.0);
        w.record(12, 1.0);
        // Only one data point: no span to compute a meaningful rate.
        assert_eq!(w.rate(1.0), 0.0);
    }

    #[test]
    fn stale_entries_outside_window_are_excluded() {
        let mut w = ThroughputWindow::new(5.0);
        // Very old entry — will be outside the 5s window when queried at t=11
        w.record(1000, 0.0);
        // Two recent entries within the window
        w.record(12, 10.0);
        w.record(12, 11.0);
        // In window at t=11: [10, 11], total=24, span=1s → 24 img/s
        let rate = w.rate(11.0);
        assert!(rate >= 20.0, "stale entries must be excluded, got {rate:.1}");
    }

    #[test]
    fn effective_rate_uses_raw_when_positive() {
        assert_eq!(super::effective_rate(12.5, 3.0), 12.5);
    }

    #[test]
    fn effective_rate_holds_prev_when_raw_is_zero() {
        assert_eq!(super::effective_rate(0.0, 9.0), 9.0);
    }

    #[test]
    fn effective_rate_returns_zero_when_both_zero() {
        assert_eq!(super::effective_rate(0.0, 0.0), 0.0);
    }
}
