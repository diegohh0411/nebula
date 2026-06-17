# External Throughput Sampler Design

**Date:** 2026-06-17
**Status:** Approved (pending spec review)

## Problem

The "Processing N images" badge frequently gets stuck on **"Calculating ETA…"**. The
inference speed is measured *inside* the pipeline loop (`pipeline/mod.rs`): each batch
records `images_processed_this_iter` into a 15s `ThroughputWindow`, computes a rate,
applies an `effective_rate` hold-last hack, and stores it in `AppState.throughput_ema`.

This coupling is fragile:

- The window needs **≥2 batch entries** to produce a non-zero rate, so the displayed
  speed is hostage to batch cadence and ONNX cold-start. With large/slow batches the
  atomic can sit at `0.0` for a long time.
- The frontend then shows "Calculating ETA…" for any `images_per_sec < 0.1`, so a stuck
  0 (or a genuinely slow pipeline) is indistinguishable from "still warming up".

This metric has been revamped repeatedly (see
`2026-05-28-local-inference-throughput-design.md`,
`2026-05-30-pipeline-stats-throughput-design.md`,
`2026-06-11-processing-label-speed-eta-design.md`). The root cause is that the metric is
derived from pipeline internals rather than from observable progress.

## Approach

Stop measuring throughput from inside the inference pipeline. Instead, measure it
**externally** by sampling how many images have actually moved into the "done" state over
a rolling wall-clock window. Progress is observed, not inferred — robust to whatever the
pipeline does internally (batch size, warmup, channel depth).

The decision (backend tokio task vs frontend-derived) is **backend tokio task**: the
existing `AppState.throughput_ema` atomic stays the single source of truth, so both the
pushed `pipeline_stats` event and the `get_processing_status` poll command keep working
unchanged.

## Architecture

### New: throughput sampler task

A dedicated background task spawned once at app startup, alongside the pipeline loop.
Loop body, every ~1 second:

1. Read `pipeline::queue::get_processing_counts(pool)` → `{ total_pending, done }`.
2. If `total_pending == 0`: clear the window, store `0.0` into `throughput_ema`, sleep
   ~2s, continue. (Preserves the TT-64 fix: a finished run must not leak a stale rate
   into the next import, and we don't hammer SQLite while idle.)
3. Otherwise compute `delta = done_delta(prev_done, done_now)` (see pure fn below),
   record `(delta, now_secs)` into a 10s `ThroughputWindow`, store `window.rate(now_secs)`
   into `throughput_ema`, update `prev_done = done_now`, sleep ~1s.

`now_secs` is `sampler_start.elapsed().as_secs_f32()` (a monotonic wall clock for the
task), mirroring how the pipeline currently uses `pipeline_start`.

### Reused: `ThroughputWindow`

`pipeline/throughput.rs::ThroughputWindow` is reused **unchanged** — it already computes
`sum(counts) / span` over a sliding window and returns exactly `0.0` when it holds `<2`
entries. We construct it with `ThroughputWindow::new(10.0)` and feed it `done` deltas
instead of per-batch counts. Its existing unit tests stay green.

### New pure function: `done_delta`

```rust
/// Completions since the previous sample. Clamps to 0 so deletions
/// (which lower the done count) never produce a negative throughput sample.
pub fn done_delta(prev_done: i64, now_done: i64) -> usize {
    (now_done - prev_done).max(0) as usize
}
```

Lives in `pipeline/throughput.rs` next to `ThroughputWindow`, unit-tested.

### Removed from `pipeline/mod.rs`

- `let mut throughput_window = throughput::ThroughputWindow::new(15.0);`
- `let pipeline_start = Instant::now();`
- The post-batch block that records into the window, computes `raw_rate`, calls
  `effective_rate`, and writes `throughput_ema` (lines ~545–561).
- The idle-branch `throughput_ema` reset (lines ~196–199) — the sampler owns reset now.

`effective_rate` is removed if it has no other callers (grep before deleting; its tests go
with it). `emit_progress` and `get_processing_status` are untouched.

### Frontend changes

The atomic and the `pipeline_stats` payload (`{ total_pending, images_per_sec }`) are
unchanged, but `images_per_sec` is now a clean sentinel: **exactly `0` means "not enough
samples yet", any `> 0` is a real measured rate.**

- `search-bar.component.html`: drop the `>= 0.1` threshold. Show "Calculating ETA…" only
  when `images_per_sec === 0`; otherwise show `img/s` + ETA. A genuinely slow pipeline now
  shows a long-but-real ETA instead of being stuck on "Calculating".
- `photo.service.ts`: remove the hold-last-known speed hack in the `pipelineStats$`
  subscription (lines ~149–154) — the value is now refreshed every second by the sampler,
  so a stale-0 heartbeat is no longer a concern. Set the payload straight through.

## Data flow

```
sampler task (1s) ── reads done count ──▶ done_delta ──▶ ThroughputWindow(10s)
        │                                                        │
        └────────────── store rate ───▶ AppState.throughput_ema ◀┘
                                              │
                  ┌───────────────────────────┴───────────────────────────┐
          emit_progress (pipeline_stats event)            get_processing_status (1s poll)
                  │                                                         │
                  └────────────────────▶ frontend pipelineStats signal ◀───┘
                                                  │
                                  search-bar badge: img/s + ETA (or "Calculating ETA…")
```

## Edge cases

- **Deletions mid-processing** → negative raw delta, clamped to 0 by `done_delta`.
- **Idle library** → `total_pending == 0` branch stores 0 and idles; no busy SQLite polling.
- **Slow pipeline (<0.1 img/s)** → real positive rate, real long ETA — no longer hidden.
- **App shutdown** → fire-and-forget task like the pipeline loop; no teardown needed.
- **Startup transient** → window has <2 samples for ~1–2s → rate 0 → "Calculating ETA…",
  which resolves within ~2s of work beginning.

## Testing

- `ThroughputWindow` — existing unit tests unchanged.
- `done_delta` — new unit tests: positive delta, zero delta, clamp on negative (deletion).
- The sampler loop is thin orchestration over pure functions + a SQLite read; the tokio
  timing is left untested, consistent with how the pipeline loop is structured.
- Manual verification: import a folder, confirm the badge shows a stable `img/s` + ETA
  within ~2s and never sticks on "Calculating ETA…".

## Out of scope

- No change to the queue schema, the `done` definition, or `get_processing_counts`.
- No new IPC commands or event types.
- No UI restyling beyond the threshold/label logic in the badge.
