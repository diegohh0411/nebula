# Constraint-Aware Incremental Face Clustering

**Date:** 2026-06-08
**Status:** Approved (brainstorm complete)
**Parent:** TT-47 (clustering evaluation), Track B
**Scope:** replaces the shipped centroid + HDBSCAN clustering with a constraint-aware, incremental graph clusterer, and moves face vectors into sqlite-vec. Alpha app, no users, no backwards-compat constraint — old code is deleted, not evolved.

---

## Why

On-device models (Buffalo-S quantized) mismatch faces; the same person forms **transitive similarity chains** (A≈B, B≈C, A≉C). The shipped clusterer (greedy single-centroid match + HDBSCAN residual, `clustering.rs`) cannot use user feedback and actively fights two of three correction channels (see TT-47). Track A fixes the bugs; Track B replaces the core so feedback is first-class.

**Governing UX principle (from the product owner):** corrections are asymmetric in cost. Merging two subjects the system already suggested is one click; removing a wrongly-grouped face is high-friction. Therefore the algorithm must be **precision-leaning**: prefer leaving a person split into pure duplicates (cheap to merge) over fusing two people (expensive to un-merge). User constraints always dominate heuristics.

---

## Decomposition

- **B1 — Storage & constraint foundation.** sqlite-vec for face vectors; first-class constraint tables; neighbor-query API. Blocks B2.
- **B2 — Constraint-aware incremental graph clustering.** The persisted graph, insert-time assignment, constraint enforcement, action wiring; deletes old clustering. Depends on B1.
- **B3 — Per-provider threshold calibration.** Follow-up; depends on B2, non-blocking.

This document specifies **B1 + B2** (tightly coupled). B3 is a separate lighter spec.

---

## B1 — Storage & constraint foundation

### Vectors: sqlite-vec

- Introduce a sqlite-vec virtual table `face_vectors(rowid = face_id, embedding float[D])` as the **single source of truth** for face embeddings.
- Remove the `faces.embedding` BLOB column and the separate face index snapshot. (Image semantic search and its `FlatIndex`/`nebula.idx` are out of scope — faces only.)
- kNN becomes a SQL query against `face_vectors`. This is the only neighbor-search path the clusterer uses, behind a thin Rust API so the impl can change later.

### Constraints: first-class tables

Replace the overloaded `faces.is_manual` and the write-only `face_corrections` with explicit, symmetric constraints:

```
constraints(
  face_a      INTEGER NOT NULL,   -- always store with face_a < face_b
  face_b      INTEGER NOT NULL,
  kind        TEXT NOT NULL,      -- 'must_link' | 'cannot_link'
  source      TEXT NOT NULL,      -- 'merge' | 'manual_assign' | 'removal' | 'dismiss'
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (face_a, face_b, kind)
)
```

- `is_manual` is retired entirely. A face being "user-labeled / anchored" is now expressed by the constraints and by having a user-sourced subject assignment.
- Dismissed merge suggestions are subject-level ("these two subjects are different people"). Model as `cannot_link` between a representative sample of each subject's faces, `source = 'dismiss'`, so they live in the same table the clusterer already consults. (Representative sampling is acceptable: the clusterer's conflict check only needs *some* cannot-linked pair across the two groups to keep them apart.)

### Neighbor-query API

```rust
/// k nearest neighbors of `face_id` by cosine similarity, descending.
fn knn(face_id: i64, k: usize) -> Vec<(i64, f32)>;
```

Backed by sqlite-vec now; swappable without touching B2.

### B1 acceptance

- Face vectors stored/queried via sqlite-vec; no `faces.embedding` blob, no face `.idx`.
- `constraints` table exists; user actions write to it (wired fully in B2, but the schema + write helpers land here).
- `knn` returns correct neighbors (test against a small seeded set).
- Migration note: alpha, so a one-shot rebuild of vectors/constraints from existing `faces`/correction data is acceptable; no incremental migration needed.

---

## B2 — Constraint-aware incremental graph clustering

### The graph

- **Node** per face.
- **Similarity edges:** for each face, its **mutual** top-k neighbors with cosine ≥ `τ_sim`. *Mutual* (A in B's top-k AND B in A's top-k) is the precision lever — it suppresses accidental "bridge" edges that would otherwise chain two people together.
- **Must-link edges:** injected from `constraints` unconditionally, bypassing `τ_sim`. Never dropped.
- Edges persisted in `face_edges(face_a, face_b, weight)` so incremental updates don't recompute the whole graph.

### Clusters = connected components

Union-Find over similarity + must-link edges. Each component maps to one subject.

### Cannot-link enforcement

Cannot-link is **not** an edge — it is forbidden co-membership:

- If adding similarity edges would place a cannot-linked pair in the same component, drop the **weakest similarity edge** on the connecting path instead of fusing.
- Must-link edges are never dropped. A must-link that *directly* contradicts a cannot-link is a genuine user contradiction → log/flag, do not silently resolve.

