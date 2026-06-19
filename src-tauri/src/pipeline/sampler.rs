use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use log::{debug, info, warn};
use sqlx::SqlitePool;
use tauri::Manager;
use tokio::sync::watch;

use crate::pipeline::queue::get_processing_counts;
use crate::pipeline::throughput::{done_delta, ThroughputWindow};

/// Rolling window length for the external rate estimate.
const WINDOW_SECS: f32 = 10.0;
/// How often we sample the DB while work is pending.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// Sleep while the queue is empty — avoids hammering SQLite when idle.
const IDLE_SLEEP: Duration = Duration::from_secs(2);

/// One sampling tick. Returns the rate to publish (img/s) and the `done`
/// count to carry forward as `prev_done` on the next tick.
///
/// When the queue is empty the run is finished: clear the window and report 0
/// so a completed import never leaks a stale rate into the next one (TT-64).
pub fn sample_once(
    window: &mut ThroughputWindow,
    prev_done: i64,
    total_pending: i64,
    done: i64,
    now_secs: f32,
) -> (f32, i64) {
    if total_pending == 0 {
        window.clear();
        return (0.0, done);
    }
    let delta = done_delta(prev_done, done);
    window.record(delta, now_secs);
    (window.rate(now_secs), done)
}

/// External throughput sampler. Measures inference speed from observable DB
/// progress (images moving into the `done` state) instead of from pipeline
/// internals, so the displayed speed/ETA never gets stuck on warmup or batch
/// cadence. Owns `AppState.throughput_ema`; the pipeline loop no longer writes it.
///
/// `shutdown_rx`: a `watch` receiver that turns `true` when the app is exiting.
/// The sampler observes this signal during every sleep so it exits promptly
/// instead of waiting out the full interval.
pub async fn run_throughput_sampler(
    pool: SqlitePool,
    app: tauri::AppHandle,
    shutdown_rx: watch::Receiver<bool>,
) {
    let mut window = ThroughputWindow::new(WINDOW_SECS);
    let start = Instant::now();

    // Seed prev_done with the current count so the first delta isn't the full
    // baseline of already-done images.
    let mut prev_done = match get_processing_counts(&pool).await {
        Ok(c) => c.done,
        Err(e) => {
            warn!("[sampler] initial count failed: {e}");
            0
        }
    };

    /// Sleeps for `duration` but returns early (as `true`) if the shutdown
    /// signal fires, or returns `false` after the full sleep.
    macro_rules! interruptible_sleep {
        ($dur:expr, $rx:expr) => {{
            let mut rx = $rx.clone();
            tokio::select! {
                _ = tokio::time::sleep($dur) => false,
                _ = rx.wait_for(|v| *v) => true,
            }
        }};
    }

    loop {
        let counts = match get_processing_counts(&pool).await {
            Ok(c) => c,
            Err(e) => {
                warn!("[sampler] count query failed: {e}");
                if interruptible_sleep!(IDLE_SLEEP, shutdown_rx) {
                    break;
                }
                continue;
            }
        };

        let now_secs = start.elapsed().as_secs_f32();
        let (rate, new_prev) =
            sample_once(&mut window, prev_done, counts.total_pending, counts.done, now_secs);
        prev_done = new_prev;

        let state = app.state::<crate::AppState>();
        state
            .throughput_ema
            .store(rate.to_bits(), Ordering::Relaxed);

        let sleep_dur = if counts.total_pending == 0 {
            IDLE_SLEEP
        } else {
            debug!("[sampler] {:.1} img/s ({} pending)", rate, counts.total_pending);
            SAMPLE_INTERVAL
        };

        if interruptible_sleep!(sleep_dur, shutdown_rx) {
            break;
        }
    }

    info!("[sampler] shutting down");
}

#[cfg(test)]
mod tests {
    use super::sample_once;
    use crate::pipeline::throughput::ThroughputWindow;

    #[test]
    fn idle_queue_clears_window_and_reports_zero() {
        let mut w = ThroughputWindow::new(10.0);
        w.record(12, 1.0);
        w.record(12, 2.0);
        // total_pending == 0 => finished run: rate 0, window cleared.
        let (rate, prev) = sample_once(&mut w, 50, 0, 62, 3.0);
        assert_eq!(rate, 0.0);
        assert_eq!(prev, 62, "prev_done carries forward the latest done count");
        assert_eq!(w.rate(3.0), 0.0, "window must be cleared on idle");
    }

    #[test]
    fn active_queue_records_delta_and_reports_rate() {
        let mut w = ThroughputWindow::new(10.0);
        // Two ticks one second apart, 12 completions each => 24 / 1s = 24 img/s.
        let (_r1, p1) = sample_once(&mut w, 100, 5, 112, 10.0);
        assert_eq!(p1, 112);
        let (r2, p2) = sample_once(&mut w, p1, 5, 124, 11.0);
        assert_eq!(p2, 124);
        assert!(r2 >= 20.0, "expected ~24 img/s, got {r2:.1}");
    }

    #[test]
    fn deletion_during_processing_is_clamped() {
        let mut w = ThroughputWindow::new(10.0);
        // First a normal tick to seed an entry.
        let (_r, p) = sample_once(&mut w, 100, 5, 112, 10.0);
        // Then done drops (deletion); delta clamps to 0, no panic, rate stays finite.
        let (r, p2) = sample_once(&mut w, p, 5, 108, 11.0);
        assert_eq!(p2, 108);
        assert!(r >= 0.0 && r.is_finite());
    }

    #[test]
    fn first_sample_after_new_import_yields_zero_then_positive_rate() {
        // Scenario: sampler was idle at prev_done=500 (baseline from a prior run).
        // A new import arrives; the first tick sees total_pending > 0 but done is
        // still 500 — nothing has finished in this batch yet.
        // Expected: delta == 0, window gets a zero entry, rate == 0 ("Calculating ETA").
        // The next tick with real progress must produce a positive rate.
        let mut w = ThroughputWindow::new(10.0);
        let prev_done = 500_i64;

        // Tick 1: import active, no completions yet (done == prev_done).
        let (rate1, p1) = sample_once(&mut w, prev_done, 10, 500, 1.0);
        assert_eq!(rate1, 0.0, "first tick with no progress must report rate 0");
        assert_eq!(p1, 500, "prev_done carried forward unchanged");

        // Tick 2: 15 images complete (done = 515).
        let (rate2, p2) = sample_once(&mut w, p1, 10, 515, 2.0);
        assert_eq!(p2, 515);
        assert!(
            rate2 > 0.0,
            "second tick with progress must produce a positive rate, got {rate2:.1}"
        );
    }

    #[test]
    fn transition_active_to_idle_clears_stale_rate() {
        // After an active run produces a positive rate, switching to idle
        // (total_pending == 0) must clear the window and report 0 so no stale
        // rate leaks into the next import (TT-64).
        let mut w = ThroughputWindow::new(10.0);

        // Two active ticks to build up a non-zero rate.
        let (_r1, p1) = sample_once(&mut w, 0, 5, 12, 10.0);
        let (r_active, p2) = sample_once(&mut w, p1, 5, 24, 11.0);
        assert!(r_active > 0.0, "expected positive rate during active phase, got {r_active:.1}");

        // Queue drains: total_pending == 0.
        let (r_idle, _p3) = sample_once(&mut w, p2, 0, 24, 12.0);
        assert_eq!(r_idle, 0.0, "idle transition must report rate 0");
        // Window must be cleared so a subsequent rate query also returns 0.
        assert_eq!(w.rate(12.0), 0.0, "window must be cleared after idle transition");
    }
}
