# Constrained k-NN Graph Clustering for Face Labeling

**Date:** 2026-04-24
**Status:** Approved
**Scope:** `clustering.rs`, `db.rs`, `embedder.rs`

---

## Problem

Nebula's face clustering uses HDBSCAN with lightweight quantized models (Buffalo-S for detection/embedding). The same person's faces — across different poses, lighting, aging, and expressions — do not always occupy a single dense region in the embedding space. Instead, they form **transitive similarity chains**: face A matches face B, face B matches face C, but A and C do not directly match. HDBSCAN splits these chains into multiple clusters, producing duplicate subjects that the user must constantly merge.

The existing anchor-guided recluster spec (2026-04-18) improved label assignment but did not fix the root cause: HDBSCAN's density-based definition of a cluster is too strict for embeddings produced by consumer-CPU-friendly models.

## Goal

Replace HDBSCAN with a graph-based clustering algorithm that:
1. Naturally captures transitive similarity chains (A-B-C all in one cluster).
2. Learns from user corrections via semi-supervised must-link and cannot-link constraints.
3. Keeps all existing lightweight models — no heavier inference.
4. Runs fast on consumer CPUs with hundreds of images per day.

## Algorithm: Constrained k-NN Graph Clustering

### Core Idea

Treat each face as a node in an undirected graph. Edges connect faces that are sufficiently similar (k-NN with a cosine similarity threshold). Connected components of this graph are clusters. Must-link constraints are injected as additional edges. Cannot-link constraints are checked after component formation to detect conflicts.

This approach is a constrained variant of the graph-clustering pipeline widely used in face recognition (Chinese Whispers-style k-NN graph clustering, Biemann 2006), with semi-supervised constraints (Wagstaff et al. 2001).

### Constants

```rust
/// Minimum cosine similarity to create an edge between two faces.
const EDGE_THRESHOLD: f32 = 0.55;

/// Number of nearest neighbors to consider per face when building the graph.
const K_NEAREST: usize = 5;

/// Minimum component size to form a new subject (faces in smaller components stay unassigned/noise).
const MIN_COMPONENT_SIZE: usize = 2;

/// Minimum cosine similarity for a representative face to match an existing subject.
const SUBJECT_MATCH_THRESHOLD: f32 = 0.55;
```

### Graph Construction

1. Load all faces with embeddings from the database.
2. For each face, find its `K_NEAREST` neighbors by cosine similarity.
3. Add an undirected edge for each neighbor where `similarity > EDGE_THRESHOLD`.
4. Inject **must-link edges** between constrained face pairs (see below). These edges bypass the threshold.

### Connected Components

Run a standard connected-components algorithm (DFS/BFS or Union-Find) on the graph. Each component is a candidate cluster.

### Component-to-Subject Assignment

For each connected component:

1. **Count manual subjects** — how many distinct subjects have `is_manual = 1` faces in this component?
2. **If exactly 1 manual subject** → assign all unassigned faces in the component to that subject.
3. **If >1 manual subjects** → **conflict**. The component chains together people who the user has explicitly marked as different. All unassigned faces in this component stay unassigned (noise). Log a warning.
4. **If 0 manual subjects** → this is a new candidate.
   - If component size < `MIN_COMPONENT_SIZE` → noise.
   - Else, pick the **highest-degree node** as the representative face.
   - Compare the representative embedding to all existing subjects via cosine similarity.
   - If best match > `SUBJECT_MATCH_THRESHOLD` → assign component to that subject.
   - Else → create a new subject and assign all unassigned faces to it.

**Why highest-degree node instead of centroid?** Graph components can be long chains. The centroid of a chain can fall in empty space between nodes, producing a poor representative. The highest-degree node is the natural "hub" of the component — it has the most direct similarity evidence.

### Manual Face Protection

Faces with `is_manual = 1` are never moved by recluster. They remain in the graph as nodes (they anchor components to the correct subjects), but they are excluded from the set of faces that can be reassigned.

## Constraints

### Must-Link (Hard Constraint)

Must-link pairs are injected as edges directly into the graph, regardless of similarity. They are sourced from:

1. **`face_corrections` table**: Every correction where `new_subject_id IS NOT NULL` links the corrected face to the subject's existing manual faces. We materialize this as edges between the corrected face and up to 10 manual faces of the target subject (sampled uniformly; the sample choice does not affect correctness because transitive closure links them all).
2. **`is_manual = 1` faces within the same subject**: All pairwise combinations of manual faces for a subject are must-link. For subjects with >50 manual faces, we sample 50 faces to keep pair generation bounded.
3. **Subject merge history**: When a merge occurs, all faces of the merged subject become must-link with all faces of the target subject.

**Transitive closure is implicit**: the graph algorithm handles it. If A must-link B and B must-link C, A, B, and C end up in the same connected component.

### Cannot-Link (Soft Conflict Detection)

Cannot-link pairs are **not** edges. They are checked after components are formed:

- If a component contains `is_manual = 1` faces from two or more different subjects, it is flagged as a conflict.
- The conflict prevents automatic assignment of unassigned faces in that component.

Cannot-link sources:
- Any two subjects that both have at least one `is_manual = 1` face and have **not** been merged are implicitly cannot-link.

## DB Changes

### New Query

```rust
/// Returns (subject_id, Vec<face_id>) for all subjects that have at least one is_manual=1 face.
/// Used to build must-link pairs.
pub async fn get_manual_faces_by_subject(pool: &SqlitePool) -> Result<Vec<(i64, Vec<i64>)>>;
```

### No Schema Migrations

The algorithm uses existing tables only: `faces`, `face_corrections`, `subjects`, `merge_suggestions`.

## Pipeline Integration

