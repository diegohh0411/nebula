# Constrained k-NN Graph Clustering Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace HDBSCAN with a graph-based clustering algorithm that captures transitive similarity chains and respects user-defined must-link/cannot-link constraints.

**Architecture:** Build an undirected k-NN graph over all face embeddings, inject must-link constraints as edges, compute connected components via Union-Find, then assign components to subjects based on manual-face membership. The batch recluster in `cluster_unassigned_faces` is fully replaced. A greedy 1-NN online assignment is added to `process_subject_one` for immediate UX feedback.

**Tech Stack:** Rust, sqlx (SQLite), no new crate dependencies (graph algorithm implemented with `HashMap`/`HashSet`/`Vec`).

---

## File Structure

| File | Responsibility |
|------|---------------|
| `src-tauri/src/clustering.rs` | **Rewrite.** All graph construction, Union-Find, component-to-subject assignment logic. Unit tests for pure functions. |
| `src-tauri/src/db.rs` | **Add** `get_manual_faces_by_subject` query. Add integration test for it. |
| `src-tauri/src/embedder.rs` | **Modify** `process_subject_one` to add greedy 1-NN online assignment after face insertion. |
| `src-tauri/Cargo.toml` | **Remove** `hdbscan = "0.12"` dependency. |

---

## Task 1: Add `get_manual_faces_by_subject` DB Query

**Files:**
- Modify: `src-tauri/src/db.rs` (after line ~693, near `get_manual_face_embeddings_by_subject`)
- Test: `src-tauri/src/db.rs` (test module at bottom)

This query returns `(subject_id, Vec<face_id>)` for all subjects that have at least one `is_manual = 1` face. Needed to build must-link pairs and check for cannot-link conflicts.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `db.rs` (after the existing `get_manual_face_embeddings_returns_only_manual` test at line 1300):

```rust
#[tokio::test]
async fn get_manual_faces_by_subject_groups_correctly() {
    let pool = make_pool().await;

    // Create two subjects
    let s1: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let s2: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // Subject 1: 2 manual faces, 1 auto face
    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual)
         VALUES (1, ?, 0,0,1,1, X'00000000', 0, 1)",
    )
    .bind(s1)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual)
         VALUES (2, ?, 0,0,1,1, X'00000000', 0, 1)",
    )
    .bind(s1)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual)
         VALUES (3, ?, 0,0,1,1, X'00000000', 0, 0)",
    )
    .bind(s1)
    .execute(&pool)
    .await
    .unwrap();

    // Subject 2: 1 manual face
    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual)
         VALUES (4, ?, 0,0,1,1, X'00000000', 0, 1)",
    )
    .bind(s2)
    .execute(&pool)
    .await
    .unwrap();

    let results = get_manual_faces_by_subject(&pool).await.unwrap();
    // Should have exactly 2 groups
    assert_eq!(results.len(), 2);

    let s1_faces = results.iter().find(|(sid, _)| *sid == s1).unwrap();
    assert_eq!(s1_faces.1.len(), 2, "subject 1 should have 2 manual faces");

    let s2_faces = results.iter().find(|(sid, _)| *sid == s2).unwrap();
    assert_eq!(s2_faces.1.len(), 1, "subject 2 should have 1 manual face");
}

#[tokio::test]
async fn get_manual_faces_by_subject_excludes_subjects_without_manual() {
    let pool = make_pool().await;

    let s1: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // Only auto faces, no manual
    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual)
         VALUES (1, ?, 0,0,1,1, X'00000000', 0, 0)",
    )
    .bind(s1)
    .execute(&pool)
    .await
    .unwrap();

    let results = get_manual_faces_by_subject(&pool).await.unwrap();
    assert!(results.is_empty(), "subjects without manual faces should not appear");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test get_manual_faces_by_subject -- --nocapture`
Expected: FAIL — `get_manual_faces_by_subject` function does not exist yet.

- [ ] **Step 3: Write the implementation**

Add the following function in `db.rs`, right after `get_manual_face_embeddings_by_subject` (after line ~693):

```rust
/// Returns (subject_id, Vec<face_id>) for all subjects that have at least one is_manual=1 face.
/// Used to build must-link pairs and detect cannot-link conflicts.
pub async fn get_manual_faces_by_subject(pool: &SqlitePool) -> Result<Vec<(i64, Vec<i64>)>> {
    let rows = sqlx::query(
        "SELECT subject_id, id FROM faces \
         WHERE subject_id IS NOT NULL AND is_manual = 1 \
         ORDER BY subject_id, id",
    )
    .fetch_all(pool)
    .await?;

    let mut map: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    for row in &rows {
        let subject_id: i64 = row.get("subject_id");
        let face_id: i64 = row.get("id");
        map.entry(subject_id).or_default().push(face_id);
    }

    let mut result: Vec<(i64, Vec<i64>)> = map.into_iter().collect();
    result.sort_by_key(|(sid, _)| *sid);
    Ok(result)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test get_manual_faces_by_subject -- --nocapture`
Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(db): add get_manual_faces_by_subject query for graph clustering constraints"
```

---

## Task 2: Implement Union-Find and Graph Construction (Pure Functions)

**Files:**
- Modify: `src-tauri/src/clustering.rs`

This task creates the core data structures and pure functions that don't touch the DB. We write them all at once with their tests because they form a tightly-coupled unit.

- [ ] **Step 1: Write the failing tests for Union-Find**

Add these tests to the `mod tests` block in `clustering.rs` (after the existing tests at line 307):

```rust
#[test]
fn union_find_single_element() {
    let mut uf = UnionFind::new(3);
    assert_eq!(uf.find(0), 0);
    assert_eq!(uf.find(1), 1);
    assert_eq!(uf.find(2), 2);
}

