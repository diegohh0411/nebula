# Processing Label: Reliable Speed + ETA

**Date:** 2026-06-11
**Task:** TT-64
**Status:** Approved design
**Area:** Pipeline throughput stats (Rust/Tauri backend + Angular frontend)

## Problem

The processing badge in the search bar (`search-bar.component.html`) shows
`Processing {total_pending} images · {images_per_sec} img/s` while inference runs.

Two issues:

1. **Bug — speed disappears during large imports.** When importing ~200 images,
   both the count and the `img/s` speed render reliably. When importing ~1000+
   images, the speed segment vanishes for the duration of discovery while the
   count keeps updating. Inference is still running; the speed just reads as
   zero/blank.

2. **Feature — no ETA.** For large jobs (1000–2000 images) the user wants an
   estimated time to completion: `remaining / speed`, rendered in human units.

## Root Cause (confirmed)

The folder scanner overwrites the real speed with a hardcoded zero.

During a folder scan, `indexer.rs:449-453` calls
`emit_progress(&self.pool, &self.app, 0.0)` every 10 files and once at scan end.
`emit_progress` (`embedder.rs:47`) emits a `pipeline_stats` Tauri event carrying
`images_per_sec: 0.0` **and a freshly-counted `total_pending`**.

On the frontend, `photo.service.ts:121-123` subscribes to that event and calls
`this.pipelineStats.set(e)` with **no merge logic** — so each scanner heartbeat
clobbers the pipeline's real rate with `0.0`. The template only shows the speed
when `images_per_sec >= 0.1` (`search-bar.component.html:77`), so the segment
disappears.

This explains the size dependency:

- **Small import:** discovery finishes quickly → few zero-emits → the 1s poll
  (`photo.service.ts:133`, pull path reading `throughput_ema`) restores the real
  rate almost immediately → speed stays visible.
- **Large import:** discovery runs for minutes, firing `0.0` ~100 times → the
  real rate is continuously stomped for the whole discovery phase → speed
  vanishes. The **count survives** because the same emit refreshes
  `total_pending`.

A secondary, lower-likelihood factor: `ThroughputWindow::rate()`
(`throughput.rs:35`) returns `0.0` when fewer than two samples fall inside the
15s window. Under severe pipeline-loop stalls this could independently zero the
rate. Evidence suggests it is not the primary cause (the count keeps updating, so
the loop is not stalled for 15s), but the design hardens against it anyway.

## Goals

- The speed never reads `0`/blank while inference is actively running and a real
  rate has been observed at least once.
- Surface an estimated time to completion alongside the speed.
- Keep the backend `pipeline_stats` payload contract minimal.

## Non-Goals

- Reworking the import/discovery scan architecture or SQLite pool tuning.
- Per-stage (embed vs face) throughput breakdowns.
- Historical throughput charts.

## Design

### Single source of truth for speed (primary fix)

`AppState.throughput_ema` (`lib.rs:32`) is the canonical speed slot, written
**only** by the pipeline loop. `emit_progress` is refactored to stop accepting an
`images_per_sec` argument and instead read the canonical rate from
`app.state::<AppState>().throughput_ema` internally:

```rust
pub(crate) async fn emit_progress(pool: &SqlitePool, app: &AppHandle) {
    let rate = app
        .state::<crate::AppState>()
        .throughput_ema
        .load(Ordering::Relaxed);
    let images_per_sec = f32::from_bits(rate);
    if let Ok(status) = db::get_processing_counts(pool).await {
        let _ = app.emit("pipeline_stats", PipelineStatsPayload {
            total_pending: status.total_pending as u32,
            images_per_sec,
        });
    }
}
```

Effects:

- The scanner's two `emit_progress` calls (`indexer.rs:450,453`) lose their `0.0`
  argument and now carry the last real speed. The stomp is gone — the scanner
  heartbeat refreshes `total_pending` without touching the speed.
- The pipeline loop (`mod.rs:398`) stores `throughput_ema` first, then calls
  `emit_progress`, which reads the same value back — behavior unchanged on that
  path.

### Effective-rate resilience (hold-last-known at the source)

Harden against the secondary window-collapse factor. Extract a pure helper:

```rust
/// Returns `raw` when it is a usable (> 0) rate, else falls back to the last
/// published value so a transient "not enough samples" 0 never blanks the speed.
fn effective_rate(raw: f32, prev: f32) -> f32 {
    if raw > 0.0 { raw } else { prev }
}
```

At the pipeline publish site (`mod.rs:391-398`):

1. Compute `raw = throughput_window.rate(now_secs)`.
2. Load `prev = throughput_ema`.
3. `let effective = effective_rate(raw, prev);`
4. Store `effective` into `throughput_ema`.
5. Call `emit_progress` (which now reads `throughput_ema`).

When the queues drain (`mod.rs:158` empty-batch branch), reset `throughput_ema`
to `0.0` so a finished run does not leak a stale speed into the next import.

### Frontend hold-last-known (defense-in-depth)

In the `pipelineStats$` subscription (`photo.service.ts:121-123`), if an incoming
stat reports `images_per_sec <= 0` while `total_pending > 0`, preserve the prior
non-zero speed instead of writing the zero:

```ts
this.events.pipelineStats$.subscribe((e) => {
  const prev = this.pipelineStats();
  const images_per_sec =
    e.images_per_sec > 0 || e.total_pending === 0
      ? e.images_per_sec
      : prev.images_per_sec;
  this.pipelineStats.set({ ...e, images_per_sec });
});
```

Cheap insurance against any future zero-emitter; harmless given the backend fix.

### ETA derivation and formatting (feature)

ETA is derived on the frontend — formatting is a view concern and the payload
contract stays minimal.

`models.ts` — pure, unit-testable formatter (compact adaptive unit):

```ts
/** Human ETA: "~45s left", "~12 min left", "~2h 10m left". Empty when unknown. */
export function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '';
  if (seconds < 60) return `~${Math.round(seconds)}s left`;
  if (seconds < 3600) return `~${Math.round(seconds / 60)} min left`;
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return m > 0 ? `~${h}h ${m}m left` : `~${h}h left`;
}
```

`photo.service.ts` — derived signal:

```ts
readonly etaSeconds = computed<number>(() => {
  const s = this.pipelineStats();
  return s.images_per_sec > 0 ? s.total_pending / s.images_per_sec : 0;
});
```

`search-bar.component.html` — append after the `img/s` segment, under the same
`images_per_sec >= 0.1` guard so ETA appears exactly when a trustworthy speed
exists:

```html
@if (photos.pipelineStats().images_per_sec >= 0.1) {
  · {{ photos.pipelineStats().images_per_sec | number:'1.0-1' }} img/s
  @if (formatEta(photos.etaSeconds())) {
    · {{ formatEta(photos.etaSeconds()) }}
  }
}
```

## Data Flow (after)

```
pipeline loop ──> throughput_window.rate() = raw
              ──> effective_rate(raw, prev) ──> throughput_ema (canonical)
              ──> emit_progress() reads throughput_ema ──> pipeline_stats event
scanner       ──> emit_progress() reads throughput_ema ──> pipeline_stats event
                                                            (carries real speed)
get_processing_status (1s poll) reads throughput_ema ──> pull path
frontend signal ──(hold-last-known guard)──> pipelineStats
                ──> etaSeconds computed ──> formatEta() ──> badge
```

## Testing

**Rust (unit):**
- `effective_rate`: returns `raw` when `raw > 0`; returns `prev` when `raw == 0`.
- Existing `throughput.rs` tests remain green (no behavior change there).

**Frontend (Vitest):**
- `formatEta`: sub-minute (`~30s left`), minutes (`~12 min left`), hours with
  minutes (`~2h 10m left`), exact hour (`~2h left`), and unknown/zero/non-finite
  → `''`.
- `etaSeconds` computed: zero speed → `0`; normal case → `remaining / speed`.

**Manual verification:**
- Import 1000+ images and confirm the speed and ETA stay visible throughout
  discovery (the original repro).

## Risks / Notes

- Dropping the `images_per_sec` parameter from `emit_progress` touches all three
  call sites; the compiler enforces completeness.
- `emit_progress` reading `app.state::<AppState>()` requires `AppHandle`'s
  `Manager` trait in scope in `embedder.rs`.
- The frontend hold-last-known guard means the speed reflects the last real
  sample during a genuine slowdown; ETA may briefly lag reality but never blanks.
