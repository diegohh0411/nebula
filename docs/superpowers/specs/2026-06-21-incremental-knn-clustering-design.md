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
    `HashMap<i64, Vec<(i64, f32)>>` by calling `build_subject_aware_knn` with the full
    list of all vectorized face IDs (for correct `subject_sizes` calculation) but
    restricted to query only the subset `S`. Computing both endpoints' neighbor lists is
    what lets `compute_mutual_sim_edges` evaluate mutuality correctly for the new edges.
  - Run `compute_mutual_sim_edges` over the local map and `upsert_face_edge` the
    results. Does **not** `clear_all_face_edges` and does **not** remove now-stale edges
    — the idle backstop reconciles those.

- **`cluster_unassigned_faces(pool, cancel)`** (full sweep with cancellation support)
  - Accepts an optional cancellation check closure: `cancel: Option<&dyn Fn() -> bool>`.
  - Runs the read-heavy `build_subject_aware_knn` first over all faces. It checks the
    `cancel` check periodically (e.g., every 250 faces) and aborts early if new work has
    entered the queues.
  - Once KNN is computed and verified not cancelled, it runs `clear_all_face_edges`,
    computes mutual edges, and upserts them in a single transaction. This keeps the
    `face_edges` table populated during the entire 300s KNN computation.
  - Runs `relabel_from_edges`. Serves as the idle backstop.

### Refactor `build_subject_aware_knn`

Widen the signature to allow querying a subset of faces while still using the full set of
face IDs to compute correct subject sizes:
```rust
async fn build_subject_aware_knn(
    pool: &SqlitePool,
    all_face_ids: &[i64],      // For computing correct subject sizes
    faces_to_query: &[i64],    // The subset to actually run KNN queries for
    face_subjects: &HashMap<i64, i64>,
    k: usize,
    cancel: Option<&dyn Fn() -> bool>, // Optional cancellation check
) -> Result<HashMap<i64, Vec<(i64, f32)>>>
```

### New repo helper (`people/repo.rs`)

```rust
pub async fn get_face_ids_with_vectors_above(pool: &SqlitePool, after_id: i64) -> Result<Vec<i64>>
// SELECT rowid FROM face_vectors WHERE rowid > ? ORDER BY rowid
```

### Pipeline loop changes (`pipeline/mod.rs`)

- At startup, load `last_clustered_face_id` and `clustering_dirty` from database settings
  (defaulting to `0` and `false` respectively).
- Replace the inline block at `mod.rs:551-588`:
  - When `processed_subject_work`:
    1. `let new_ids = get_face_ids_with_vectors_above(&pool, last_clustered_face_id)`.
    2. If `new_ids` is not empty, call `update_edges_incremental(&pool, &new_ids)`.
    3. Call `relabel_from_edges(&pool)`.
    4. Run the thumbnail-upgrade/eager-crop/emit helper.
    5. On success, advance `last_clustered_face_id` to the max of `new_ids` (when non-empty)
       and persist it to the DB settings.
    6. Set `clustering_dirty = true` and persist it to the DB settings.
  - All steps are cheap → the loop immediately pulls the next batch.
- In the idle branch (`mod.rs:190`, both queues empty):
  - If `clustering_dirty` is true:
    1. Run `cluster_unassigned_faces(&pool, Some(&|| queue_has_work(&pool)))`.
    2. If it completed without cancellation, run the thumbnail/emit helper, then set
       `clustering_dirty = false` and persist it to the DB settings.
  - (Keep the existing 2s sleep/continue.)

### High-water mark semantics

Face IDs are autoincrement `rowid`s, so `> last_clustered_face_id` reliably selects
faces vectorized since the last incremental pass. On startup, we recover the last
successfully processed ID from settings. Faces created out-of-band (e.g. user split)
are reconciled by the next idle full sweep.

## Data flow

```
import active:
  batch -> inference -> save_faces
        -> new_ids = face_vectors.rowid > last_clustered_face_id
        -> update_edges_incremental(new_ids)   # few KNN queries
        -> relabel_from_edges()                # in-mem union-find, ms
        -> thumbnails + emit subjects_updated   # live People view
        -> last_clustered_face_id = max(new_ids); dirty = true (persisted)
  (loop pulls next batch immediately)

queue drained (idle):
  if dirty:
    cluster_unassigned_faces(cancel_if_new_work) # full sweep (deferred clear)
    if not cancelled:
      thumbnails + emit
      dirty = false (persisted)
  sleep(2s)
```

## Error handling

- `update_edges_incremental` / `relabel_from_edges` return `Result`; on `Err` log
  `[pipeline] incremental clustering failed: {e}` and continue the loop (do not block
  inference).
- If the incremental pass fails, `last_clustered_face_id` is NOT advanced in settings, so
  the next batch will retry processing the failed faces.
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
  `update_edges_incremental`; refactor `cluster_unassigned_faces` and `build_subject_aware_knn`
  to support subsets and cancellation.
- `src-tauri/src/people/repo.rs` — add `get_face_ids_with_vectors_above`.
- `src-tauri/src/pipeline/mod.rs` — settings-backed state recovery, cancellation callback
  definition, incremental path, and cancel-aware idle backstop; extract thumbnail/emit helper.