#[test]
fn union_find_merges_sets() {
    let mut uf = UnionFind::new(4);
    uf.union(0, 1);
    uf.union(1, 2);
    assert_eq!(uf.find(0), uf.find(2), "0 and 2 should be in the same set");
    assert_ne!(uf.find(0), uf.find(3), "3 should be in a different set");
}

#[test]
fn union_find_components_group_correctly() {
    let mut uf = UnionFind::new(5);
    // {0,1,2} and {3,4}
    uf.union(0, 1);
    uf.union(1, 2);
    uf.union(3, 4);

    let components = uf.components();
    assert_eq!(components.len(), 2);
    let mut sets: Vec<Vec<usize>> = components.into_values().collect();
    sets.sort_by_key(|s| s[0]);
    assert_eq!(sets[0], vec![0, 1, 2]);
    assert_eq!(sets[1], vec![3, 4]);
}

#[test]
fn build_knn_graph_transitive_chain() {
    // 3 faces: A-B edge, B-C edge, no A-C edge. All should be in same component.
    let faces: Vec<(usize, Vec<f32>)> = vec![
        (0, emb(&[1.0, 0.0])),   // A
        (1, emb(&[0.9, 0.1])),   // B (near A)
        (2, emb(&[0.8, 0.2])),   // C (near B, not near A)
    ];

    let mut uf = UnionFind::new(3);
    build_knn_graph(&faces, 2, 0.55, &mut uf);

    let components = uf.components();
    assert_eq!(components.len(), 1, "all 3 faces should form one component");
}

#[test]
fn build_knn_graph_isolated_nodes_stay_separate() {
    let faces: Vec<(usize, Vec<f32>)> = vec![
        (0, emb(&[1.0, 0.0])),
        (1, emb(&[0.0, 1.0])),
        (2, emb(&[-1.0, 0.0])),
    ];

    let mut uf = UnionFind::new(3);
    build_knn_graph(&faces, 2, 0.55, &mut uf);

    let components = uf.components();
    assert_eq!(components.len(), 3, "all faces should be isolated");
}

#[test]
fn inject_must_link_edges_bypasses_threshold() {
    // Two faces far apart (sim < 0.55) but must-linked
    let mut uf = UnionFind::new(2);
    inject_must_link_edges(&[(0, 1)], &mut uf);

    assert_eq!(uf.find(0), uf.find(1), "must-link faces should be in same component");
}

#[test]
fn inject_must_link_transitive_closure() {
    // A must-link B, C must-link D, B must-link C → all in same component
    let mut uf = UnionFind::new(4);
    inject_must_link_edges(&[(0, 1), (2, 3), (1, 2)], &mut uf);

    assert_eq!(uf.find(0), uf.find(3), "transitive must-link should connect all");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test union_find -- --nocapture; cargo test build_knn_graph -- --nocapture; cargo test inject_must_link -- --nocapture`
Expected: FAIL — `UnionFind`, `build_knn_graph`, `inject_must_link_edges` not defined yet.

- [ ] **Step 3: Write the implementations**

First, **replace the imports** at the top of `clustering.rs` (lines 1-6). Change:

```rust
use anyhow::Result;
use hdbscan::{Hdbscan, HdbscanHyperParams};
use sqlx::SqlitePool;
use std::collections::HashMap;

use crate::db;
```

To:

```rust
use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};

use crate::db;
use crate::embedder::cosine_similarity;
```

Then **add constants** (replace the old constants at line 103 and 155). Place them right after the imports, before the `cluster_unassigned_faces` function:

```rust
/// Minimum cosine similarity to create an edge between two faces.
const EDGE_THRESHOLD: f32 = 0.55;

/// Number of nearest neighbors to consider per face when building the graph.
const K_NEAREST: usize = 5;

/// Minimum component size to form a new subject.
const MIN_COMPONENT_SIZE: usize = 2;

/// Minimum cosine similarity for a representative face to match an existing subject.
const SUBJECT_MATCH_THRESHOLD: f32 = 0.55;