### Batch Recluster (Primary Mode)

Entry point remains `clustering::cluster_unassigned_faces(pool)`. The internals are replaced:

```
1. Load all faces with embeddings (assigned + unassigned)
2. Load manual-face groups by subject
3. Build must-link pairs and inject as graph edges
4. Build k-NN graph with threshold
5. Compute connected components
6. For each component, assign unassigned faces using manual-subject rules
7. Delete empty subjects, auto-assign thumbnails, find merge suggestions
```

### Online Greedy Assignment (Immediate UX Improvement)

In `embedder.rs`, after `db::insert_face(...)` in `process_subject_one`:

```rust
// Greedy 1-NN assignment: load existing face embeddings, find the nearest
// neighbor by cosine similarity, and if it exceeds the edge threshold, assign
// the new face to the same subject immediately. The batch recluster will
// correct any mistakes via transitive graph analysis.
let existing = db::get_all_faces_with_embeddings(pool).await?;
let nearest = existing.iter()
    .filter(|(id, _, _, _)| *id != new_face_id)
    .map(|(id, _, emb_bytes, _)| {
        let emb = bytes_to_f32_vec(emb_bytes).unwrap_or_default();
        (*id, cosine_similarity(&face_emb, &emb))
    })
    .filter(|(_, sim)| *sim > EDGE_THRESHOLD)
    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

if let Some((nearest_face_id, _sim)) = nearest {
    if let Some(subject_id) = db::get_face_subject_id(pool, nearest_face_id).await? {
        db::update_face_subject(pool, new_face_id, Some(subject_id)).await?;
    }
}
```

This gives users immediate subject labels in the UI. The batch graph recluster runs after each batch and:
- Re-assigns any faces that landed in the wrong subject via greedy assignment.
- Merges transitive chains that greedy 1-NN missed.

## Future Work: Full Online Mode

The current design is batch-centric for simplicity. A future iteration can implement true online graph clustering:

1. Maintain the graph and connected-component index in memory across batches.
2. When a new face arrives, add its node + k-NN edges to the existing graph.
3. If the new node bridges two previously separate components, merge them.
4. If a new edge creates a conflict (cannot-link pair in same component), flag it.
5. Assign the new face to its component's subject immediately.

This would eliminate the need for periodic full recluster entirely. It is deferred because:
- It requires an in-memory graph data structure with incremental update support.
- Conflict resolution becomes more complex when components can merge dynamically.
- The batch mode delivers the core quality improvement immediately.

## Edge Cases

| Case | Handling |
|---|---|
| Single isolated face (component size = 1) | Treated as noise — stays unassigned. |
| Component with faces from >1 manual subject | Conflict. Unassigned faces stay unassigned. |
| Component with 0 manual faces, no subject match | Creates new subject if size >= MIN_COMPONENT_SIZE. |
| Subject with >50 manual faces | Sample 50 faces for must-link pair generation. |
| No edges in graph (all faces isolated) | All faces remain unassigned — same as HDBSCAN with no density. |
| Representative face matches multiple subjects equally | Pick the subject with the most faces in the component (if any). Otherwise, create new subject. |

## Testing Strategy

### Unit Tests in `clustering.rs`

1. **Transitive chain**: 3 faces where A-B and B-C edges exist but A-C does not. Assert all 3 in same component.
2. **Must-link edge injection**: Two faces with similarity 0.2 (below threshold) but must-linked. Assert same component.
3. **Cannot-link conflict**: Component contains manual faces from subject 1 and subject 2. Assert unassigned faces stay unassigned.
4. **Single-node noise**: One isolated face. Assert stays unassigned.
5. **Subject match via representative**: Component of 4 unassigned faces near subject 10. Assert assigned to subject 10.
6. **New subject creation**: Component of 3 unassigned faces with no near subject. Assert new subject created.
7. **Merge suggestion**: Two components both match subject 5. Assert merge suggestion inserted.

### Integration Test

- In-memory SQLite DB with 12 faces:
  - 6 faces forming a chain for person A
  - 6 faces forming a chain for person B
  - 1 ambiguous face weakly linking A and B (low similarity)
- Run `cluster_unassigned_faces`.
- Assert 2 subjects created, 1 noise (the ambiguous bridge face).

## Files to Modify

**Backend (Rust):**
- `src-tauri/src/clustering.rs` — Replace HDBSCAN with graph clustering algorithm.
- `src-tauri/src/db.rs` — Add `get_manual_faces_by_subject` query.
- `src-tauri/src/embedder.rs` — Add greedy 1-NN online assignment after `insert_face`.
- `src-tauri/Cargo.toml` — Remove `hdbscan` dependency (no longer used).

## References

- Biemann, C. (2006). *Chinese Whispers: An Efficient Graph Clustering Algorithm and its Application to Natural Language Processing Problems*. HLT-NAACL Workshop.
- Wagstaff, K., Cardie, C., Rogers, S., & Schroedl, S. (2001). *Constrained K-Means Clustering with Background Knowledge*. ICML 2001.
- Schroff, F., Kalenichenko, D., & Philbin, J. (2015). *FaceNet: A Unified Embedding for Face Recognition and Clustering*. CVPR 2015.
- Immich Facial Recognition Documentation (2026). [Facial Recognition | Immich](https://docs.immich.app/docs/features/facial-recognition)

## Safety Properties

- Manual faces are never reassigned by recluster.
- A single user correction (must-link) cannot incorrectly merge two well-separated subjects unless a transitive chain physically exists in the embedding space. If such a chain exists, the cannot-link conflict detector catches it.
- The algorithm degrades gracefully: if embeddings are poor and the graph has no edges, behavior is identical to HDBSCAN with no dense regions (all noise).
