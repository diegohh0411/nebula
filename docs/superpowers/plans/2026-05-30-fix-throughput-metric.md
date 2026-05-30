# Fix Throughput Metric (TT-10) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the `images_per_sec` metric that permanently reads `0 img/s` in the badge by replacing the broken rolling-window approach with a per-batch EMA calculation.

**Architecture:** The pipeline loop in `src-tauri/src/pipeline/mod.rs` currently uses a `VecDeque<(Instant, usize)>` rolling window that is evicted before a second entry can accumulate (batches take >5 s), so the `len() < 2` guard always returns `0.0`. The fix captures a `batch_start: Instant` before Stage 1 decode, measures real elapsed time after Stage 2 completes, computes an instantaneous rate, then smooths it with an EMA stored in a `mut ema: f32` local variable. The `VecDeque` and its associated `use` import are removed entirely.

**Tech Stack:** Rust (Tokio async, `std::time::Instant`), existing `emit_progress` API — no new dependencies.

---

## File Map

| Action | File |
|--------|------|
| Modify | `src-tauri/src/pipeline/mod.rs` |

No frontend changes needed — the `>= 0.5` gate in `search-bar.component.html:59` is correct and will work once the backend emits a real rate.

---

### Task 1: Remove the rolling-window and replace with EMA

This is a single focused change to `mod.rs`. We do it in two atomic sub-steps: write tests first (in Rust's inline test module), then make them pass.

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`

---

- [ ] **Step 1: Understand the exact current state of the three change sites**

Read the following line ranges so you know exactly what to replace:

- Lines 34–36 (imports — `VecDeque` is here)
- Lines 120 (where `throughput_window` is declared)
- Lines 350–365 (where the window is written and `images_per_sec` is computed)

```bash
sed -n '34,36p' src-tauri/src/pipeline/mod.rs
sed -n '118,122p' src-tauri/src/pipeline/mod.rs
sed -n '348,366p' src-tauri/src/pipeline/mod.rs
```

---

- [ ] **Step 2: Write a failing unit test for EMA math in `mod.rs`**

Add this test module at the very bottom of `src-tauri/src/pipeline/mod.rs` (after the closing `}` of `run_pipeline`):

```rust
#[cfg(test)]
mod tests {
    fn ema_step(prev: f32, inst: f32) -> f32 {
        if prev == 0.0 { inst } else { 0.3 * inst + 0.7 * prev }
    }

    #[test]
    fn ema_seeds_on_first_batch() {
        // First batch: ema starts at 0.0, should seed with inst_rate directly
        let ema = ema_step(0.0, 4.0);
        assert_eq!(ema, 4.0, "first batch must seed ema = inst_rate");
    }

    #[test]
    fn ema_smooths_on_subsequent_batches() {
        let ema = ema_step(4.0, 8.0);
        // 0.3 * 8.0 + 0.7 * 4.0 = 2.4 + 2.8 = 5.2
        let expected = 0.3_f32 * 8.0 + 0.7_f32 * 4.0;
        assert!(
            (ema - expected).abs() < 1e-5,
            "ema={ema} expected={expected}"
        );
    }

    #[test]
    fn ema_is_positive_for_nonzero_input() {
        let mut ema = 0.0_f32;
        for _ in 0..5 {
            ema = ema_step(ema, 3.0);
        }
        assert!(ema > 0.0, "ema must be positive after several batches with nonzero rate");
    }
}
```

---

- [ ] **Step 3: Run the test to verify it fails (function doesn't exist yet)**

```bash
cd src-tauri && cargo test pipeline::tests -- --nocapture 2>&1 | tail -20
```

Expected: The test module compiles but `ema_step` is a local helper so the tests should compile. They may actually pass already since `ema_step` is self-contained — if they pass, that's fine, proceed. The key is that the *production code* still uses the old window (next steps fix that).

---

- [ ] **Step 4: Remove `VecDeque` import**

In `src-tauri/src/pipeline/mod.rs`, remove the `VecDeque` use statement. Find it at line ~34:

```rust
// REMOVE this line:
use std::collections::VecDeque;
```

The `use std::sync::Arc;` and `use std::time::{Duration, Instant};` lines stay.

---

- [ ] **Step 5: Replace `throughput_window` declaration with EMA state + `batch_start` capture**

Find this block (around line 120):

```rust
    let mut throughput_window: VecDeque<(Instant, usize)> = VecDeque::new();
```

Replace it with:

```rust
    let mut throughput_ema: f32 = 0.0;