/// Minimum cosine similarity between subject centroids to generate a merge suggestion.
const MERGE_CENTROID_SIMILARITY_THRESHOLD: f32 = 0.65;
```

Then **add the `UnionFind` struct and methods**. Place this right after the constants, before `cluster_unassigned_faces`:

```rust
/// Union-Find (Disjoint Set Union) with path compression and union by rank.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return;
        }
        if self.rank[rx] < self.rank[ry] {
            self.parent[rx] = ry;
        } else if self.rank[rx] > self.rank[ry] {
            self.parent[ry] = rx;
        } else {
            self.parent[ry] = rx;
            self.rank[rx] += 1;
        }
    }

    /// Returns connected components as {root_index: [member_indices]}.
    fn components(&mut self) -> HashMap<usize, Vec<usize>> {
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..self.parent.len() {
            let root = self.find(i);
            groups.entry(root).or_default().push(i);
        }
        groups
    }
}
```

Then **add the graph construction functions** right after `UnionFind`:

```rust
/// Build k-NN graph edges and merge connected pairs into the Union-Find.
/// `faces` is (local_index, embedding). Only edges with similarity > threshold are added.
fn build_knn_graph(
    faces: &[(usize, Vec<f32>)],
    k: usize,
    threshold: f32,
    uf: &mut UnionFind,
) {
    let n = faces.len();
    if n == 0 {
        return;
    }

    for i in 0..n {
        // Compute similarities from face i to all other faces
        let mut sims: Vec<(usize, f32)> = Vec::with_capacity(n - 1);
        for j in 0..n {
            if i == j {
                continue;
            }
            let sim = cosine_similarity(&faces[i].1, &faces[j].1);
            if sim > threshold {
                sims.push((j, sim));
            }
        }

        // Sort by similarity descending, take top-k
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (j, _sim) in sims.into_iter().take(k) {
            uf.union(i, j);
        }
    }
}

/// Inject must-link constraint edges into the Union-Find.
/// Each pair (i, j) is unconditionally merged regardless of similarity.
fn inject_must_link_edges(pairs: &[(usize, usize)], uf: &mut UnionFind) {
    for &(i, j) in pairs {
        uf.union(i, j);
    }
}
```

**Delete** the old `ANCHOR_MATCH_THRESHOLD` constant (line 155), the old `compute_anchor_centroids` function (lines 157-198), and the old `find_nearest_anchor` function (lines 200-211). These are no longer used — the graph algorithm replaces them.

Keep the `ReclusterResult` struct (lines 147-153) but move it to after the constants (it is still used).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test union_find -- --nocapture; cargo test build_knn_graph -- --nocapture; cargo test inject_must_link -- --nocapture`
Expected: ALL PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(clustering): add Union-Find, k-NN graph builder, and must-link injection"
```

---

## Task 3: Rewrite `cluster_unassigned_faces` with Graph Clustering

**Files:**
- Modify: `src-tauri/src/clustering.rs`

This is the core algorithm replacement. The function signature and return type stay the same.

- [ ] **Step 1: Write the failing tests for the assignment logic**

Add these tests to the `mod tests` block in `clustering.rs`:

```rust
#[test]
fn assign_component_single_manual_subject() {
    // Component has faces from subject 10 (manual). All unassigned faces go to subject 10.
    let face_subjects: Vec<Option<i64>> = vec![Some(10), Some(10), None, None];
    let is_manual: Vec<bool> = vec![true, false, false, false];
    let manual_by_subject: HashMap<i64, Vec<i64>> = {
        let mut m = HashMap::new();
        m.insert(10, vec![0]);
        m
    };

    let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
    assert_eq!(result, Some(ComponentAssignment::AssignExisting(10)));
}

#[test]
fn assign_component_conflict_multiple_manual_subjects() {
    // Component has manual faces from subject 10 AND subject 20 → conflict
    let face_subjects: Vec<Option<i64>> = vec![Some(10), Some(20), None];
    let is_manual: Vec<bool> = vec![true, true, false];
    let manual_by_subject: HashMap<i64, Vec<i64>> = {
        let mut m = HashMap::new();
        m.insert(10, vec![0]);
        m.insert(20, vec![1]);
        m
    };

    let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
    assert_eq!(result, None, "conflict should return None (noise)");
}

#[test]
fn assign_component_no_manual_new_subject() {
    // Component of 3 unassigned faces with no manual faces → new subject (size >= MIN)
    let face_subjects: Vec<Option<i64>> = vec![None, None, None];
    let is_manual: Vec<bool> = vec![false, false, false];
    let manual_by_subject: HashMap<i64, Vec<i64>> = HashMap::new();

    let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
    assert_eq!(result, Some(ComponentAssignment::CreateNew));
}

#[test]
fn assign_component_no_manual_too_small() {
    // Component of 1 unassigned face → noise
    let face_subjects: Vec<Option<i64>> = vec![None];
    let is_manual: Vec<bool> = vec![false];
    let manual_by_subject: HashMap<i64, Vec<i64>> = HashMap::new();

    let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
    assert_eq!(result, None, "single unassigned face should be noise");
}

