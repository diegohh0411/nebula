# External Throughput Sampler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace in-pipeline throughput measurement with an external background task that samples the DB `done` count over a rolling window, so the "Processing N images" badge stops getting stuck on "Calculating ETA…".

**Architecture:** A dedicated tokio task (`run_throughput_sampler`) wakes ~every second, reads `get_processing_counts`, feeds clamped `done` deltas into a reused 10s `ThroughputWindow`, and stores the rate in the existing `AppState.throughput_ema` atomic. The pipeline loop stops touching throughput entirely; both the `pipeline_stats` event and `get_processing_status` poll keep reading the atomic unchanged. The frontend treats `images_per_sec === 0` as "calculating" and any `> 0` as a real rate.

**Tech Stack:** Rust / Tauri (`tauri::async_runtime::spawn`, `sqlx`, atomics), Angular / TypeScript frontend.

## Global Constraints

- Background tasks are spawned with `tauri::async_runtime::spawn`, **not** `tokio::spawn` (project convention, commit 86b77e0).
- Rolling window length: **10.0 seconds**. Sample interval: **1 second**. Idle sleep: **2 seconds**.
- `AppState.throughput_ema` is an `AtomicU32` holding `f32::to_bits`; read/write with `Ordering::Relaxed`, matching existing code.
- Spec: `docs/superpowers/specs/2026-06-17-external-throughput-sampler-design.md`.

---

## File Structure

- `src-tauri/src/pipeline/throughput.rs` — add `done_delta` pure fn + `ThroughputWindow::clear`; **remove** `effective_rate` and its 3 tests.
- `src-tauri/src/pipeline/sampler.rs` — **new**: `sample_once` pure fn + `run_throughput_sampler` loop.
- `src-tauri/src/pipeline/mod.rs` — register `pub mod sampler;`; **remove** in-pipeline throughput measurement (window, `pipeline_start`, idle reset, post-batch block).
- `src-tauri/src/app/mod.rs` — spawn the sampler task.
- `src/app/components/search-bar/search-bar.component.html` — drop the `>= 0.1` threshold.
- `src/app/services/photo.service.ts` — remove the hold-last-known speed hack.

---

## Task 1: `done_delta` and `ThroughputWindow::clear`

**Files:**
- Modify: `src-tauri/src/pipeline/throughput.rs`
- Test: `src-tauri/src/pipeline/throughput.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn done_delta(prev_done: i64, now_done: i64) -> usize`
  - `pub fn ThroughputWindow::clear(&mut self)`

- [ ] **Step 1: Write the failing tests**

Add these tests inside the existing `mod tests` block in `src-tauri/src/pipeline/throughput.rs`:

```rust
#[test]
fn done_delta_returns_completions_since_last_sample() {
    assert_eq!(super::done_delta(100, 112), 12);
}

#[test]
fn done_delta_zero_when_no_progress() {
    assert_eq!(super::done_delta(100, 100), 0);
}

#[test]
fn done_delta_clamps_negative_from_deletions() {
    // A deletion lowers the done count; must never yield a negative sample.
    assert_eq!(super::done_delta(100, 95), 0);
}

#[test]
fn clear_empties_the_window_so_rate_returns_zero() {
    let mut w = ThroughputWindow::new(10.0);
    w.record(12, 1.0);
    w.record(12, 2.0);
    assert!(w.rate(2.0) > 0.0);
    w.clear();
    assert_eq!(w.rate(2.0), 0.0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p nebula --lib pipeline::throughput`
Expected: FAIL — `no function or associated item named 'done_delta'` / `clear`.

- [ ] **Step 3: Write minimal implementation**

Add `clear` inside `impl ThroughputWindow` (after `rate`):

```rust
    /// Drop all observations. Used by the sampler when the queue drains so a
    /// finished run never leaks a stale rate into the next import (TT-64).
    pub fn clear(&mut self) {
        self.entries.clear();
    }
```

