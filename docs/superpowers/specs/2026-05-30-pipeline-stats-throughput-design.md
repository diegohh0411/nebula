# Pipeline Stats & Throughput Badge — Design

**Date:** 2026-05-30
**Task:** TT-1
**Status:** Approved
**Goal:** Surface real-time inference throughput (images/sec) in the UI during pipeline runs, while simplifying the existing progress badge into a single non-technical indicator aimed at everyday users.

---

## Context

Nebula's pipeline coordinator (`src-tauri/src/pipeline/mod.rs`) processes images through decode → embed → face stages and emits a `processing_progress` event after each batch. The frontend search bar consumes this to show a technical badge: "N recognizing · M indexing". There is no visibility into how fast processing is happening.

This design replaces `processing_progress` end-to-end with a unified `pipeline_stats` event carrying `total_pending` and `images_per_sec`, and redesigns the badge for non-technical users.

---

## Non-goals

- Per-stage timing breakdown (decode ms, embed ms).
- Exposing raw semantic/subject pending counts in the UI.
- Any change to pipeline throughput or scheduling behavior.

---

## Backend

### New payload type

Remove `ProcessingProgressPayload` from `src-tauri/src/models/`. Add:

```rust
pub struct PipelineStatsPayload {
    pub total_pending: u32,   // semantic_pending + subject_pending
    pub images_per_sec: f32,  // 0.0 when no recent data
}
```

### Updated `emit_progress`

`embedder::emit_progress` gains an `images_per_sec: f32` parameter:

```rust
pub async fn emit_progress(pool: &SqlitePool, app: &AppHandle, images_per_sec: f32) {
    if let Ok(status) = db::get_processing_counts(pool).await {
        let _ = app.emit("pipeline_stats", PipelineStatsPayload {
            total_pending: status.semantic_pending + status.subject_pending,
            images_per_sec,
        });
    }
}
```

Both call sites in `indexer.rs` (lines 444 and 447) pass `0.0`. The call site in `pipeline/mod.rs` passes the computed rate.

### Rolling-window tracker in `run_pipeline`

A `VecDeque<(Instant, usize)>` maintained at the top of the pipeline loop. Each entry records `(completion_time, images_completed_in_batch)`.

After each batch completes:
1. Push `(Instant::now(), batch_image_count)`.
2. Drain all entries where `entry.0.elapsed() > 5s`.
3. Compute `window_span = oldest_entry_time - newest_entry_time` (time between first and last retained entry; floor at 1 ms if only one entry). `rate = sum_images as f32 / window_span.as_secs_f32()`.
4. Pass `rate` to `emit_progress`.

**Window size: 5 seconds.** Responsive enough to feel live; stable enough to avoid erratic flickering on small batches.

Rate is `0.0` when the deque is empty (pipeline just started or idle).

---

## Frontend — data model & service

### `src/app/models/models.ts`

Remove `ProcessingStatus` and `ProcessingProgressEvent`. Add:

```ts
export interface PipelineStats {
  total_pending: number;
  images_per_sec: number;
}
```

### `TauriEventsService`

Replace:
```ts
readonly processingProgress$ = new Subject<ProcessingProgressEvent>();
// listen: 'processing_progress'
```
With:
```ts
readonly pipelineStats$ = new Subject<PipelineStats>();
// listen: 'pipeline_stats'
```

### `PhotoService`

Replace the `processingStatus` signal (typed `ProcessingStatus`) with:
```ts
readonly pipelineStats = signal<PipelineStats>({ total_pending: 0, images_per_sec: 0 });
```
Fed by `pipelineStats$` from `TauriEventsService`.

---

## Frontend — badge UI

### State machine

The search-bar component tracks a badge state signal with three values:

| State | Trigger | Clears |
|---|---|---|
| `active` | `total_pending > 0` arrives | — |
| `completing` | `total_pending` transitions from > 0 to 0 | auto after 2.5 s → `idle` |
| `idle` | 2.5 s after `completing`, or initial load | — |

A `setTimeout` handle is stored in the component to cancel pending transitions on rapid re-activation.

### Display

**`active`:**
```
● Processing 142 images · 18 img/s
```
- Pulsing dot (existing `.embed-badge-dot` style, unchanged).
- `img/s` segment only rendered when `images_per_sec ≥ 0.5` — suppresses noisy "0.1 img/s" during ramp-up or very slow batches.
- Numbers rounded to nearest integer.

**`completing`:**
```
Library up to date
```
- No dot. Same badge styling, no animation.

**`idle`:** badge hidden (same as current behavior when pending = 0).

### Template sketch

```html
@if (badgeState() !== 'idle') {
  <span class="embed-badge">
    @if (badgeState() === 'active') {
      <span class="embed-badge-dot"></span>
      Processing {{ photos.pipelineStats().total_pending }} images
      @if (photos.pipelineStats().images_per_sec >= 0.5) {
        · {{ photos.pipelineStats().images_per_sec | number:'1.0-0' }} img/s
      }
    } @else {
      Library up to date
    }
  </span>
}
```

The state machine logic lives entirely in `SearchBarComponent` — no new service needed.

---

## Affected files

| File | Change |
|---|---|
| `src-tauri/src/models/mod.rs` (or entities) | Remove `ProcessingProgressPayload`; add `PipelineStatsPayload` |
| `src-tauri/src/embedder.rs` | Update `emit_progress` signature and event name |
| `src-tauri/src/indexer.rs` | Pass `0.0` to updated `emit_progress` (4 call sites) |
| `src-tauri/src/pipeline/mod.rs` | Add rolling-window tracker; pass rate to `emit_progress` |
| `src/app/models/models.ts` | Remove old types; add `PipelineStats` |
| `src/app/services/tauri-events.service.ts` | Replace `processingProgress$` with `pipelineStats$` |
| `src/app/services/photo.service.ts` | Replace `processingStatus` signal with `pipelineStats` |
| `src/app/components/search-bar/search-bar.component.ts` | Add badge state machine |
| `src/app/components/search-bar/search-bar.component.html` | New badge template |

---

## Testing

- **Unit (Rust):** Rolling-window produces `0.0` on empty deque; rate converges correctly over a synthetic sequence of timed batch completions; entries older than 5s are drained.
- **Integration (Rust):** `emit_progress(pool, app, 12.5)` emits a `pipeline_stats` event with correct `total_pending` and `images_per_sec`.
- **Component (Angular):** Badge shows `active` when `total_pending > 0`; transitions to `completing` when it drops to 0; transitions to `idle` after 2.5 s; re-activates correctly if new work arrives during `completing`.