#[test]
fn find_highest_degree_node_picks_hub() {
    // Node 0 connected to 1,2 (degree 2). Node 1 connected to 0 (degree 1). Node 2 connected to 0 (degree 1).
    let faces: Vec<(usize, Vec<f32>)> = vec![
        (0, emb(&[1.0, 0.0])),   // hub
        (1, emb(&[0.9, 0.1])),   // near 0
        (2, emb(&[0.95, 0.05])), // near 0
        (3, emb(&[0.0, 1.0])),   // far from all
    ];

    let mut uf = UnionFind::new(4);
    uf.union(0, 1);
    uf.union(0, 2);

    let adj = build_adjacency(&faces, K_NEAREST, EDGE_THRESHOLD);
    let component_indices = vec![0, 1, 2]; // faces 0,1,2 in same component

    let hub = find_highest_degree_node(&component_indices, &adj);
    assert_eq!(hub, 0, "node 0 should be the hub");
}

#[test]
fn find_best_subject_match_above_threshold() {
    // Representative embedding near subject 10's centroid
    let rep_emb = emb(&[1.0, 0.0]);
    let subject_centroids: HashMap<i64, Vec<f32>> = {
        let mut m = HashMap::new();
        m.insert(10, emb(&[0.95, 0.05]));
        m.insert(20, emb(&[0.0, 1.0]));
        m
    };

    let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
    assert_eq!(result, Some(10));
}

#[test]
fn find_best_subject_match_below_threshold() {
    let rep_emb = emb(&[1.0, 0.0]);
    let subject_centroids: HashMap<i64, Vec<f32>> = {
        let mut m = HashMap::new();
        m.insert(20, emb(&[0.0, 1.0])); // orthogonal, sim ~0
        m
    };

    let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
    assert_eq!(result, None, "should not match when similarity is below threshold");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test assign_component -- --nocapture; cargo test find_highest_degree -- --nocapture; cargo test find_best_subject -- --nocapture`
Expected: FAIL — types and functions not defined yet.

- [ ] **Step 3: Write the helper types and functions**

Add these after the `inject_must_link_edges` function, before the `cluster_unassigned_faces` function:

```rust
/// Result of deciding what to do with a connected component.
#[derive(Debug, PartialEq)]
enum ComponentAssignment {
    /// All unassigned faces in this component should be assigned to this existing subject.
    AssignExisting(i64),
    /// This component represents a new subject — create one.
    CreateNew,
}

/// Decide what to do with a connected component based on its manual-face composition.
/// `face_subjects[i]` = current subject_id of face i (None if unassigned).
/// `is_manual[i]` = whether face i is a manual face.
/// `manual_by_subject` = map from subject_id to list of manual face indices (global).
fn assign_component(
    face_subjects: &[Option<i64>],
    is_manual: &[bool],
    _manual_by_subject: &HashMap<i64, Vec<i64>>,
) -> Option<ComponentAssignment> {
    // Collect distinct subjects that have manual faces in this component
    let mut manual_subjects: HashSet<i64> = HashSet::new();
    for (i, &manual) in is_manual.iter().enumerate() {
        if manual {
            if let Some(sid) = face_subjects[i] {
                manual_subjects.insert(sid);
            }
        }
    }

    match manual_subjects.len() {
        0 => {
            // New candidate — check size
            let component_size: usize = face_subjects.iter().filter(|s| s.is_none()).count();
            if component_size < MIN_COMPONENT_SIZE {
                None // noise
            } else {
                Some(ComponentAssignment::CreateNew)
            }
        }
        1 => {
            // Exactly one manual subject — assign all unassigned to it
            Some(ComponentAssignment::AssignExisting(
                *manual_subjects.iter().next().unwrap(),
            ))
        }
        _ => {
            // Conflict: multiple manual subjects in same component
            eprintln!(
                "[clustering] Conflict: component contains manual faces from {} subjects ({:?}). Faces stay unassigned.",
                manual_subjects.len(),
                manual_subjects,
            );
            None
        }
    }
}

/// Build an adjacency list from k-NN graph edges (for degree computation).
/// Returns adjacency as vec of sets, indexed by local face index.
fn build_adjacency(
    faces: &[(usize, Vec<f32>)],
    k: usize,
    threshold: f32,
) -> Vec<HashSet<usize>> {
    let n = faces.len();
    let mut adj: Vec<HashSet<usize>> = vec![HashSet::new(); n];

    for i in 0..n {
        let mut sims: Vec<(usize, f32)> = Vec::with_capacity(n - 1);
        for j in 0..n {
            if i == j {
                continue;
            }
            let sim = cosine_similarity(&faces[i].1, &faces[j].1);
            if sim > threshold {
                sims.push((j, sim));
            }
        }
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (j, _sim) in sims.into_iter().take(k) {
            adj[i].insert(j);
            adj[j].insert(i);
        }
    }

    adj
}

/// Find the face with the highest degree in a component (most edges = natural hub).
fn find_highest_degree_node(component_indices: &[usize], adj: &[HashSet<usize>]) -> usize {
    *component_indices
        .iter()
        .max_by_key(|&&idx| adj[idx].len())
        .unwrap()
}

/// Find the best matching subject for a representative embedding.
/// Returns the subject_id with highest cosine similarity above the threshold, or None.
fn find_best_subject_match(
    rep_emb: &[f32],
    subject_centroids: &HashMap<i64, Vec<f32>>,
    threshold: f32,
) -> Option<i64> {
    subject_centroids
        .iter()
        .map(|(&sid, centroid)| (sid, cosine_similarity(rep_emb, centroid)))
        .filter(|(_, sim)| *sim > threshold)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(sid, _)| sid)
}
```

- [ ] **Step 4: Run the new tests**

Run: `cd src-tauri && cargo test assign_component -- --nocapture; cargo test find_highest_degree -- --nocapture; cargo test find_best_subject -- --nocapture`
Expected: ALL PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(clustering): add component assignment logic, adjacency builder, and subject matching"
```

