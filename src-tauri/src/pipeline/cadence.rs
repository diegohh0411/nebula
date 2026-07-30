//! Batch/time cadence policy for deferring per-batch side work.
//!
//! The pipeline loop used to save the vector-index snapshot and re-run the
//! clustering relabel after every batch. `Cadence` centralises the "run this
//! at most every N batches or T seconds" decision so those costs amortise
//! across batches while staying bounded, and so the policy is unit-testable
//! (same pattern as `ThroughputWindow`).

pub struct Cadence {
    max_batches: usize,
    min_interval_secs: f32,
    batches_since_fire: usize,
    last_fire_secs: f32,
}

impl Cadence {
    pub fn new(max_batches: usize, min_interval_secs: f32, now_secs: f32) -> Self {
        Self {
            max_batches,
            min_interval_secs,
            batches_since_fire: 0,
            last_fire_secs: now_secs,
        }
    }

    /// Record one completed batch. Returns true when the deferred work should
    /// run now — the batch budget is exhausted or the interval has elapsed —
    /// and resets the counters for the next cycle.
    pub fn record(&mut self, now_secs: f32) -> bool {
        self.batches_since_fire += 1;
        let budget_exhausted = self.batches_since_fire >= self.max_batches;
        let interval_elapsed = now_secs - self.last_fire_secs >= self.min_interval_secs;
        if budget_exhausted || interval_elapsed {
            self.mark_fired(now_secs);
            return true;
        }
        false
    }

    /// True while batches have accumulated since the last fire. The idle
    /// branch uses this to flush deferred work before the queue goes quiet.
    pub fn pending(&self) -> bool {
        self.batches_since_fire > 0
    }

    /// Reset after running the deferred work outside `record` (idle flush).
    pub fn mark_fired(&mut self, now_secs: f32) {
        self.batches_since_fire = 0;
        self.last_fire_secs = now_secs;
    }
}

#[cfg(test)]
mod tests {
    use super::Cadence;

    #[test]
    fn fires_when_batch_budget_is_reached() {
        let mut c = Cadence::new(3, 1000.0, 0.0);
        assert!(!c.record(1.0));
        assert!(!c.record(2.0));
        assert!(c.record(3.0), "third batch must exhaust the budget");
    }

    #[test]
    fn fires_when_interval_elapses_before_budget() {
        let mut c = Cadence::new(usize::MAX, 30.0, 0.0);
        assert!(!c.record(1.0));
        assert!(c.record(31.0), "interval elapsed — must fire on this batch");
    }

    #[test]
    fn resets_counters_after_firing() {
        let mut c = Cadence::new(2, 1000.0, 0.0);
        assert!(!c.record(1.0));
        assert!(c.record(2.0));
        // Fresh cycle: budget starts over.
        assert!(!c.record(3.0));
        assert!(c.record(4.0));
    }

    #[test]
    fn interval_measures_from_last_fire_not_last_record() {
        let mut c = Cadence::new(usize::MAX, 30.0, 0.0);
        assert!(!c.record(10.0));
        assert!(!c.record(20.0));
        assert!(c.record(30.5), "30s since construction/last fire");
        assert!(!c.record(40.0), "only 9.5s since the fire at 30.5");
    }

    #[test]
    fn pending_tracks_unflushed_batches() {
        let mut c = Cadence::new(2, 1000.0, 0.0);
        assert!(!c.pending());
        c.record(1.0);
        assert!(c.pending(), "one unflushed batch");
        c.record(2.0); // fires, resets
        assert!(!c.pending(), "fire must clear pending");
    }

    #[test]
    fn mark_fired_clears_pending_and_restarts_interval() {
        let mut c = Cadence::new(usize::MAX, 30.0, 0.0);
        c.record(29.0);
        assert!(c.pending());
        c.mark_fired(29.5); // idle flush ran the work
        assert!(!c.pending());
        assert!(
            !c.record(31.0),
            "interval restarts at mark_fired, so 1.5s later must not fire"
        );
    }
}