```

Then find the line that begins the outer `loop {` body. The very first thing inside the loop body (before the `get_queue_batch` calls) should capture the batch start time. Locate the `loop {` line (around line 122) and the first `let sem_batch = match` line inside it. Insert `batch_start` capture just after `loop {`:

```rust
    loop {
        let batch_start = Instant::now();
        // Pull both queues
        let sem_batch = match crate::db::get_queue_batch(&pool, "semantic", config.batch_size as i64).await {
```

---

- [ ] **Step 6: Replace the rolling-window computation with EMA**

Find and replace the entire block from `let now = Instant::now();` through `crate::embedder::emit_progress(...)` (lines ~350–365):

**Old code (remove all of this):**
```rust
        let now = Instant::now();
        throughput_window.push_back((now, images_processed_this_iter));
        throughput_window.retain(|(t, _)| now.duration_since(*t) <= Duration::from_secs(5));
        let images_per_sec = if throughput_window.len() < 2 {
            0.0_f32
        } else {
            let sum_images: usize = throughput_window.iter().map(|(_, n)| n).sum();
            let window_span = throughput_window
                .front()
                .map(|(t, _)| now.duration_since(*t))
                .unwrap_or(Duration::from_millis(1))
                .max(Duration::from_millis(1));
            sum_images as f32 / window_span.as_secs_f32()
        };

        crate::embedder::emit_progress(&pool, &app, images_per_sec).await;
```

**New code (replace with):**
```rust
        let dt = batch_start.elapsed().as_secs_f32().max(1e-3);
        let inst_rate = images_processed_this_iter as f32 / dt;
        throughput_ema = if throughput_ema == 0.0 {
            inst_rate
        } else {
            0.3 * inst_rate + 0.7 * throughput_ema
        };

        crate::embedder::emit_progress(&pool, &app, throughput_ema).await;
```

---

- [ ] **Step 7: Verify the code compiles**

```bash
cd src-tauri && cargo check 2>&1
```

Expected: No errors. If you see "unused import" for `VecDeque`, you missed Step 4 — remove the import. If you see "unused import" for `Duration`, check whether `Duration` is still used elsewhere in the file (it is, in the `sleep` call on the idle path), so keep it.

---

- [ ] **Step 8: Run the unit tests**

```bash
cd src-tauri && cargo test pipeline::tests -- --nocapture 2>&1
```

Expected output (all three pass):
```
test pipeline::tests::ema_seeds_on_first_batch ... ok
test pipeline::tests::ema_smooths_on_subsequent_batches ... ok
test pipeline::tests::ema_is_positive_for_nonzero_input ... ok
```

---

- [ ] **Step 9: Run the full Rust test suite**

```bash
cd src-tauri && cargo test 2>&1 | tail -30
```

Expected: All tests pass, zero failures. If unrelated tests fail, note them and confirm they pre-exist on `main` before proceeding.

---

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "fix(pipeline): replace broken throughput window with per-batch EMA (TT-10)

Rolling VecDeque window was evicted before accumulating 2 entries (batches
>5 s), so images_per_sec was permanently 0.0. Replaced with batch_start
Instant + EMA smoothing (α=0.3) that produces a real rate from the first
completed batch.
"
```

---

### Task 2: Manual smoke-test verification

This task is not automated — it requires a running app with queued images.

**Files:** none (read-only verification)

- [ ] **Step 1: Build and run the app in release mode**

```bash
# From repo root
npm run tauri build -- --debug 2>&1 | tail -20
# or use the dev server:
npm run tauri dev
```

- [ ] **Step 2: Queue at least one batch of images**

Add 12+ images to the library so the pipeline has work to do. Watch the search bar badge.

- [ ] **Step 3: Verify the badge shows a non-zero img/s figure**

The badge (in `search-bar.component.html`) shows `X img/s` only when `images_per_sec >= 0.5`. After one completed batch you should see a figure like `2 img/s` or `8 img/s` depending on hardware.

Cross-check sanity: count images ÷ wall-clock seconds should be in the same ballpark.

- [ ] **Step 4: Verify the pending-count portion of the badge is unaffected**

The `N images` count portion should still update correctly and not regress.

---

### Task 3: Open PR and update Notion

- [ ] **Step 1: Push the branch**

```bash
git push -u origin worktree-tt-10-fix-throughput-metric
```

- [ ] **Step 2: Open a PR**

```bash
gh pr create \
  --title "fix(pipeline): replace broken throughput window with per-batch EMA (TT-10)" \
  --body "$(cat <<'EOF'
## Summary

- The `images_per_sec` metric was permanently `0.0` because the 5-second rolling window was evicted before a second entry could accumulate (batches take >5 s on debug builds / slow machines).
- Replaced `VecDeque<(Instant, usize)>` window + `len() < 2` guard with a direct per-batch timing (`batch_start = Instant::now()` captured before Stage 1) and EMA smoothing (α = 0.3).
- The badge now shows a non-zero `img/s` figure from the very first completed batch.
- No frontend changes required; the `>= 0.5` display gate in `search-bar.component.html` is correct and intentional.

## Test plan

- [ ] `cargo test pipeline::tests` passes (3 unit tests for EMA math)
- [ ] `cargo test` (full suite) passes with no new failures
- [ ] Running app: badge shows non-zero img/s within first completed batch
- [ ] Pending-count portion of badge unaffected

Fixes: TT-10

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Update Notion task status to "Ready for review" and record PR number**

Get the PR number from the `gh pr create` output, then:

```bash
PAGE_ID=370e954d-b476-8110-9f73-de44870762fa
PR_NUMBER=<number from gh pr create output>

ntn api v1/pages/$PAGE_ID -X PATCH \
  'properties[Status][status][name]=Ready for review' \
  "properties[PR number][number]:=$PR_NUMBER"
```

---

## Self-Review

**Spec coverage check:**
- ✅ Badge shows non-zero `img/s` within first batch → Tasks 1 + 2
- ✅ Figure is in believable range → Task 2 smoke test
- ✅ No regression to pending-count portion → Task 2 step 4
- ✅ Fix is backend only; `>= 0.5` gate stays → documented in File Map

**Placeholder scan:** No TBDs, no "handle edge cases", all code shown inline.

**Type consistency:**
- `throughput_ema: f32` declared in Task 1 Step 5, used in Task 1 Step 6 — matches.
- `batch_start: Instant` captured in Task 1 Step 5, consumed in Task 1 Step 6 — matches.
- `images_processed_this_iter: usize` already exists at line 193 — used unchanged.