---

## Task 4: Rewrite `cluster_unassigned_faces` Function Body

**Files:**
- Modify: `src-tauri/src/clustering.rs`

Now we replace the old function body with the new graph-clustering pipeline.

- [ ] **Step 1: Replace the `cluster_unassigned_faces` function body**

Replace the entire function body (lines 8-101) with:

```rust
pub async fn cluster_unassigned_faces(pool: &SqlitePool) -> Result<ReclusterResult> {
    // 1. Load ALL faces with embeddings (assigned + unassigned)
    let all_faces_raw = db::get_all_faces_with_embeddings(pool).await?;

    // Decode embeddings
    let mut all_faces: Vec<(i64, Option<i64>, Vec<f32>, bool)> = Vec::new();
    for (face_id, subject_id, emb_blob, is_manual) in all_faces_raw {
        match crate::embedder::bytes_to_f32_vec(&emb_blob) {
            Ok(emb) => all_faces.push((face_id, subject_id, emb, is_manual)),
            Err(_) => eprintln!("[clustering] Failed to decode embedding for face {}", face_id),
        }
    }

    if all_faces.is_empty() {
        return Ok(ReclusterResult {
            clusters: 0,
            noise: 0,
            merged: 0,
            deleted: 0,
        });
    }

    let n = all_faces.len();

    // 2. Load manual face groups by subject for must-link and conflict detection
    let manual_groups_raw = db::get_manual_faces_by_subject(pool).await?;
    let manual_face_to_subject: HashMap<i64, i64> = manual_groups_raw
        .iter()
        .flat_map(|(sid, face_ids)| face_ids.iter().map(|&fid| (fid, *sid)))
        .collect();
    let manual_by_subject: HashMap<i64, Vec<i64>> = manual_groups_raw
        .into_iter()
        .map(|(sid, fids)| (sid, fids))
        .collect();

    // Build index: face_id -> local index
    let face_id_to_idx: HashMap<i64, usize> = all_faces
        .iter()
        .enumerate()
        .map(|(idx, (fid, _, _, _))| (*fid, idx))
        .collect();

    // 3. Build graph faces list for k-NN
    let graph_faces: Vec<(usize, Vec<f32>)> = all_faces
        .iter()
        .enumerate()
        .map(|(idx, (_, _, emb, _))| (idx, emb.clone()))
        .collect();

    // 4. Build Union-Find and k-NN graph
    let mut uf = UnionFind::new(n);
    let effective_k = K_NEAREST.min(n.saturating_sub(1));
    build_knn_graph(&graph_faces, effective_k, EDGE_THRESHOLD, &mut uf);

    // 5. Inject must-link edges
    let mut must_link_pairs: Vec<(usize, usize)> = Vec::new();

    // 5a. is_manual faces within the same subject are must-linked
    for (subject_id, manual_face_ids) in &manual_by_subject {
        let sampled: Vec<i64> = if manual_face_ids.len() > 50 {
            // Sample 50 faces uniformly to keep pair generation bounded
            let step = manual_face_ids.len() as f64 / 50.0;
            (0..50)
                .map(|i| manual_face_ids[(i as f64 * step) as usize])
                .collect()
        } else {
            manual_face_ids.clone()
        };

        // All pairwise combinations
        for i in 0..sampled.len() {
            for j in (i + 1)..sampled.len() {
                if let (Some(&idx_i), Some(&idx_j)) =
                    (face_id_to_idx.get(&sampled[i]), face_id_to_idx.get(&sampled[j]))
                {
                    must_link_pairs.push((idx_i, idx_j));
                }
            }
        }
    }

    inject_must_link_edges(&must_link_pairs, &mut uf);

    // 6. Compute connected components
    let components = uf.components();

    // 7. Compute subject centroids for matching new components to existing subjects
    let mut subject_emb_groups: HashMap<i64, Vec<&Vec<f32>>> = HashMap::new();
    for (_, subject_id, emb, _) in &all_faces {
        if let Some(sid) = subject_id {
            subject_emb_groups.entry(*sid).or_default().push(emb);
        }
    }
    let subject_centroids: HashMap<i64, Vec<f32>> = subject_emb_groups
        .iter()
        .map(|(&sid, embs)| {
            if embs.is_empty() {
                return (sid, vec![]);
            }
            let dim = embs[0].len();
            let mut centroid = vec![0.0f32; dim];
            for emb in embs {
                for (i, &v) in emb.iter().enumerate() {
                    centroid[i] += v;
                }
            }
            let count = embs.len() as f32;
            for v in &mut centroid {
                *v /= count;
            }
            (sid, centroid)
        })
        .filter(|(_, c)| !c.is_empty())
        .collect();

    // Build adjacency for degree computation
    let adj = build_adjacency(&graph_faces, effective_k, EDGE_THRESHOLD);

    // 8. Process each component
    let mut new_clusters_count = 0;
    let mut noise_count = 0;

    for (_root, member_indices) in &components {
        // Collect face metadata for this component
        let comp_face_subjects: Vec<Option<i64>> = member_indices
            .iter()
            .map(|&idx| all_faces[idx].1)
            .collect();
        let comp_is_manual: Vec<bool> = member_indices
            .iter()
            .map(|&idx| all_faces[idx].3)
            .collect();

        let assignment = assign_component(&comp_face_subjects, &comp_is_manual, &manual_by_subject);

        match assignment {
            Some(ComponentAssignment::AssignExisting(subject_id)) => {
                // Assign all unassigned faces in this component to the subject
                for &idx in member_indices {
                    if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                        db::update_face_subject(pool, all_faces[idx].0, Some(subject_id)).await?;
                    }
                }
            }
            Some(ComponentAssignment::CreateNew) => {
                // Try to match to an existing subject via representative embedding
                let hub_idx = find_highest_degree_node(member_indices, &adj);
                let rep_emb = &all_faces[hub_idx].2;

                let subject_match = find_best_subject_match(rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);

                if let Some(existing_sid) = subject_match {
                    // Assign to existing subject
                    for &idx in member_indices {
                        if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                            db::update_face_subject(pool, all_faces[idx].0, Some(existing_sid))
                                .await?;
                        }
                    }
                } else {
                    // Create new subject
                    new_clusters_count += 1;
                    let new_sid = db::insert_subject(pool, None, "person").await?;
                    for &idx in member_indices {
                        if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                            db::update_face_subject(pool, all_faces[idx].0, Some(new_sid)).await?;
                        }
                    }
                }
            }
            None => {
                // Noise or conflict — count unassigned faces as noise
                for &idx in member_indices {
                    if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                        noise_count += 1;
                    }
                }
            }
        }
    }

    // 9. Post-processing
    let deleted = db::delete_subjects_with_no_faces(pool).await?;
    let _ = db::auto_assign_missing_thumbnails(pool).await;
    let _ = find_merge_suggestions(pool).await;

    Ok(ReclusterResult {
        clusters: new_clusters_count,
        noise: noise_count,
        merged: 0,
        deleted,
    })
}
```

