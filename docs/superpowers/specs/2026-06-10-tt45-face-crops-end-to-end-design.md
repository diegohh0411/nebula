# TT-45 — Face crops end-to-end: progressive generation, quality & fast serving

**Status:** Design approved (pending spec review)
**Date:** 2026-06-10
**Task:** TT-45 (consolidates archived TT-35 progressive enrichment + TT-38 crop quality/speed)

## Problem & current state

TT-45 was written describing a problem state that predates the pipeline refactor (#18, 2026-05-29) and the TT-50 incremental clustering (#41, 2026-06-09). A git-grounded audit of current `main` shows Part A's backend is already in place:

- **Subjects form progressively.** `cluster_unassigned_faces` runs after every batch that produced faces (`pipeline/mod.rs:368`), assigns `face.subject_id`, and emits `subjects_updated`. Photo↔subject links are already incremental.
- **Thumbnail face is assigned incrementally** — `auto_assign_missing_thumbnails` (`clustering.rs:250`) fills `thumbnail_face_id` with the *largest* face for any subject lacking one, each batch.

What remains genuinely broken:

1. **Crops are lazy.** `get_face_crop` (`commands.rs:269`) renders the WebP only when the frontend first requests it → first-paint delay; the grid only warms if the user is sitting on the People page during import.
2. **Quality is naive.** `generate_face_crop` (`thumbnail.rs:26`) does a raw bbox crop then `thumbnail_exact(200,200)` — distorts aspect ratio, no padding (clips chins/foreheads), only 200px, and selection is "largest face," with no upgrade as better detections arrive. Crucially, `face_actor.rs:21` **discards** the detector's `detection.score` and `detection.landmarks` — the signal needed for quality scoring never reaches the DB.
3. **Serving** is already on the Tauri asset protocol (`convertFileSrc`) but suffers from the lazy-generation delay; cache-header behavior is unverified.

## Goal

During an active import, the People grid fills progressively — subjects show a **well-framed** profile picture and their already-linked photos as soon as clusters form — and crops load **without a visible delay**, with no regression to the final post-import state.

## Scope & approach

One spec, one plan, one PR, internally phased. Part A is treated as **verify + close the one gap (lazy crops)**; the bulk of the work is Part B (quality) and a thin Part C (serving).

### Phase 1 — Eager crop generation on assignment (closes Part A's gap)

- After `cluster_unassigned_faces` selects/upgrades a subject's `thumbnail_face_id` (per batch), **generate the crop immediately** from the source image, instead of waiting for a frontend request.
- `get_face_crop` remains as an idempotent lazy fallback (generates only if the file is missing).
- Effect: when `subjects_updated` fires mid-import, the crop file already exists → no first-paint delay.

### Phase 2 — Crop quality & best-detection selection (Part B, the core)

**Capture discarded signal.** Propagate `detection.score` and `detection.landmarks` from `face_actor` through `save_faces` (`pipeline/mod.rs:38`) into the DB. The face actor's `.map(|f| (f.detection.bbox, f.embedding))` must be widened to carry score + landmarks.

**Schema change (alpha — edit base schema directly, no migration).** The app is in alpha and the DB will be wiped, so add columns to the original `CREATE TABLE faces` in `BASE_SCHEMA` (`db.rs:82`) rather than adding a versioned migration:

```sql
det_score      REAL,   -- detector confidence (kept for debugging/retuning)
quality_score  REAL    -- final composite; drives selection
```

(These survive the existing migrations — migration 5 only drops `embedding`/`is_manual`.)

**Composite score**, computed at detect time while the image is decoded (no re-decode):

- `det_score` — detector confidence (already produced).
- **frontality** — symmetry/centering derived from the 5-point landmarks (eyes/nose/mouth) the detector already returns. Measures how level/centered the eyes are and how centered the nose sits between them.
- **sharpness** — variance-of-Laplacian over the face region (one cheap pass on the decoded crop). Normalized to a bounded range.
- Combined into a single normalized `quality_score` via documented weight constants (tunable in one place).

**Selection / upgrade.** Replace the fill-only "largest face" logic in `auto_assign_missing_thumbnails` with **highest-`quality_score`-wins**, re-evaluated each batch:

- If a higher-scoring face arrives, the profile crop **upgrades**: update `thumbnail_face_id` and regenerate the crop.
- **Never set `thumbnail_face_id` to NULL once set** → never reverts to blank.

**Framing fix** in `generate_face_crop`:

- Expand the bbox by a margin (~30–40%) so foreheads/chins are not clipped.
- **Square it centered on the face** — no aspect distortion (current `thumbnail_exact` squishes non-square crops). Clamp to image bounds.
- Render at **320px** WebP — covers the largest UI render (`w-32 h-32` = 128px People grid card, hi-DPI) with headroom. Tuned WebP quality.

### Phase 3 — Serving & load speed (Part C)

- **Eager warm** (Phase 1) already removes the main delay.
- Verify the Tauri asset-protocol response carries sensible **cache headers**; add `Cache-Control: immutable` (or equivalent) if missing. Crop filenames are already content-stable per `face_id`.
- Frontend: confirm **lazy-load** on the People grid (`loading="lazy"` / strategic) so off-screen crops don't block visible ones.

## Concurrency

The pipeline runs clustering **single-threaded after each batch** (one loop in `run_pipeline`), so cross-worker writes to a single subject are not actually concurrent today; the existing serialized clustering plus DB transactions satisfy the "guard against races" criterion. **Decision:** add a convergence *test* rather than new locking primitives. (Revisit if explicit guarding is later wanted.)

## Existing libraries (backfill)

A one-time **re-framing pass** regenerates crops for already-imported subjects with the new framing. Frontality/sharpness cannot be backfilled without re-detection, so pre-existing faces keep largest-face selection until re-detected; new imports get full scoring. **Decision:** re-framing only, no full re-detect backfill. (Given the alpha wipe, this may be moot — kept for completeness.)

## Testing & benchmark

- **Progressive test:** simulated batch → subjects gain crop file + linked photos progressively (assert crop exists after assignment, not only at end).
- **Quality/selection test:** faces with varied score/frontality/sharpness → selection picks the highest composite; a later better face upgrades; `thumbnail_face_id` never becomes NULL.
- **Framing test:** output crop is square, centered, within image bounds, no aspect distortion.
- **Convergence test:** repeated clustering over one subject yields a single stable crop/link set.
- **Benchmark:** median People-grid crop load time before/after on a ~400-photo library.

## Acceptance criteria (from task)

- [ ] During an import of 100+ photos, the People page shows subjects **with** profile pictures as soon as the first clusters form.
- [ ] Clicking a subject mid-import shows the photos already linked to it.
- [ ] Profile crop uses the best available detection and **upgrades** as better detections arrive; never reverts to blank.
- [ ] Crops are well-framed: centered, consistent padding, no clipping.
- [ ] Profile pictures load **without a visible delay** on a normal import.
- [ ] No regression on final state: after completion, all subjects have correct crops and full photo sets.
- [ ] Concurrent workers writing the same subject do not corrupt or duplicate crops/links.

## Key files

- `src-tauri/src/pipeline/face_actor.rs` — stop discarding `detection.score` / `landmarks`.
- `src-tauri/src/pipeline/mod.rs` — `save_faces` persists score/landmarks; eager crop after clustering.
- `src-tauri/src/db.rs` — base `faces` schema columns; replace `auto_assign_missing_thumbnails` with best-score selection/upgrade.
- `src-tauri/src/thumbnail.rs` — framing fix + 320px; quality-score helper (frontality, sharpness).
- `src-tauri/src/clustering.rs` — call best-score selection + eager crop generation.
- `src-tauri/src/commands.rs` — `get_face_crop` stays as idempotent fallback.
- `src/app/components/people-view/*` — lazy-load verification.

## Judgment calls flagged for review

1. **320px crop size** (vs 256 / 400).
2. **No explicit concurrency locking** — rely on serialized per-batch clustering + a convergence test.
3. **Re-framing-only backfill** (no re-detect for existing libraries) — likely moot given alpha wipe.
