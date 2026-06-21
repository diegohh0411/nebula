# Incremental KNN clustering with idle backstop

**Date:** 2026-06-21
**Status:** Approved (design)
**Slices touched:** `people` (clustering, repo), `pipeline`

## Problem

During an import, the inference pipeline freezes for entire minutes at a time. The
pipeline driver loop (`src-tauri/src/pipeline/mod.rs:161`) is sequential:

1. Pull a batch of pending images (`batch_size`).
2. Decode → embed + face inference → save faces.
3. `if processed_subject_work { cluster_unassigned_faces(&pool).await }`
   (`mod.rs:551-553`).
4. Loop back to pull the next batch.

Step 3 is `.await`ed *inline on the critical path*. `cluster_unassigned_faces` runs
one sqlite-vec KNN query **per face over the entire library**
(`people/clustering.rs:141-167`) — ~14,314 sequential queries taking 300s+. While it
runs, the loop never returns to step 1, so inference sits at `0.0 img/s` with work
pending. Worse, it re-runs the **full-library** sweep after *every* batch that found a
face, redoing the whole 14k-face sweep many times during a single import. The separate
sampler task keeps logging, which is why inference looks frozen while the sampler is
alive.

Observed:

```
[clustering] knn progress 2250/14314 faces in 314.0s
[sampler] 0.0 img/s (285 pending)   # repeated for minutes
```

## Root cause

The expensive work (the full KNN sweep) runs synchronously inside the single-threaded
pipeline driver loop, blocking the next inference batch until it completes. The KNN
sweep is the *only* expensive part of `cluster_unassigned_faces`; everything after it
(union-find, constraints, label actions, subject assignment, thumbnail upgrade, merge
suggestions) reads the persisted `face_edges` table and is in-memory cheap.

## Goals

- Inference never stalls waiting on clustering.
- People view still updates live during a long import (new people appear as faces are
  processed).
- The mutual-kNN edge graph is exact once an import settles.

## Non-goals

- Exact byte-for-byte equivalence with a full resweep *at every instant during* an
  import. We accept transient approximation while the queue is draining; an
  authoritative full sweep reconciles at idle.
- Changing the clustering algorithm's semantics (mutual kNN + τ_sim + union-find +
  constraints all unchanged).

## Design

### Correctness model: approximate during import, authoritative at idle

- **Per batch (active import):** only compute KNN edges for newly-vectorized faces (and
  their immediate neighbors, to preserve mutuality for the *new* edges). Cheap — a
  handful of queries. May briefly miss/keep a stale mutual edge between two pre-existing
  faces whose neighborhoods a new face perturbed; this is intentional.
- **At idle (queue drained):** run one authoritative full sweep
  (`cluster_unassigned_faces`) that clears and rebuilds the entire edge graph,
  reconciling any drift. Nothing is waiting on inference at that point.

### Refactor `people/clustering.rs` into three pieces

Today `cluster_unassigned_faces` does: clear edges → full KNN → mutual edges → upsert →
union-find/constraints → label actions → assign → cleanup → thumbnails → merge
suggestions. Split the cheap "back half" out so it can be reused:

- **`relabel_from_edges(pool)`** — the back half, KNN-free. Load persisted edges via
  `people_repo::get_all_similarity_edges`, all face IDs, constraints, and subject names;
  build union-find with constraints; run `compute_label_actions`; apply
  assign/new-subject/noise writes; `delete_subjects_with_no_faces`;
  `auto_assign_missing_thumbnails`; `find_merge_suggestions`. Returns `ReclusterResult`.
  In-memory union-find over ~14k faces + a few writes — milliseconds.

- **`update_edges_incremental(pool, new_face_ids)`** — for the new faces only:
  - Build the affected set `S = new_face_ids ∪ {candidate neighbors of each new face}`.
  - Compute subject-aware KNN for every face in `S` into a local
    `HashMap<i64, Vec<(i64, f32)>>` (reusing the existing `build_subject_aware_knn`
    logic, restricted to `S`). Computing both endpoints' neighbor lists is what lets
    `compute_mutual_sim_edges` evaluate mutuality correctly for the new edges.
  - Run `compute_mutual_sim_edges` over the local map and `upsert_face_edge` the
    results. Does **not** `clear_all_face_edges` and does **not** remove now-stale edges
    — the idle backstop reconciles those.

- **`cluster_unassigned_faces(pool)`** (full, behavior unchanged) — `clear_all_face_edges`
  → full `build_subject_aware_knn` over all faces → mutual edges → upsert →
  `relabel_from_edges`. Serves as the idle backstop. Existing integration tests keep
  calling this entry point and remain valid.