**Also delete** the old `compute_anchor_centroids` function, `find_nearest_anchor` function, and `ANCHOR_MATCH_THRESHOLD` constant if they still remain. The new function does not use them.

- [ ] **Step 2: Update `find_merge_suggestions` to use `cosine_similarity` import**

The existing `find_merge_suggestions` function uses `crate::embedder::cosine_similarity`. Since we now import `cosine_similarity` at the top of the file, update the calls inside `find_merge_suggestions` to use the short form. Change `crate::embedder::cosine_similarity(emb_a, emb_b)` to `cosine_similarity(emb_a, emb_b)`.

Also, `find_merge_suggestions` still uses `compute_anchor_centroids`. We need to keep a simpler version. Replace the body of `find_merge_suggestions` to compute centroids inline:

```rust
pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    crate::db::clear_merge_suggestions(pool).await?;

    // Load all faces grouped by subject and compute centroids
    let all_raw = db::get_subject_embeddings(pool).await?;
    let mut groups: HashMap<i64, Vec<Vec<f32>>> = HashMap::new();
    for (sid, blob) in all_raw {
        if let Ok(emb) = crate::embedder::bytes_to_f32_vec(&blob) {
            groups.entry(sid).or_default().push(emb);
        }
    }

    let centroids: HashMap<i64, Vec<f32>> = groups
        .into_iter()
        .filter_map(|(sid, embs)| {
            if embs.is_empty() {
                return None;
            }
            let dim = embs[0].len();
            let mut centroid = vec![0.0f32; dim];
            for emb in &embs {
                for (i, &v) in emb.iter().enumerate() {
                    centroid[i] += v;
                }
            }
            let n = embs.len() as f32;
            for v in &mut centroid {
                *v /= n;
            }
            Some((sid, centroid))
        })
        .collect();

    let mut subject_list: Vec<(i64, Vec<f32>)> = centroids.into_iter().collect();
    subject_list.sort_unstable_by_key(|(id, _)| *id);

    for i in 0..subject_list.len() {
        for j in (i + 1)..subject_list.len() {
            let (id_a, emb_a) = &subject_list[i];
            let (id_b, emb_b) = &subject_list[j];
            let sim = cosine_similarity(emb_a, emb_b);
            if sim > MERGE_CENTROID_SIMILARITY_THRESHOLD {
                crate::db::insert_merge_suggestion(pool, *id_a, *id_b, sim as f64).await?;
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 3: Update the old tests**

The old tests reference `compute_anchor_centroids`, `find_nearest_anchor`, and `ANCHOR_MATCH_THRESHOLD` which we deleted. **Delete all the old tests** in the `mod tests` block (the 6 tests from lines 221-307 about anchor centroids and nearest anchor). They test functions that no longer exist. Keep the `emb` helper function and all the new tests from Tasks 2 and 3.

- [ ] **Step 4: Remove the `hdbscan` import and old `ANCHOR_MATCH_THRESHOLD`**

Verify the top of the file now reads:
```rust
use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};

