# Subject-Model Switching with Data Preservation — Design

**Date:** 2026-07-02
**Status:** Approved design, pre-implementation
**Context:** `docs/people-recognition-model-switching-findings.md`

## Goal

Make the `subject_model` setting actually control which face-recognition preset the pipeline uses (§1 wiring bug), and make switching presets preserve all user-facing people data — subject names, face→subject assignments, must-link/cannot-link constraints, subject tags, thumbnails — instead of the current full wipe (`reset_all_subject_data`). Only data that is genuinely model-specific (face embeddings, face edges, merge suggestions) is recomputed.

## Key insight

`faces` rows contain no embedding — just `(image_id, bbox, subject_id, scores)`. Embeddings live in `face_vectors`, keyed by face id. If face ids stay stable across a model switch, then `subject_id` assignments, `constraints (face_a, face_b)`, and `subjects.thumbnail_face_id` all survive with zero migration. Face ids are kept stable by matching new-model detections to existing face rows by bbox IoU per image.

## Chosen approach

**In-place face-row update via IoU matching.** Alternatives considered and rejected: shadow-table snapshot/replay (same result, more moving parts, empty-UI window) and subject-centroid re-linking after a wipe (cannot preserve constraints).

## Schema change

One entry appended to `VERSIONED_MIGRATIONS` (`db/mod.rs`):

```sql
ALTER TABLE faces ADD COLUMN embedder_id TEXT NOT NULL DEFAULT 'buffalo_s_recognition';
```

- The stamped identifier is the **embedder `ModelSpec.id`** (`preset.embedder.id`, e.g. `"buffalo_s_recognition"`, `"antelopev2_recognition"`), not the preset id. The column's sole invariant is vector comparability, and only the recognizer model determines that — detector and gender/age models don't. Presets that share a recognizer therefore never trigger re-embedding of each other's rows.
- `NOT NULL`, backfilled by the `DEFAULT` — factually correct for all legacy rows, since the §1 hardcode means every embedding produced to date came from `BUFFALO_S_PRESET`'s recognizer, even on installs showing "Standard" as active. No NULL-handling is needed anywhere downstream.
- `BASE_SCHEMA`'s `faces` definition gains the same column for fresh installs.
- Insert/update code always sets `embedder_id` explicitly to the active preset's embedder id; the `DEFAULT` exists only for the migration backfill.
- No changes to `face_vectors`, `constraints`, `subjects`, `merge_suggestions`, `dismissed_pairs`.

## Wiring fix (§1)

- `run_pipeline` (`pipeline/mod.rs`) no longer hardcodes `BUFFALO_S_PRESET`. The preset is resolved **per batch inside the loop**: read the `subject_model` setting, `FaceIdPreset::find_by_id`, fall back to `BUFFALO_S_PRESET`.
- The `FaceAnalyzer` is rebuilt only when the resolved preset differs from the currently loaded one; otherwise the cached analyzer is reused.
- Per-batch resolution means a mid-session switch takes effect without restarting the pipeline loop — no signalling/restart machinery needed.

## Switch flow (`settings/commands.rs`, key `"subject_model"`)

On change:

1. `ensure_ready` for the new preset's detector/embedder/gender-age models (unchanged from today).
2. If the new preset's `embedder.id` equals the old one's, skip staleness entirely — existing vectors remain valid. Otherwise, replace the `reset_all_subject_data` call with a new `people::repo::mark_subject_data_stale(pool)`:
   - `DELETE FROM merge_suggestions;`
   - `DELETE FROM face_edges;`
   - `UPDATE images SET subject_analysis_done = 0 WHERE deleted_at IS NULL;`
   - Re-enqueue all non-deleted images on the `'subject'` pipeline (dedupe against existing queue rows).
   - **No deletes** of `faces`, `face_vectors`, `subjects`, `constraints`.
3. Persist the setting (unchanged).

`reset_all_subject_data` remains available for an explicit user-initiated "reset people data" action, but is no longer invoked on model switch.

## Per-image reprocessing (subject pipeline)

For each dequeued image:

1. Detect faces with the current preset.
2. Load the image's existing `faces` rows. Match each new detection to at most one existing row by **IoU ≥ 0.5** (greedy, highest IoU first).
3. **Matched:** update the existing row in place — bbox, `det_score`, `quality_score`, `embedder_id` — and replace its row in `face_vectors`. Face id (hence `subject_id`, constraints, thumbnail references) is preserved.
4. **New detection, no match:** insert a fresh face row (`subject_id = NULL`, current `embedder_id`) + vector.
5. **Existing row unmatched by any new detection:** delete it. FK cascades clean up its constraints and edges, but its `face_vectors` row must be deleted explicitly — `face_vectors` is a `vec0` virtual table and does not participate in FK cascades.
6. Mark `subject_analysis_done = 1`.

This same code path serves both first-time analysis (no existing faces → everything inserts) and re-analysis after a switch — no separate migration mode.

## Clustering guard (mixed-state safety)

During migration the library holds vectors from two models. `people/clustering.rs` (edge building, `cluster_unassigned_faces`, merge-suggestion generation) must filter to `faces.embedder_id = <current preset's embedder id>` so cross-model vector comparisons never occur. Stale faces keep displaying their existing subject assignments in the UI until reprocessed.

## Failure & resume

The migration is just queue items on the existing `embedding_queue` `'subject'` pipeline: background, resumable across app restarts, retried on error. No new state machine or progress store.

## Testing

- Repo test: IoU matcher updates in place, preserving face id, `subject_id`, and constraint rows; unmatched old faces are deleted with cascades; unmatched detections insert unassigned.
- Repo test: `mark_subject_data_stale` preserves `subjects`, `faces`, `face_vectors`, `constraints`; clears `merge_suggestions`/`face_edges`; re-enqueues images.
- Clustering test: faces with differing `embedder_id` are never joined by an edge or merge suggestion; migration test confirms legacy rows are backfilled to `'buffalo_s_recognition'`.
- Switch-flow test: changing to a preset with the same embedder id does not mark data stale.
- Pipeline test (or manual verification): changing `subject_model` mid-session causes the next batch to use the new preset's analyzer.

## Out of scope (follow-ups)

- Adding `buffalo_l` as a new/replacement "Standard" tier (findings §4) — a registry-only change once this lands.
- Migration progress UI beyond what the queue already surfaces.
- Constraint re-derivation heuristics (unnecessary under this design — constraints survive by id).