Note: thumbnail-upgrade + eager face-crop generation + `subjects_updated` emit currently
live in `pipeline/mod.rs:554-585` around the clustering call, not inside
`cluster_unassigned_faces`. Those stay in the pipeline layer and run after both the
per-batch relabel and the idle full sweep (factor into a small local helper to avoid
duplication).

### New repo helper (`people/repo.rs`)

```rust
pub async fn get_face_ids_with_vectors_above(pool: &SqlitePool, after_id: i64) -> Result<Vec<i64>>
// SELECT rowid FROM face_vectors WHERE rowid > ? ORDER BY rowid
```

### Pipeline loop changes (`pipeline/mod.rs`)

- Add loop-scoped state before the `loop {`: `let mut last_clustered_face_id: i64 = 0;`
  and `let mut clustering_dirty = false;`.
- Replace the inline block at `mod.rs:551-588`:
  - When `processed_subject_work`:
    1. `let new_ids = get_face_ids_with_vectors_above(&pool, last_clustered_face_id)`.
    2. `update_edges_incremental(&pool, &new_ids)`.
    3. `relabel_from_edges(&pool)`.
    4. Run the thumbnail-upgrade/eager-crop/emit helper.
    5. Advance `last_clustered_face_id` to the max of `new_ids` (when non-empty).
    6. `clustering_dirty = true;`
  - All steps are cheap → the loop immediately pulls the next batch.
- In the idle branch (`mod.rs:190`, both queues empty): if `clustering_dirty`, run the
  full `cluster_unassigned_faces(&pool)` once, run the thumbnail/emit helper, then set
  `clustering_dirty = false`. (Keep the existing 2s sleep/continue.)

### High-water mark semantics

Face IDs are autoincrement `rowid`s, so `> last_clustered_face_id` reliably selects
faces vectorized since the last incremental pass. On a fresh process start
`last_clustered_face_id = 0`; the first idle backstop (or first incremental pass) covers
the backlog. Faces created out-of-band (e.g. user split) are reconciled by the next idle
full sweep.

## Data flow

```
import active:
  batch -> inference -> save_faces
        -> new_ids = face_vectors.rowid > last_clustered_face_id
        -> update_edges_incremental(new_ids)   # few KNN queries
        -> relabel_from_edges()                # in-mem union-find, ms
        -> thumbnails + emit subjects_updated   # live People view
        -> last_clustered_face_id = max(new_ids); dirty = true
  (loop pulls next batch immediately)

queue drained (idle):
  if dirty:
    cluster_unassigned_faces()                 # full clear+resweep+relabel
    thumbnails + emit
    dirty = false
  sleep(2s)
```

## Error handling

- `update_edges_incremental` / `relabel_from_edges` return `Result`; on `Err` log
  `[pipeline] incremental clustering failed: {e}` and continue the loop (do not block
  inference). Matches today's `else { error!("[pipeline] Clustering failed") }` posture.
- Empty `new_ids` (subject work that produced no new vectors): skip
  `update_edges_incremental`, still run `relabel_from_edges` (constraints/assignments may
  have changed). Cheap.

## Testing

Reuse the existing in-memory sqlite-vec integration harness in `clustering.rs` tests.

- **Refactor safety:** existing `cluster_unassigned_faces` integration tests
  (`integration_*`, `crowded_subject_*`, `unassigned_face_*`,
  `graph_suggestions_*`) must pass unchanged — they pin the full-sweep behavior.
- **`relabel_from_edges` equivalence:** seed `face_edges` directly, call
  `relabel_from_edges`, assert the same assignments/new-subjects a full sweep would
  produce from those edges.
- **`update_edges_incremental` builds the right edges:** seed an existing assigned
  cluster, add a new face inside it, call `update_edges_incremental` with just that face
  id, assert the expected mutual edge is upserted and a subsequent `relabel_from_edges`
  assigns it.
- **Incremental + idle convergence:** start from empty, add faces in two incremental
  batches, then run the full `cluster_unassigned_faces`; assert the final state equals a
  single full sweep over all faces (backstop reconciles approximation).
- **High-water mark:** `get_face_ids_with_vectors_above` returns only rowids strictly
  greater than the argument, ordered.

## Files

- `src-tauri/src/people/clustering.rs` — split into `relabel_from_edges`,
  `update_edges_incremental`; refactor `cluster_unassigned_faces` to reuse the back half.
- `src-tauri/src/people/repo.rs` — add `get_face_ids_with_vectors_above`.
- `src-tauri/src/pipeline/mod.rs` — high-water mark + dirty flag, per-batch incremental
  path, idle backstop; extract thumbnail/emit helper.