use crate::db;
use crate::embedder::cosine_similarity;
```

And that there is no `use hdbscan` line, no `ANCHOR_MATCH_THRESHOLD` constant, no `compute_anchor_centroids` function, and no `find_nearest_anchor` function remaining.

- [ ] **Step 5: Run all clustering tests**

Run: `cd src-tauri && cargo test --lib clustering -- --nocapture`
Expected: ALL PASS (no compilation errors, all new tests pass).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(clustering): replace HDBSCAN with constrained k-NN graph clustering"
```

---

## Task 5: Remove `hdbscan` Dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Remove the `hdbscan` line from Cargo.toml**

In `src-tauri/Cargo.toml`, remove the line:
```
hdbscan = "0.12"
```

- [ ] **Step 2: Verify the project compiles**

Run: `cd src-tauri && cargo check`
Expected: Compiles successfully with no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml
git commit -m "chore: remove hdbscan dependency (replaced by graph clustering)"
```

---

## Task 6: Add Greedy 1-NN Online Assignment in `embedder.rs`

**Files:**
- Modify: `src-tauri/src/embedder.rs`

After each face is inserted in `process_subject_one`, immediately try to assign it to the nearest existing subject face.

- [ ] **Step 1: Write the test**

Add a test module at the bottom of `embedder.rs` (this file currently has no tests):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn bytes_roundtrip() {
        let original = vec![1.0, -0.5, 0.3, 0.0];
        let bytes = f32_slice_to_bytes(&original);
        let decoded = bytes_to_f32_vec(&bytes).unwrap();
        assert_eq!(original, decoded);
    }
}
```

- [ ] **Step 2: Run test to verify**

Run: `cd src-tauri && cargo test --lib embedder -- --nocapture`
Expected: PASS (these test existing functions that already work).

- [ ] **Step 3: Add greedy 1-NN assignment after face insertion**

In `process_subject_one`, after the `db::insert_face` call (around line 178), add the greedy assignment. Replace the face insertion block:

```rust
                    for (bbox, face_emb) in faces {
                        let face_blob = f32_slice_to_bytes(&face_emb);
                        let _ = db::insert_face(
                            pool,
                            image_id,
                            None,
                            (
                                bbox.x1 as f64,
                                bbox.y1 as f64,
                                (bbox.x2 - bbox.x1) as f64,
                                (bbox.y2 - bbox.y1) as f64,
                            ),
                            Some(&face_blob),
                        ).await;
                    }
```

With:

```rust
                    for (bbox, face_emb) in faces {
                        let face_blob = f32_slice_to_bytes(&face_emb);

                        // Greedy 1-NN assignment: assign immediately to nearest existing face's subject.
                        // The batch graph recluster will correct any mistakes.
                        let existing = match db::get_all_faces_with_embeddings(pool).await {
                            Ok(f) => f,
                            Err(_) => vec![],
                        };

                        let new_face_id = match db::insert_face(
                            pool,
                            image_id,
                            None,
                            (
                                bbox.x1 as f64,
                                bbox.y1 as f64,
                                (bbox.x2 - bbox.x1) as f64,
                                (bbox.y2 - bbox.y1) as f64,
                            ),
                            Some(&face_blob),
                        ).await {
                            Ok(id) => id,
                            Err(e) => {
                                eprintln!("[embedder] Failed to insert face for image {}: {}", image_id, e);
                                continue;
                            }
                        };

                        // Find nearest existing face by cosine similarity
                        let nearest = existing
                            .iter()
                            .filter(|(id, _, _, _)| *id != new_face_id)
                            .filter_map(|(id, _, emb_bytes, _)| {
                                let emb = bytes_to_f32_vec(emb_bytes).ok()?;
                                let sim = cosine_similarity(&face_emb, &emb);
                                if sim > 0.55 {
                                    Some((id, sim))
                                } else {
                                    None
                                }
                            })
                            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                        if let Some((nearest_face_id, _sim)) = nearest {
                            if let Some(subject_id) = db::get_face_subject_id(pool, nearest_face_id).await.unwrap_or(None) {
                                let _ = db::update_face_subject(pool, new_face_id, Some(subject_id)).await;
                            }
                        }
                    }
```

- [ ] **Step 4: Verify the project compiles**