Add `done_delta` as a free function after `effective_rate` (or anywhere in the file's top-level scope):

```rust
/// Completions since the previous sample. Clamps to 0 so deletions
/// (which lower the done count) never produce a negative throughput sample.
pub fn done_delta(prev_done: i64, now_done: i64) -> usize {
    (now_done - prev_done).max(0) as usize
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nebula --lib pipeline::throughput`
Expected: PASS (all throughput tests, old and new).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pipeline/throughput.rs
git commit -m "feat(pipeline): add done_delta and ThroughputWindow::clear for external sampler"
```

---

## Task 2: Throughput sampler module

**Files:**
- Create: `src-tauri/src/pipeline/sampler.rs`
- Modify: `src-tauri/src/pipeline/mod.rs:5` (add `pub mod sampler;`)
- Test: `src-tauri/src/pipeline/sampler.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes:
  - `crate::pipeline::throughput::{ThroughputWindow, done_delta}` (Task 1)
  - `crate::pipeline::queue::get_processing_counts(pool) -> Result<ProcessingStatus>` where `ProcessingStatus { total_pending: i64, done: i64 }`
  - `crate::AppState { throughput_ema: AtomicU32 }`
- Produces:
  - `pub fn sample_once(window: &mut ThroughputWindow, prev_done: i64, total_pending: i64, done: i64, now_secs: f32) -> (f32, i64)`
  - `pub async fn run_throughput_sampler(pool: sqlx::SqlitePool, app: tauri::AppHandle)`

- [ ] **Step 1: Register the module**

In `src-tauri/src/pipeline/mod.rs`, change line 5 from:

```rust
pub mod throughput;
```

to:

```rust
pub mod sampler;
pub mod throughput;
```

- [ ] **Step 2: Write the failing test for `sample_once`**

Create `src-tauri/src/pipeline/sampler.rs` with only the test module first:

```rust
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
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p nebula --lib pipeline::sampler`
Expected: FAIL — `cannot find function 'sample_once'`.

- [ ] **Step 4: Implement `sample_once` and `run_throughput_sampler`**

Prepend this above the `#[cfg(test)]` block in `src-tauri/src/pipeline/sampler.rs`:

```rust
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use log::{debug, warn};
use sqlx::SqlitePool;
use tauri::Manager;

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
pub async fn run_throughput_sampler(pool: SqlitePool, app: tauri::AppHandle) {
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

    loop {
        let counts = match get_processing_counts(&pool).await {
            Ok(c) => c,
            Err(e) => {
                warn!("[sampler] count query failed: {e}");
                tokio::time::sleep(IDLE_SLEEP).await;
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

        if counts.total_pending == 0 {
            tokio::time::sleep(IDLE_SLEEP).await;
        } else {
            debug!("[sampler] {:.1} img/s ({} pending)", rate, counts.total_pending);
            tokio::time::sleep(SAMPLE_INTERVAL).await;
        }
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nebula --lib pipeline::sampler`
Expected: PASS (3 tests).

- [ ] **Step 6: Verify it compiles with no warnings**

Run: `cargo clippy -p nebula --lib -- -D warnings`
Expected: PASS. (`run_throughput_sampler` is unused until Task 3 — if clippy flags it as dead code, that is resolved in Task 3 Step 4 where it gets spawned. If CI treats the warning as an error at this commit, proceed directly to Task 3 before pushing; the two commits land together.)

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/pipeline/sampler.rs src-tauri/src/pipeline/mod.rs
git commit -m "feat(pipeline): add external throughput sampler (sample_once + run loop)"
```

---

## Task 3: Swap the pipeline to the external sampler

Removes the in-pipeline measurement and spawns the sampler — the single-writer swap, so the atomic always has exactly one owner.

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs` (remove lines ~159–160, ~196–199 reset, ~545–561 post-batch block)
- Modify: `src-tauri/src/pipeline/throughput.rs` (remove `effective_rate` + its 3 tests)
- Modify: `src-tauri/src/app/mod.rs` (spawn the sampler near line 69)

**Interfaces:**
- Consumes: `crate::pipeline::sampler::run_throughput_sampler(pool, app)` (Task 2).
- Produces: nothing new.

- [ ] **Step 1: Remove the in-pipeline window setup**

In `src-tauri/src/pipeline/mod.rs`, delete these two lines (currently 159–160):

```rust
    let mut throughput_window = throughput::ThroughputWindow::new(15.0);
    let pipeline_start = Instant::now();
```

- [ ] **Step 2: Remove the idle-branch reset**

In the same file, in the `if sem_batch.is_empty() && sub_batch.is_empty()` block, delete the throughput reset so only the sleep/continue remain. Remove this (currently ~195–199):

```rust
            // Nothing pending: clear the held speed so a finished run does not
            // leak a stale rate into the next import (TT-64).
            let app_state: tauri::State<crate::AppState> = app.state();
            app_state
                .throughput_ema
                .store(0.0f32.to_bits(), std::sync::atomic::Ordering::Relaxed);
```

The block now reads:

```rust
        if sem_batch.is_empty() && sub_batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
```

- [ ] **Step 3: Remove the post-batch measurement block**

In the same file, delete the throughput measurement after the per-image loop (currently ~545–561), from `let now_secs = pipeline_start.elapsed()...` through the `.store(effective.to_bits(), ...)` call:

```rust
        let now_secs = pipeline_start.elapsed().as_secs_f32();
        throughput_window.record(images_processed_this_iter, now_secs);
        let raw_rate = throughput_window.rate(now_secs);

        // Single source of truth for speed (TT-7/TT-64): hold the last-known rate
        // when the window momentarily lacks samples, so the speed never blanks
        // mid-processing.
        let app_state: tauri::State<crate::AppState> = app.state();
        let prev = f32::from_bits(
            app_state
                .throughput_ema
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let effective = crate::pipeline::throughput::effective_rate(raw_rate, prev);
        app_state
            .throughput_ema
            .store(effective.to_bits(), std::sync::atomic::Ordering::Relaxed);
```

Leave the following `crate::search::math::emit_progress(&pool, &app).await;` line — the event still publishes `total_pending` and the (now sampler-owned) rate.

> Note: `images_processed_this_iter` (defined ~291) is still used by the log line at ~294; do not remove it. `Instant` may become unused after removing `pipeline_start` — if `cargo clippy` flags the `use std::time::{Duration, Instant};` import at line 41, change it to `use std::time::Duration;`.

- [ ] **Step 4: Spawn the sampler in app setup**

In `src-tauri/src/app/mod.rs`, immediately after the hash-worker spawn (line 69):

```rust
            tauri::async_runtime::spawn(crate::library::hasher::run_hash_worker(pool.clone()));
```

add:

```rust
            // External throughput sampler: measures inference speed from DB
            // progress instead of pipeline internals (fixes stuck "Calculating ETA").
            tauri::async_runtime::spawn(crate::pipeline::sampler::run_throughput_sampler(
                pool.clone(),
                app.handle().clone(),
            ));
```

- [ ] **Step 5: Remove `effective_rate` and its tests**

In `src-tauri/src/pipeline/throughput.rs`, delete the `effective_rate` function (the doc comment + fn, currently ~48–57) and its three tests inside `mod tests` (`effective_rate_uses_raw_when_positive`, `effective_rate_holds_prev_when_raw_is_zero`, `effective_rate_returns_zero_when_both_zero`, currently ~113–127).

- [ ] **Step 6: Verify the backend builds clean and all tests pass**

Run: `cargo clippy -p nebula --lib -- -D warnings && cargo test -p nebula --lib pipeline`
Expected: PASS — no `effective_rate`/dead-code/unused-import warnings; `pipeline::throughput` and `pipeline::sampler` tests green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs src-tauri/src/pipeline/throughput.rs src-tauri/src/app/mod.rs
git commit -m "refactor(pipeline): measure throughput via external sampler, drop in-loop EMA"
```

---

## Task 4: Frontend — clean sentinel for the badge

**Files:**
- Modify: `src/app/components/search-bar/search-bar.component.html:69-76`
- Modify: `src/app/services/photo.service.ts:146-155`

**Interfaces:**
- Consumes: `pipelineStats().images_per_sec` where `0` means "no samples yet" and `> 0` is a real rate.
- Produces: nothing.

- [ ] **Step 1: Drop the `>= 0.1` threshold in the badge**

In `src/app/components/search-bar/search-bar.component.html`, replace the block (lines 69–76):

```html
        @if (photos.pipelineStats().images_per_sec >= 0.1) {
          · {{ photos.pipelineStats().images_per_sec | number:'1.0-1' }} img/s
          @if (formatEta(photos.etaSeconds())) {
            · {{ formatEta(photos.etaSeconds()) }}
          }
        } @else {
          · Calculating ETA…
        }
```

with (use `> 0`, so any measured rate shows a real ETA — only the no-samples-yet state says "Calculating"):

```html
        @if (photos.pipelineStats().images_per_sec > 0) {
          · {{ photos.pipelineStats().images_per_sec | number:'1.0-1' }} img/s
          @if (formatEta(photos.etaSeconds())) {
            · {{ formatEta(photos.etaSeconds()) }}
          }
        } @else {
          · Calculating ETA…
        }
```

- [ ] **Step 2: Remove the hold-last-known speed hack**

In `src/app/services/photo.service.ts`, the `pipelineStats$` subscription in the constructor (lines ~146–155) currently reads:

```typescript
    this.events.pipelineStats$.subscribe((e) => {
      // Hold-last-known speed: a 0 while work remains is a heartbeat without a
      // fresh sample, not a real stop — keep the prior speed (TT-64).
      const prev = this.pipelineStats();
      const images_per_sec =
        e.images_per_sec > 0 || e.total_pending === 0
          ? e.images_per_sec
          : prev.images_per_sec;
      this.pipelineStats.set({ ...e, images_per_sec });
    });
```

Replace it with a straight passthrough — the sampler refreshes the rate every second, so a stale-0 heartbeat is no longer a concern:

```typescript
    this.events.pipelineStats$.subscribe((e) => {
      // The backend sampler refreshes the rate every second, so 0 means
      // "no samples yet", not a missed heartbeat — pass it straight through.
      this.pipelineStats.set(e);
    });
```

- [ ] **Step 3: Verify the frontend builds and tests pass**

Run: `pnpm test -- --watch=false` and `pnpm build`
Expected: PASS — no type errors; existing search-bar/photo.service specs green.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/search-bar/search-bar.component.html src/app/services/photo.service.ts
git commit -m "fix(ui): show real ETA for any measured rate; drop hold-last-speed hack"
```

---

## Manual Verification (after Task 4)

- [ ] Build and run the app (`pnpm tauri dev`), import a folder of images.
- [ ] Confirm the badge shows "Processing N images" and within ~2s displays a stable `img/s` + ETA — it must **not** stay on "Calculating ETA…".
- [ ] Let processing finish; confirm the badge transitions to "Library up to date" and the rate does not leak into a subsequent import (start a second import and confirm it recomputes from ~0, not the previous run's speed).

---

## Self-Review

**Spec coverage:**
- External backend sampler task → Task 2 + Task 3 Step 4. ✓
- Reuse `ThroughputWindow`, 10s window → Task 2 (`WINDOW_SECS = 10.0`). ✓
- `done_delta` clamp → Task 1. ✓
- Idle reset / TT-64 (clear window, store 0) → Task 1 (`clear`) + Task 2 (`sample_once` idle branch) + test. ✓
- Remove in-pipeline measurement + `effective_rate` → Task 3. ✓
- Drop `>= 0.1` threshold, `0` sentinel → Task 4 Step 1. ✓
- Remove frontend hold-last hack → Task 4 Step 2. ✓
- Downstream unchanged (`emit_progress`, `get_processing_status`) → not modified; verified by reuse of atomic. ✓
- Out of scope (no schema/IPC/event changes) → respected. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code. ✓

**Type consistency:** `sample_once(&mut ThroughputWindow, i64, i64, i64, f32) -> (f32, i64)` used identically in Task 2 impl, tests, and Task 3's `run_throughput_sampler`. `done_delta(i64, i64) -> usize` and `clear(&mut self)` match Task 1 definitions. `ProcessingStatus { total_pending: i64, done: i64 }` matches `entities.rs:38`. ✓
