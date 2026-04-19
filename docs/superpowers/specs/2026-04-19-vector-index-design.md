# Vector Index Overhaul — Design Spec

**Date:** 2026-04-19  
**Status:** Approved  
**Scope:** Replace per-query SQLite embedding scan with a persistent in-memory flat vector index

---

## Problem

Every semantic image search calls `db::get_all_embeddings()`, which loads all embedding BLOBs from SQLite into memory and computes cosine similarity one-by-one in a loop. There is no persistent in-memory index — every query is a cold load from disk. At 15k–50k photos this becomes a noticeable bottleneck.

---

## Goals

- Eliminate SQLite I/O on the hot search path
- Search latency under 30ms for up to 50k images (768-dim SigLIP2 embeddings)
- Persist the index to disk so startup after the first build is near-instant
- Abstraction layer that makes a future HNSW swap transparent to call sites
- Pure Rust, no C++ dependencies, minimal new crates (`half`, `rayon` already in ecosystem)

## Non-Goals

- ANN for face embeddings (HDBSCAN clustering loads them all at once already; that path is acceptable)
- Multi-user / server deployment
- Vectors beyond image semantic embeddings (face embeddings stay in SQLite)

---

## Architecture

```
VectorIndex (trait)
  └── FlatIndex (impl)          ← ships now
  // TODO: HnswIndex when library grows past ~200k images

IndexStore = Arc<RwLock<Box<dyn VectorIndex>>>
  ├── registered as Tauri managed state at startup
  ├── populated from .idx file (or rebuilt from SQLite on first run)
  ├── updated incrementally by embedder after each new image
  └── queried by search.rs — no SQLite I/O per search
```

No changes to `db.rs`, `clustering.rs`, `commands.rs`, or the Angular frontend.

---

## New Module: `src-tauri/src/vector_index.rs`

### Trait

```rust
// TODO: swap FlatIndex for HnswIndex if library grows past ~200k images
pub trait VectorIndex: Send + Sync {
    fn add(&mut self, id: i64, embedding: &[f32]);
    fn remove(&mut self, id: i64);  // tombstones; compacted at startup
    fn search(&self, query: &[f32], limit: usize) -> Vec<(i64, f32)>;
    fn len(&self) -> usize;
}
```

### `FlatIndex`

- In-memory store: `Vec<(i64, [f32])>` of pre-normalized unit vectors
- Normalization happens once in `add()` — dot product equals cosine similarity on unit vectors
- `search()` uses `rayon` parallel iterator: dot-product every entry against the query, then partial sort for top-k
- Deletions set `id = -1` (tombstone); no immediate file rewrite
- Compaction runs at startup when tombstone ratio exceeds 10%

---

## On-Disk Format (`.idx` file, lives next to `nebula.db`)

```
[magic:   8 bytes  "NEBULAVX"]
[version: u8]
[dim:     u32]
[count:   u64]
[entries: count × (i64 id + dim × f16 components)]
```

- **f16 on disk** (`half` crate): ~75 MB for 50k × 768-dim vs ~150 MB for f32
- **f32 in memory** after load: no precision loss on the hot math path
- Tombstoned entries (`id == -1`) are skipped during load
- A future `HnswIndex` uses a separate `.hnsw` file; the trait swap is transparent

---

## Startup Sequence

In app setup (`lib.rs`):

1. Call `FlatIndex::load_or_rebuild(data_dir, &pool)`:
   - If `.idx` exists and its `dim` field matches the current model's embedding size → deserialize (f16 → f32), compact tombstones
   - Otherwise → call `db::get_all_embeddings()` once, normalize all vectors, build index, write `.idx`
2. Wrap in `Arc<RwLock<FlatIndex>>` and register as Tauri managed state

---

## Incremental Write Path

In `embedder.rs`, immediately after `db::mark_semantic_analysis_done()`:

1. Normalize the new embedding to a unit vector
2. `index.write().add(image_id, &normalized)`
3. Append the single entry to the `.idx` file (append-only; no full rewrite)

---

## Delete Path

When an image is soft-deleted:

1. `index.write().remove(image_id)` — sets tombstone in memory
2. Tombstone is not immediately written to `.idx`; it is removed during the next startup compaction pass

To prevent soft-deleted images from leaking into results between a delete and the next startup compaction, `build_search_results` adds a `deleted_at IS NULL` check when loading each result image from SQLite. This is safe because `build_search_results` only touches the small result set (not all N images), so there is no I/O regression on the hot path.

---

## Search Path

In `search.rs`, replace:

```rust
let all_embeddings = db::get_all_embeddings(pool).await?;
// ... brute-force loop
```

with:

```rust
let results = index.read().search(&query_embedding, limit);
```

The gap heuristic (elbow method) in `search_images` is unchanged — it operates on the `Vec<(i64, f32)>` result list, not on the index internals.

---

## Dependencies

| Crate | Purpose | Already present? |
|-------|---------|-----------------|
| `half` | f16 encode/decode for on-disk storage | No — add to Cargo.toml |
| `rayon` | Parallel dot products in `FlatIndex::search` | Likely no — add to Cargo.toml |

No C++ or non-Rust dependencies introduced.

---

## File Changelist

| File | Change |
|------|--------|
| `src-tauri/src/vector_index.rs` | New — `VectorIndex` trait + `FlatIndex` impl |
| `src-tauri/src/lib.rs` | Add `FlatIndex::load_or_rebuild` call at startup; register state |
| `src-tauri/src/embedder.rs` | After `mark_semantic_analysis_done`, update index |
| `src-tauri/src/search.rs` | `search_images` takes `&IndexStore` instead of pool; `build_search_results` adds `deleted_at IS NULL` guard |
| `src-tauri/Cargo.toml` | Add `half`, `rayon` |

---

## Success Criteria

- Search latency ≤ 30ms at 50k images on a mid-range consumer laptop
- App startup with an existing `.idx` file completes index load in < 1s
- First-run rebuild from SQLite is a one-time cost, clearly indicated in logs
- All existing search behavior (gap heuristic, result shape) is preserved
- Face clustering and all other subsystems are unaffected