Run: `cd src-tauri && cargo check`
Expected: Compiles successfully.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/embedder.rs
git commit -m "feat(embedder): add greedy 1-NN online face-to-subject assignment"
```

---

## Task 7: Add Integration Test for Full Pipeline

**Files:**
- Modify: `src-tauri/src/db.rs` (test module)

This test verifies the full graph clustering pipeline with an in-memory SQLite database.

- [ ] **Step 1: Write the integration test**

Add to `db.rs` `mod tests` block:

```rust
#[tokio::test]
async fn get_manual_faces_by_subject_empty_db() {
    let pool = make_pool().await;
    let results = get_manual_faces_by_subject(&pool).await.unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn get_manual_faces_by_subject_multiple_manual_per_subject() {
    let pool = make_pool().await;

    let s1: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // Insert 5 manual faces for subject 1
    for i in 0..5 {
        sqlx::query(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual)
             VALUES (?, ?, 0,0,1,1, X'00000000', 0, 1)",
        )
        .bind(i + 1)
        .bind(s1)
        .execute(&pool)
        .await
        .unwrap();
    }

    let results = get_manual_faces_by_subject(&pool).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, s1);
    assert_eq!(results[0].1.len(), 5);
}
```

- [ ] **Step 2: Run all db tests**

Run: `cd src-tauri && cargo test --lib db -- --nocapture`
Expected: ALL PASS (old test + 4 new tests).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "test(db): add integration tests for get_manual_faces_by_subject"
```

---

## Task 8: Add Graph Clustering Unit Tests for Spec Scenarios

**Files:**
- Modify: `src-tauri/src/clustering.rs`

These test the specific scenarios listed in the spec's Testing Strategy section.

- [ ] **Step 1: Write the spec scenario tests**

Add to the `mod tests` block in `clustering.rs`:

```rust
#[test]
fn transitive_chain_a_b_c() {
    // A-B edge, B-C edge, no A-C edge. All 3 in same component.
    let faces: Vec<(usize, Vec<f32>)> = vec![
        (0, emb(&[1.0, 0.0, 0.0])),
        (1, emb(&[0.95, 0.05, 0.0])),  // near face 0
        (2, emb(&[0.90, 0.10, 0.0])),  // near face 1, not near face 0
    ];

    let mut uf = UnionFind::new(3);
    build_knn_graph(&faces, 2, 0.55, &mut uf);

    let components = uf.components();
    assert_eq!(components.len(), 1, "transitive chain should produce one component");

    let mut members: Vec<usize> = components.values().flatten().copied().collect();
    members.sort();
    assert_eq!(members, vec![0, 1, 2]);
}

#[test]
fn must_link_edge_below_threshold() {
    // Two faces with cosine similarity ~0.0 but must-linked
    let mut uf = UnionFind::new(2);
    inject_must_link_edges(&[(0, 1)], &mut uf);
    assert_eq!(uf.find(0), uf.find(1));
}

#[test]
fn cannot_link_conflict_prevents_assignment() {
    // Component has manual faces from subject 10 and subject 20
    let face_subjects: Vec<Option<i64>> = vec![Some(10), Some(20), None];
    let is_manual: Vec<bool> = vec![true, true, false];
    let manual_by_subject: HashMap<i64, Vec<i64>> = {
        let mut m = HashMap::new();
        m.insert(10, vec![0]);
        m.insert(20, vec![1]);
        m
    };

    let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
    assert_eq!(result, None, "conflict component should stay unassigned");
}

#[test]
fn single_node_noise() {
    let face_subjects: Vec<Option<i64>> = vec![None];
    let is_manual: Vec<bool> = vec![false];
    let manual_by_subject: HashMap<i64, Vec<i64>> = HashMap::new();

    let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
    assert_eq!(result, None, "single unassigned face should be noise");
}

#[test]
fn subject_match_via_representative() {
    // Component of 4 unassigned faces near subject 10's centroid
    let rep_emb = emb(&[0.95, 0.05, 0.0]);
    let subject_centroids: HashMap<i64, Vec<f32>> = {
        let mut m = HashMap::new();
        m.insert(10, emb(&[1.0, 0.0, 0.0]));
        m.insert(20, emb(&[0.0, 1.0, 0.0]));
        m
    };

    let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
    assert_eq!(result, Some(10), "should match subject 10 via representative");
}

#[test]
fn new_subject_creation_no_near_subject() {
    // Component of 3 unassigned faces with no nearby subject
    let rep_emb = emb(&[0.0, 0.0, 1.0]); // orthogonal to all subjects
    let subject_centroids: HashMap<i64, Vec<f32>> = {
        let mut m = HashMap::new();
        m.insert(10, emb(&[1.0, 0.0, 0.0]));
        m
    };

    let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
    assert_eq!(result, None, "should not match any subject");
}
```

Note: Use `.collect()` then `.sort()` as shown above (not `.sorted()` which requires nightly).

- [ ] **Step 2: Run all clustering tests**

Run: `cd src-tauri && cargo test --lib clustering -- --nocapture`
Expected: ALL PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "test(clustering): add spec scenario tests for transitive chains, conflicts, and matching"
```

---

## Task 9: Final Build Verification

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cd src-tauri && cargo test -- --nocapture`
Expected: ALL PASS, no compilation warnings about unused imports.

- [ ] **Step 2: Run cargo check for warnings**

Run: `cd src-tauri && cargo check 2>&1`
Expected: No errors. Check for any unused import warnings and fix them.

- [ ] **Step 3: Verify the binary builds**

Run: `cd src-tauri && cargo build`
Expected: Builds successfully.