### Label assignment (precision-leaning rules)

For a component after edges settle:

1. **One labeled subject present** → auto-assign all unlabeled faces in the component to that subject. Immediate, safe.
2. **Two or more distinct labeled subjects** would join via *similarity* edges → **do not auto-fuse.** A similarity edge whose endpoints already belong to two different labeled subjects is treated as an implicit soft conflict (same machinery as cannot-link): the cross-edge does not merge the components; instead each subject keeps its own faces and a **merge suggestion** is emitted for the pair. (This is the cheap one-click recovery; never the expensive auto-over-merge.) A *must-link* between two labeled subjects is a real user merge and DOES fuse them — only similarity edges are held back.
3. **No labeled subject, size ≥ `MIN_COMPONENT_SIZE`** → create a new (unnamed) subject.
4. **No labeled subject, size < `MIN_COMPONENT_SIZE`** → stays unassigned (noise) until more faces arrive.

**Override — user labeling is immediate and size-exempt:** when the user names/assigns a face that has no subject yet, create its subject *immediately*, even at size 1. `MIN_COMPONENT_SIZE` gates only automatic, unlabeled cluster formation. User intent is never subject to the heuristic.

### Incremental insert flow (primary mode)

New face F embedded:

1. Insert F's vector into `face_vectors`.
2. `knn(F, k)`; keep mutual neighbors with sim ≥ `τ_sim`.
3. Add F's node + similarity edges + any must-link edges from `constraints`.
4. Recompute only F's affected component(s) via Union-Find.
5. Apply the label-assignment rules above; if F bridges two labeled subjects, emit a suggestion rather than fusing.
6. Persist graph delta + subject assignment. F gets a label immediately (or stays noise if isolated).

**Batch recluster = replay** this flow over all faces. Same code path; no separate batch algorithm.

### User actions → constraints (closing the loop)

- **Merge(A, B)** → `must_link` between A's and B's faces (durable: future reclustering can never re-split them — fixes today's revert bug at the root). Reassign faces, delete source subject.
- **Remove face F from subject S** → `cannot_link` between F and S's faces; un-assign F. F may later join a *different* subject but never S.
- **Manual assign / name face F → subject S** → label F with S (creating S immediately if new) + `must_link` F to S's existing faces.
- **Dismiss suggestion(A, B)** → subject-level `cannot_link` (representative sample), `source = 'dismiss'`.

All of these are now **enforced by the clusterer**, not merely shaping centroids or hiding UI.

### Merge suggestions (restated in graph terms)

A suggestion is a **cross-edge**: a similarity edge between faces of two distinct subjects where the pair is not `cannot_link` and not dismissed. Falls out of the graph directly — no separate centroid-cosine pass. (At least one subject named, matching current product behavior.)

### Deleted

`compute_anchor_centroids`, `find_nearest_anchor`, the HDBSCAN residual pass, the `hdbscan` dependency, and the centroid-cosine `find_merge_suggestions`. Replaced wholesale.

### Constants (calibrated in B3; sensible defaults here)

```
τ_sim              = 0.55   // mutual-kNN edge threshold (provider-dependent; B3 tunes)
K_NEAREST          = 5
MIN_COMPONENT_SIZE = 2      // gates AUTOMATIC unlabeled clusters only
```

---

## Testing strategy

**B1**
- `knn` returns correct ordering on a seeded `face_vectors` set.
- Constraint writes are symmetric and de-duplicated (face_a < face_b invariant).

**B2 (unit, deterministic)**
- Transitive chain: A–B, B–C edges (no A–C) → all three one component.
- Mutual-kNN suppresses a one-directional bridge between two groups → groups stay separate.
- Must-link injects an edge below `τ_sim` → faces co-located.
- Cannot-link: a chain that would join two cannot-linked faces → weakest edge dropped, faces separated.
- Auto-assign-one: unlabeled component touching one subject → assigned.
- Suggest-two: new face bridging two named subjects → not fused, suggestion emitted.
- New-subject only at size ≥ MIN; smaller stays noise.
- **User-label override:** naming a lone face creates a subject immediately (size 1).

**B2 (integration, in-memory SQLite + sqlite-vec)**
- Remove face from subject, then recluster → face is NOT back in that subject (the headline TT-47 bug, now structurally impossible).
- Merge two embedding-distant groups, then recluster → still one subject (no re-split).

---

## Out of scope

- Image semantic search / `FlatIndex` / `nebula.idx` (faces only here).
- Per-provider threshold calibration (B3).
- Any backwards-compatible migration (alpha: rebuild from scratch is fine).

## Open items for B3

- Calibrate `τ_sim` and the suggestion threshold per embedding provider (mixed-provider direction).
- Optional: HNSW-backed `knn` if sqlite-vec brute-force kNN gets slow near 200k (decision deferred; interface already abstracts it).
