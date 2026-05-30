# Fast Preview / Thumbnail Pipeline (TT-15)

**Status:** Design approved, ready for planning
**Date:** 2026-05-30
**Task:** TT-15 — "fix: make image preview/thumbnail generation more performant and swift"

## Problem

Loading a folder of ~400 images leaves the grid showing placeholders for *a couple of minutes*. Thumbnails arrive "on par with the inference pipeline" rather than near-instantly.

### Root cause

Despite TT-9 (#24) moving thumbnail generation "before ML inference," thumbnails are still produced **inside the inference pipeline loop** (`pipeline/mod.rs::run_pipeline`, Stage 1, ~lines 182–221):

1. **Cadence coupling.** The loop pulls a batch of `batch_size` (12) and only pulls the next batch after the current batch's Stage 2 (embed + face inference — the slow part) completes. Thumbnail work lives in Stage 1 of that same loop, so previews are delivered at *inference batch cadence*, not faster. TT-9's "background spawn" only stops the WebP **encode** from blocking dispatch; it does not decouple the *rate* of preview production from inference.
2. **Full-resolution decode.** `decoded_image::load_decoded` does a full `image::open()` (decode-once, shared with embed/face). Producing an 800px thumbnail therefore fully decodes a 24MP image, ignoring embedded EXIF thumbnails and DCT-scaled decode.

Additionally: the grid shows a placeholder with **no fallback** to the original until `thumbnail_path` is set, so the user stares at blanks the whole time. (The lightbox also renders from the thumbnail today — that concern is split into a separate task, see below.)

## Goals

- Grid previews appear **almost instantly** after a folder is added.
- **Hybrid** delivery: a decoupled fast background pass over all images **plus** viewport prioritization for what the user is currently looking at.
- Previews **win the CPU briefly, then yield** to the inference pipeline (no hard-blocking of inference).

## Non-goals

- Lightbox rendering the original full-resolution image. Split into its own task: *"feat: lightbox renders the original full-res image instead of the thumbnail"* (Notion, status Noticed, `370e954d-b476-8172-b5e5-c30a0ce3cbda`). `photo.service.ts` already exposes `originalUrl()` for that work.
- Any DB migration framework usage. The app is alpha; schema changes go inline into `BASE_SCHEMA` and the user wipes `APP_DATA` to reset.

## Chosen approach

**Approach 1 — a dedicated preview subsystem fully decoupled from the inference pipeline.** A new `src-tauri/src/preview.rs` module owns thumbnailing end-to-end. Thumbnail generation is **removed entirely** from `pipeline/mod.rs` Stage 1.

Rejected alternatives:
- *Preview-stage actor inside the pipeline* — still structurally coupled to the batch loop, the very thing we need to escape; risks recreating the TT-9 problem.
- *Pure on-demand lazy thumbnailing* — no background completion; fast scrolling outruns generation and folders never "finish."

**Accepted trade-off:** the inference pipeline loses decode-once sharing for thumbnails and re-decodes the full image itself. This is fine — its full decode is required for embed/face regardless, and the preview path now does its own *cheap* decode. The two subsystems share nothing but the DB and the CPU.

## Architecture

```
                          ┌─────────────────────────────┐
   indexer / startup ───► │   PreviewService            │
   (image_added,          │                             │
    folder scan)          │  • backlog feeder (DB scan) │
                          │  • priority queue (viewport)│ ──► worker pool
   frontend viewport ───► │  • burst→trickle governor   │     (spawn_blocking
   (prioritize_previews)  └─────────────────────────────┘      × N permits)
                                                                     │
                                                          tier-1 tiny ──emit──► grid
                                                          tier-2 800px ─emit──► grid
```

Components, each independently testable:

1. **`PreviewService`** — handle holding the priority queue, backlog cursor, and concurrency governor. Constructed once at startup, stored in `AppState`.
2. **Work sources**, both feeding one priority structure:
   - *Backlog feeder*: queries `images WHERE thumbnail_path IS NULL AND deleted_at IS NULL`, enqueues at **low** priority. Covers startup and newly-indexed images.
   - *Priority injector*: `prioritize_previews(image_ids)` command pushes viewport IDs at **high** priority.
3. **Worker pool** — bounded by a semaphore sized to the governor's current parallelism. Each worker: cheap-decode → write tier-1 → write tier-2 → update DB + emit.
4. **Governor** — moves pool parallelism between **burst** (`num_cpus::get()`) and **trickle** (1–2).

## Two-tier preview decode

### Schema (inline, no migration)

Add to `BASE_SCHEMA` `CREATE TABLE images` (`db.rs:23`):
```sql
preview_path TEXT
```
- `preview_path` → tiny instant tier, file `{id}_p.webp`
- `thumbnail_path` → existing 800px tier, file `{id}.webp` (unchanged)

Distinct filenames so the WebView asset cache never serves a stale tier. No `VERSIONED_MIGRATIONS` entry; user wipes `APP_DATA`.

### Tier 1 — instant (≤ ~200px, paints the grid)

1. **JPEG:** read the embedded EXIF thumbnail (add `kamadak-exif`); cameras embed ~160–320px JPEGs at near-zero decode cost. **Re-encode to WebP** for format consistency.
2. **Fallback** (no EXIF thumb, or PNG): DCT-scaled decode at 1/8 via `jpeg-decoder` `scale()` for JPEG; decode + fast downscale for PNG (typically small screenshots).

Write `preview_path`, update DB, emit `image_updated`.

### Tier 2 — 800px (current grid quality)

Scaled decode at 1/2 or 1/4 (enough headroom above 800px for clean CatmullRom resize), then reuse `thumbnail::write_thumbnail_from_image` to resize to 800px longest side and encode WebP. Replaces the current full-resolution `image::open()` for thumbnails — a large saving on its own. Write `thumbnail_path`, update DB, emit `image_updated` again.

### Decode helper

`decode_scaled(path, max_dim) -> DynamicImage` picks EXIF-thumb / DCT-scale / full-decode based on format and requested size, isolating format-specific fiddliness behind one tested interface. Confirm `jpeg-decoder`'s `scale` API is reachable through the `image` dep; add `jpeg-decoder` directly if not.

## Viewport prioritization & governor

**Priority queue** — `Arc<Mutex<PreviewQueue>>` with `high: VecDeque<i64>` (viewport) and `low: VecDeque<i64>` (backlog). Workers drain `high` before `low`. A `HashSet` of in-flight/done IDs dedupes so an image is never thumbnailed twice.

**Viewport signal** — new command:
```rust
#[tauri::command]
async fn prioritize_previews(image_ids: Vec<i64>, state) -> Result<(), String>
```
The grid, on scroll (debounced ~100ms), sends in/near-viewport IDs lacking a `thumbnail_path`. Backend promotes matching `low` IDs to `high`, ignores done IDs.

**Governor (burst → yield)** — shared `parallelism: AtomicUsize` read by the worker-spawn loop:
- **Burst:** on high-priority demand or a fresh folder scan, set parallelism = `num_cpus::get()` for a short window.
- **Trickle:** once `high` is empty and the burst window (~3–5s since last high demand) elapses, drop to 1–2.

Inference is never hard-blocked; it runs concurrently throughout. During a burst, previews out-compete it (more workers + cheap work) so the grid paints fast; then they back off and inference reclaims the CPU. Controlling *worker count* (portable) is chosen over OS thread-priority APIs (platform-specific, fragile on the Windows/iGPU-laptop target).

## Events & integration points

**Events:** reuse the existing `image_updated` event for both tiers — the grid already listens (`tauri-events.service.ts:34`) and re-reads the row; the row's `preview_path`/`thumbnail_path` indicate what's available. No new event type. Grid `thumbUrl()` becomes `thumbnail_path ?? preview_path` → `convertFileSrc`.

**Changes:**
1. `pipeline/mod.rs` — delete the Stage-1 thumbnail block (~182–221) and `thumb_sem`. Pipeline still emits `image_updated` after inference (analysis-done badges).
2. `db.rs` — add `preview_path` to `BASE_SCHEMA`, to `SELECT` column lists + row mapping; add `update_preview_path()` and a "needs preview" query (`thumbnail_path IS NULL AND deleted_at IS NULL`).
3. `models` — add `preview_path: Option<String>` to `Image` and `SearchResult`; mirror in `models.ts`.
4. `indexer.rs` — on `image_added`, notify `PreviewService` to enqueue at low priority (existing emit stays).
5. `lib.rs`/`main.rs` — construct `PreviewService` at startup, store in `AppState`, start the backlog feeder, register `prioritize_previews`.
6. `thumbnail.rs` — keep `write_thumbnail_from_image`; add `decode_scaled` (here or in `preview.rs`).

**Lifecycle / edge cases:**
- *Startup backlog:* feeder scans for un-thumbnailed images and trickles them in, completing interrupted imports.
- *Deleted images:* worker re-checks `deleted_at` before writing and skips.
- *Hash change (re-index):* clear both `preview_path` and `thumbnail_path` so stale previews regenerate.
- *Failure:* tier-1 decode failure falls through to tier-2 full decode; if both fail, log and leave paths NULL (placeholder stays). No queue poisoning.

## Testing strategy

Unit-tests-in-module, TDD during implementation.

**`decode_scaled`:** EXIF-thumb path used when present (dims ≤ target); DCT fallback when absent (non-empty, ≤ target); tier-2 of a >800px image fits 800×800 touching one edge; tiny source not upscaled; corrupt/missing file → `Err`, no panic.

**`PreviewQueue` (pure logic):** drains `high` before `low`; promoting a `low` ID removes it from `low`; enqueuing an in-flight/done ID is a no-op; empty queue idles without spinning.

**Governor:** high-demand arrival → parallelism = `num_cpus`; `high` empty + burst window elapsed → parallelism drops to trickle.

**DB:** `update_preview_path` persists and is readable; "needs preview" query returns only `thumbnail_path IS NULL AND deleted_at IS NULL`.

**Integration (end-to-end):** seed N images (temp DB + temp files) → run the service with inference *not* running → assert all get `preview_path` then `thumbnail_path` and `image_updated` fires.

**Manual (`verify`/`run` at the end):** add ~400 images, confirm previews paint in seconds, and scrolling pulls visible images forward.
