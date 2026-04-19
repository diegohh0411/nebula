# Anchor-Guided Recluster Design

**Date:** 2026-04-18  
**Status:** Approved  
**Scope:** `clustering.rs`, `db.rs` (one new query)

---

## Problem

The current `recluster_all()` assigns HDBSCAN clusters to subjects via majority vote: whichever `subject_id` appears most often among a cluster's faces wins. User corrections (`is_manual = 1`) are respected in the sense that manual faces are excluded from cluster mutation — but corrections don't actually *inform* the labeling decision for non-manual faces. A user fixing grandma's face does not make the algorithm better at finding grandma elsewhere.

## Goal

User-confirmed face assignments should act as labeled anchors that guide cluster-to-subject label assignment during every recluster, without risking destabilization of well-formed clusters.

## Approach: Anchor-Centroid Matching

Replace majority-vote with nearest-anchor assignment:

1. Before HDBSCAN runs, compute a **per-subject anchor centroid** — the mean embedding of that subject's `is_manual = 1` faces. If a subject has no manual faces, fall back to the mean of all its faces.
2. HDBSCAN runs unchanged on all embeddings (geometry is unaffected by corrections).
3. For each resulting cluster, compute its centroid (mean of member embeddings) and find the nearest anchor subject by cosine similarity.
4. If `best_score > ANCHOR_MATCH_THRESHOLD` → assign cluster to that subject.
5. If two clusters map to the same anchor → generate a merge suggestion (same mechanism as today).
6. If no anchor matches → create a new subject (same as today).

Manual faces remain excluded from `cluster_to_face_indices` (same as today), so they are never reassigned.

## Constants

```rust
const ANCHOR_MATCH_THRESHOLD: f32 = 0.45;
// Slightly above CLUSTERING_THRESHOLD (0.4) to reduce false anchor matches.
```

## Changes

### `db.rs` — one new query

```rust
// Returns (subject_id, embedding_blob) for all is_manual = 1 faces.
pub async fn get_manual_face_embeddings_by_subject(
    pool: &SqlitePool,
) -> Result<Vec<(i64, Vec<u8>)>>
```

### `clustering.rs` — replace majority-vote block

New helper:

```rust
fn compute_anchor_centroids(
    manual_embeddings: &[(i64, Vec<f32>)],   // (subject_id, embedding)
    all_subject_embeddings: &[(i64, Vec<f32>)], // fallback if no manual faces
) -> HashMap<i64, Vec<f32>>
// Returns subject_id → centroid embedding (mean of anchor faces, or all faces).
```

Updated `recluster_all()` cluster-assignment block (replaces lines 55–82 in current `clustering.rs`):

```
for each HDBSCAN cluster (label >= 0):
    cluster_centroid = mean(embeddings[face_indices])
    best_subject = None
    best_score = 0.0
    for (subject_id, anchor_centroid) in anchor_centroids:
        score = cosine_similarity(cluster_centroid, anchor_centroid)
        if score > best_score:
            best_score = score
            best_subject = subject_id
    if best_score > ANCHOR_MATCH_THRESHOLD:
        assign cluster → best_subject
    else:
        assign cluster → new subject
```

Merge suggestion when two clusters share the same best anchor is handled by `find_merge_suggestions` (unchanged — runs after assignment as today).

## What Does NOT Change

- `find_merge_suggestions` — untouched
- Noise handling (`label == -1` → unassigned)
- Thumbnail auto-assignment
- `ReclusterResult` struct
- `embedder.rs` — online matcher, worker loop, recluster trigger
- `face_id` crate and `VisionEngine` — untouched
- DB schema — no migrations needed

## Safety Properties

- A single user correction cannot destabilize well-formed clusters because HDBSCAN determines cluster geometry independently of labels. Corrections only influence which label attaches to a cluster after geometry is settled.
- Subjects with zero manual faces behave exactly as before (centroid falls back to all-faces mean, approximating the old majority-vote behavior).
- The `is_manual` exclusion guard on `cluster_to_face_indices` is preserved — manual faces are never moved by recluster regardless of anchor matching outcome.

## Informed By

- Apple Photos two-pass agglomerative clustering (anchor-seeded label assignment after geometry pass)
- Immich DBSCAN-derived approach (named/confirmed faces as labeled seeds)
- Research: semi-supervised HDBSCAN label propagation via minimum spanning tree (Malzer & Baum 2020)
