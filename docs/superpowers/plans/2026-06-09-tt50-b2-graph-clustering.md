# TT-50: B2 Constraint-Aware Incremental Graph Clustering

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the HDBSCAN/centroid clusterer in `clustering.rs` with a mutual-kNN connected-components clusterer that natively enforces must-link / cannot-link constraints and emits merge suggestions directly from graph cross-edges.

**Architecture:** Persist similarity edges in a new `face_edges` table; run Kruskal-style Union-Find (strongest edges first, skip any that would violate cannot-link); apply precision-leaning label rules per component; wire user actions (merge/remove/assign/dismiss) to write durable constraints so future reclusters never undo user intent.

**Tech Stack:** Rust · sqlx · sqlite-vec · Union-Find (pure in-memory, no external crate) · `#[tokio::test]` for integration tests · `#[test]` for pure-function unit tests

---

## File Map

| File | Action | What changes |
|---|---|---|
| `src-tauri/src/db.rs` | Modify | Migration 6 (`face_edges`); new helpers; constraint writes on merge/remove/assign/dismiss |
| `src-tauri/src/face_store.rs` | Modify | Add `knn_cosine_sim` + `l2_dist_to_cosine_sim` |
| `src-tauri/src/clustering.rs` | Rewrite | UnionFind; pure graph functions; new `cluster_unassigned_faces`; new `find_merge_suggestions` |
| `src-tauri/src/commands.rs` | Modify | Wire merge/unassign/assign/dismiss to write constraints |
| `src-tauri/Cargo.toml` | Modify | Remove `hdbscan` dependency |

---

## Constants (used across tasks — define once in `clustering.rs`)

```rust
pub const TAU_SIM: f32 = 0.55;
pub const K_NEAREST: usize = 5;
pub const MIN_COMPONENT_SIZE: usize = 2;
```

---

### Task 1: Migration 6 — `face_edges` table + DB helpers

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add migration 6 to `VERSIONED_MIGRATIONS` in `db.rs`**

In `db.rs`, find `const VERSIONED_MIGRATIONS` and append at the end:

```rust
(6, "
    CREATE TABLE IF NOT EXISTS face_edges (
        face_a  INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
        face_b  INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
        weight  REAL NOT NULL,
        PRIMARY KEY (face_a, face_b)
    )
"),
```

- [ ] **Step 2: Write the migration test**

Add to `db.rs` `#[cfg(test)] mod tests { ... }`:

```rust
#[tokio::test]
async fn migration_6_creates_face_edges_table() {
    let pool = init_test_pool().await;
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        tables.contains(&"face_edges".to_string()),
        "face_edges table must exist after migration 6"
    );
    let cols: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('face_edges')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(cols.contains(&"face_a".to_string()));
    assert!(cols.contains(&"face_b".to_string()));
    assert!(cols.contains(&"weight".to_string()));
}
```

- [ ] **Step 3: Run test — expect PASS** (migration runs on `init_test_pool`)

```bash
cd src-tauri && cargo test migration_6_creates_face_edges_table 2>&1 | tail -15
```

Expected: `test db::tests::migration_6_creates_face_edges_table ... ok`

- [ ] **Step 4: Add `face_edges` DB helpers to `db.rs`**

Add these functions after `add_cannot_link`:

```rust
pub async fn upsert_face_edge(pool: &SqlitePool, face_a: i64, face_b: i64, weight: f32) -> Result<()> {
    let (a, b) = if face_a < face_b { (face_a, face_b) } else { (face_b, face_a) };
    sqlx::query(
        "INSERT OR REPLACE INTO face_edges (face_a, face_b, weight) VALUES (?, ?, ?)",
    )
    .bind(a)
    .bind(b)
    .bind(weight)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_all_face_edges(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM face_edges").execute(pool).await?;
    Ok(())
}

pub async fn get_all_similarity_edges(pool: &SqlitePool) -> Result<Vec<(i64, i64, f32)>> {
    let rows = sqlx::query("SELECT face_a, face_b, weight FROM face_edges")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.get("face_a"), r.get("face_b"), r.get::<f32, _>("weight"))).collect())
}

pub async fn get_all_must_link_pairs(pool: &SqlitePool) -> Result<Vec<(i64, i64)>> {
    let rows = sqlx::query("SELECT face_a, face_b FROM constraints WHERE kind = 'must_link'")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.get("face_a"), r.get("face_b"))).collect())
}

pub async fn get_all_cannot_link_pairs(pool: &SqlitePool) -> Result<std::collections::HashSet<(i64, i64)>> {
    let rows = sqlx::query("SELECT face_a, face_b FROM constraints WHERE kind = 'cannot_link'")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| {
        let a: i64 = r.get("face_a");
        let b: i64 = r.get("face_b");
        if a < b { (a, b) } else { (b, a) }
    }).collect())
}

pub async fn get_assigned_face_subject_map(pool: &SqlitePool) -> Result<std::collections::HashMap<i64, i64>> {
    let rows = sqlx::query("SELECT id, subject_id FROM faces WHERE subject_id IS NOT NULL")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.get::<i64, _>("id"), r.get::<i64, _>("subject_id"))).collect())
}

pub async fn get_face_ids_for_subject(pool: &SqlitePool, subject_id: i64) -> Result<Vec<i64>> {
    let rows = sqlx::query("SELECT id FROM faces WHERE subject_id = ?")
        .bind(subject_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.get::<i64, _>("id")).collect())
}

pub async fn get_all_face_ids_with_vectors(pool: &SqlitePool) -> Result<Vec<i64>> {
    let rows = sqlx::query("SELECT rowid FROM face_vectors")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.get::<i64, _>("rowid")).collect())
}
```

- [ ] **Step 5: Write tests for face_edges helpers**

Add to `db.rs` tests:

```rust
#[tokio::test]
async fn upsert_face_edge_normalizes_order_and_deduplicates() {
    let pool = init_test_pool().await;
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1,1,0,0,1,1,0),(2,1,0,0,1,1,0)")
        .execute(&pool).await.unwrap();

    upsert_face_edge(&pool, 2, 1, 0.8).await.unwrap();  // reversed order
    upsert_face_edge(&pool, 1, 2, 0.9).await.unwrap();  // should replace

    let edges = get_all_similarity_edges(&pool).await.unwrap();
    assert_eq!(edges.len(), 1, "duplicate upsert must replace");
    assert_eq!(edges[0].0, 1, "face_a must be smaller id");
    assert_eq!(edges[0].1, 2, "face_b must be larger id");
    assert!((edges[0].2 - 0.9).abs() < 1e-6, "latest weight must win");
}

#[tokio::test]
async fn clear_all_face_edges_removes_all_rows() {
    let pool = init_test_pool().await;
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1,1,0,0,1,1,0),(2,1,0,0,1,1,0)")
        .execute(&pool).await.unwrap();
    upsert_face_edge(&pool, 1, 2, 0.7).await.unwrap();
    clear_all_face_edges(&pool).await.unwrap();
    let edges = get_all_similarity_edges(&pool).await.unwrap();
    assert!(edges.is_empty());
}
```

- [ ] **Step 6: Run tests — expect PASS**

```bash
cd src-tauri && cargo test upsert_face_edge clear_all_face_edges 2>&1 | tail -15
```

Expected: both tests `ok`

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(TT-50): migration 6 face_edges table + DB helpers"
```

---

### Task 2: `knn_cosine_sim` in `face_store.rs`

**Files:**
- Modify: `src-tauri/src/face_store.rs`

- [ ] **Step 1: Write the failing test**

Add to `face_store.rs` test module:

```rust
#[tokio::test]
async fn knn_cosine_sim_returns_similarity_descending() {
    let pool = make_pool(3).await;
    // A=[1,0,0], B=[0.9,0.44,0] (close to A), C=[0,0,1] (orthogonal)
    upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap();
    upsert_vector(&pool, 2, &[0.9, 0.44, 0.0]).await.unwrap();
    upsert_vector(&pool, 3, &[0.0, 0.0, 1.0]).await.unwrap();

    let sims = knn_cosine_sim(&pool, 1, 2).await.unwrap();
    assert_eq!(sims.len(), 2, "should return k=2 results");
    assert_eq!(sims[0].0, 2, "B should be most similar to A");
    assert!(sims[0].1 > sims[1].1, "similarities must be descending");
    assert!(sims[0].1 > 0.5, "B-A cosine similarity should be > 0.5");
    assert!(sims[1].1 < 0.2, "C-A cosine similarity should be near 0");
}

#[tokio::test]
async fn l2_dist_to_cosine_sim_unit_vector_identity() {
    // For identical unit vectors: L2 dist = 0 → cosine sim = 1.0
    assert!((l2_dist_to_cosine_sim(0.0) - 1.0).abs() < 1e-6);
    // For orthogonal unit vectors: L2 dist = sqrt(2) → cosine sim = 0.0
    assert!((l2_dist_to_cosine_sim(2.0f32.sqrt()) - 0.0).abs() < 0.01);
    // For opposite unit vectors: L2 dist = 2.0 → cosine sim = -1.0
    assert!((l2_dist_to_cosine_sim(2.0) - (-1.0)).abs() < 1e-6);
}
```

- [ ] **Step 2: Run — expect FAIL** (functions don't exist)

```bash
cd src-tauri && cargo test knn_cosine_sim l2_dist_to_cosine_sim 2>&1 | tail -10
```

Expected: compile error `cannot find function`

- [ ] **Step 3: Implement in `face_store.rs`**

Add after the existing `knn` function:

```rust
/// Convert sqlite-vec L2 distance to cosine similarity (valid for L2-normalized unit vectors).
/// cos_sim = 1 - d² / 2
pub fn l2_dist_to_cosine_sim(l2_dist: f32) -> f32 {
    1.0 - (l2_dist * l2_dist) / 2.0
}

/// k nearest neighbors of `face_id` by cosine similarity, descending.
/// Excludes `face_id` itself. Returns at most k results.
pub async fn knn_cosine_sim(pool: &SqlitePool, face_id: i64, k: usize) -> Result<Vec<(i64, f32)>> {
    let mut sims: Vec<(i64, f32)> = knn(pool, face_id, k)
        .await?
        .into_iter()
        .map(|(id, dist)| (id, l2_dist_to_cosine_sim(dist)))
        .collect();
    sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(sims)
}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cd src-tauri && cargo test knn_cosine_sim l2_dist_to_cosine_sim 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/face_store.rs
git commit -m "feat(TT-50): knn_cosine_sim + l2_dist_to_cosine_sim in face_store"
```

---

### Task 3: `UnionFind` struct (pure, no DB)

**Files:**
- Modify: `src-tauri/src/clustering.rs` (add at top of file)

- [ ] **Step 1: Write failing tests for UnionFind**

Add to the `#[cfg(test)] mod tests` block in `clustering.rs`:

```rust
#[test]
fn union_find_transitive_chain() {
    let mut uf = UnionFind::new();
    uf.union(1, 2);
    uf.union(2, 3);
    assert!(uf.connected(1, 3), "A-B + B-C edges must put A and C in same component");
    let comps = uf.components(&[1, 2, 3]);
    assert_eq!(comps.len(), 1, "all three must be in one component");
}

#[test]
fn union_find_independent_components_stay_separate() {
    let mut uf = UnionFind::new();
    uf.union(1, 2);
    uf.union(3, 4);
    assert!(!uf.connected(1, 3));
    let comps = uf.components(&[1, 2, 3, 4]);
    assert_eq!(comps.len(), 2);
}

#[test]
fn union_find_components_groups_by_root() {
    let mut uf = UnionFind::new();
    uf.union(1, 2);
    uf.union(1, 3);
    let comps = uf.components(&[1, 2, 3, 4]);
    assert_eq!(comps.len(), 2, "component {1,2,3} and singleton {4}");
    let sizes: Vec<usize> = {
        let mut s: Vec<usize> = comps.values().map(|v| v.len()).collect();
        s.sort_unstable();
        s
    };
    assert_eq!(sizes, vec![1, 3]);
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cd src-tauri && cargo test union_find 2>&1 | tail -10
```

Expected: compile error `cannot find struct UnionFind`

- [ ] **Step 3: Add `UnionFind` to `clustering.rs`**

Add near the top of `clustering.rs`, before the existing functions:

```rust
use std::collections::{HashMap, HashSet};

struct UnionFind {
    parent: HashMap<i64, i64>,
    rank: HashMap<i64, u32>,
}

impl UnionFind {
    fn new() -> Self {
        Self { parent: HashMap::new(), rank: HashMap::new() }
    }

    fn add(&mut self, x: i64) {
        self.parent.entry(x).or_insert(x);
        self.rank.entry(x).or_insert(0);
    }

    fn find(&mut self, x: i64) -> i64 {
        self.add(x);
        if self.parent[&x] != x {
            let root = self.find(self.parent[&x]);
            self.parent.insert(x, root);
        }
        self.parent[&x]
    }

    fn union(&mut self, a: i64, b: i64) {
        self.add(a);
        self.add(b);
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb { return; }
        match self.rank[&ra].cmp(&self.rank[&rb]) {
            std::cmp::Ordering::Less    => { self.parent.insert(ra, rb); }
            std::cmp::Ordering::Greater => { self.parent.insert(rb, ra); }
            std::cmp::Ordering::Equal   => {
                self.parent.insert(rb, ra);
                *self.rank.entry(ra).or_insert(0) += 1;
            }
        }
    }

    fn connected(&mut self, a: i64, b: i64) -> bool {
        self.add(a); self.add(b);
        self.find(a) == self.find(b)
    }

    fn components(&mut self, nodes: &[i64]) -> HashMap<i64, Vec<i64>> {
        let mut groups: HashMap<i64, Vec<i64>> = HashMap::new();
        for &node in nodes {
            let root = self.find(node);
            groups.entry(root).or_default().push(node);
        }
        groups
    }
}
```

Also add at the top of `clustering.rs` if not already present:
```rust
use std::collections::{HashMap, HashSet};
```

- [ ] **Step 4: Run — expect PASS**

```bash
cd src-tauri && cargo test union_find 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(TT-50): UnionFind data structure"
```

---

### Task 4: `compute_mutual_sim_edges` (pure function)

**Files:**
- Modify: `src-tauri/src/clustering.rs`

- [ ] **Step 1: Write failing tests**

Add to the test module in `clustering.rs`:

```rust
#[test]
fn mutual_knn_both_directions_required() {
    // Face 1's knn contains face 2; face 2's knn does NOT contain face 1 → no edge
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    all_knn.insert(1, vec![(2, 0.9), (3, 0.8)]);
    all_knn.insert(2, vec![(3, 0.85), (4, 0.7)]);  // face 1 absent from face 2's list
    all_knn.insert(3, vec![(2, 0.85), (1, 0.8)]);
    all_knn.insert(4, vec![(2, 0.7), (3, 0.6)]);

    let edges = compute_mutual_sim_edges(&all_knn, 0.55);
    let has_1_2 = edges.iter().any(|(a, b, _)| (*a == 1 && *b == 2) || (*a == 2 && *b == 1));
    assert!(!has_1_2, "1-2 must not be an edge: non-mutual (face 1 not in face 2's knn)");
}

#[test]
fn mutual_knn_creates_edge_when_both_in_each_others_topk() {
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    all_knn.insert(1, vec![(2, 0.9)]);
    all_knn.insert(2, vec![(1, 0.9)]);

    let edges = compute_mutual_sim_edges(&all_knn, 0.55);
    assert_eq!(edges.len(), 1);
    assert!((edges[0].2 - 0.9).abs() < 1e-6);
}

#[test]
fn mutual_knn_filters_below_tau_sim() {
    // Both in each other's top-k but similarity is below τ_sim
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    all_knn.insert(1, vec![(2, 0.4)]);  // 0.4 < 0.55
    all_knn.insert(2, vec![(1, 0.4)]);

    let edges = compute_mutual_sim_edges(&all_knn, 0.55);
    assert!(edges.is_empty(), "below-tau mutual pair must not create an edge");
}

#[test]
fn mutual_knn_no_duplicate_edges() {
    // Each side would generate (1,2) and (2,1); function must deduplicate
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    all_knn.insert(1, vec![(2, 0.9)]);
    all_knn.insert(2, vec![(1, 0.9)]);

    let edges = compute_mutual_sim_edges(&all_knn, 0.55);
    assert_eq!(edges.len(), 1, "each pair must appear at most once");
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cd src-tauri && cargo test mutual_knn 2>&1 | tail -10
```

- [ ] **Step 3: Implement `compute_mutual_sim_edges`**

Add to `clustering.rs` (module level, before any `pub` functions):

```rust
fn compute_mutual_sim_edges(
    all_knn: &HashMap<i64, Vec<(i64, f32)>>,
    tau_sim: f32,
) -> Vec<(i64, i64, f32)> {
    let mut edges = Vec::new();
    for (&face_a, neighbors) in all_knn {
        for &(face_b, sim) in neighbors {
            if sim < tau_sim { continue; }
            if face_a >= face_b { continue; }  // deduplicate: only emit with a < b
            // Check mutuality: face_a must appear in face_b's knn
            let is_mutual = all_knn
                .get(&face_b)
                .map_or(false, |nb| nb.iter().any(|(id, _)| *id == face_a));
            if is_mutual {
                edges.push((face_a, face_b, sim));
            }
        }
    }
    edges
}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cd src-tauri && cargo test mutual_knn 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(TT-50): compute_mutual_sim_edges pure function"
```

---

### Task 5: `build_components_with_constraints` (cannot-link enforcement)

**Files:**
- Modify: `src-tauri/src/clustering.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn must_link_joins_below_threshold() {
    // sim=0.3 < TAU_SIM — would be filtered out of sim_edges
    // but must_link forces co-location
    let sim_edges: Vec<(i64, i64, f32)> = vec![];  // no similarity edges pass tau
    let must_links = vec![(1i64, 2i64)];
    let cannot_links: HashSet<(i64, i64)> = HashSet::new();
    let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2, 3]);
    assert!(uf.connected(1, 2), "must_link must co-locate faces regardless of similarity");
    assert!(!uf.connected(1, 3), "face 3 has no edges — must stay isolated");
}

#[test]
fn cannot_link_prevents_weakest_bridge_edge() {
    // Chain 1-2 (sim=0.9), 2-3 (sim=0.6); cannot-link (1,3)
    // Kruskal: add (1,2,0.9) → ok. Try (2,3,0.6) → would put 1 and 3 together → skip.
    let sim_edges = vec![(1i64, 2, 0.9_f32), (2, 3, 0.6)];
    let must_links: Vec<(i64, i64)> = vec![];
    let mut cannot_links: HashSet<(i64, i64)> = HashSet::new();
    cannot_links.insert((1i64, 3i64));
    let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2, 3]);
    assert!(uf.connected(1, 2));
    assert!(!uf.connected(2, 3));
    assert!(!uf.connected(1, 3));
}

#[test]
fn cannot_link_between_independent_groups_not_violated() {
    // Group A: 1-2; Group B: 3-4; cannot-link (1,3); no cross edges → fine
    let sim_edges = vec![(1i64, 2, 0.9_f32), (3, 4, 0.85)];
    let mut cannot_links: HashSet<(i64, i64)> = HashSet::new();
    cannot_links.insert((1i64, 3i64));
    let mut uf = build_components_with_constraints(sim_edges, &[], &cannot_links, &[1, 2, 3, 4]);
    assert!(uf.connected(1, 2));
    assert!(uf.connected(3, 4));
    assert!(!uf.connected(1, 3));
}

#[test]
fn must_link_always_overrides_even_with_cannot_link_warning() {
    // Direct contradiction: must_link AND cannot_link on same pair.
    // Per spec: flag (we eprintln), but must_link still wins.
    let sim_edges: Vec<(i64, i64, f32)> = vec![];
    let must_links = vec![(1i64, 2i64)];
    let mut cannot_links: HashSet<(i64, i64)> = HashSet::new();
    cannot_links.insert((1i64, 2i64));
    let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2]);
    // must_link is applied after sim-edge pass, so 1 and 2 end up connected
    assert!(uf.connected(1, 2), "must_link wins over cannot_link contradiction");
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cd src-tauri && cargo test cannot_link must_link_joins must_link_always 2>&1 | tail -10
```

- [ ] **Step 3: Implement `build_components_with_constraints`**

```rust
fn build_components_with_constraints(
    mut sim_edges: Vec<(i64, i64, f32)>,
    must_links: &[(i64, i64)],
    cannot_links: &HashSet<(i64, i64)>,
    all_faces: &[i64],
) -> UnionFind {
    let mut uf = UnionFind::new();
    for &f in all_faces { uf.add(f); }

    // Kruskal: strongest-first, skip any edge that would newly co-locate a cannot-linked pair
    sim_edges.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    for (fa, fb, _) in &sim_edges {
        let root_fa = uf.find(*fa);
        let root_fb = uf.find(*fb);
        if root_fa == root_fb { continue; }

        let would_violate = cannot_links.iter().any(|&(ca, cb)| {
            let root_ca = uf.find(ca);
            let root_cb = uf.find(cb);
            if root_ca == root_cb { return false; }  // already co-located (pre-existing)
            let root_fa2 = uf.find(*fa);
            let root_fb2 = uf.find(*fb);
            (root_ca == root_fa2 || root_ca == root_fb2) &&
            (root_cb == root_fa2 || root_cb == root_fb2)
        });

        if !would_violate {
            uf.union(*fa, *fb);
        }
    }

    // Must-link: always apply (flag contradiction but don't block)
    for &(fa, fb) in must_links {
        let ordered = if fa < fb { (fa, fb) } else { (fb, fa) };
        if cannot_links.contains(&ordered) {
            eprintln!("[clustering] WARNING: must_link/cannot_link contradiction for faces {} and {}", fa, fb);
        }
        uf.union(fa, fb);
    }

    uf
}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cd src-tauri && cargo test cannot_link must_link_joins must_link_always 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(TT-50): build_components_with_constraints Kruskal + constraint enforcement"
```

---

### Task 6: `compute_label_actions` (pure label assignment)

**Files:**
- Modify: `src-tauri/src/clustering.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn label_assign_one_subject_fills_unlabeled_faces() {
    let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2, 3])].into_iter().collect();
    let face_subjects: HashMap<i64, i64> = [(1i64, 10i64)].into_iter().collect(); // face 1 → subject 10
    let subject_names: HashMap<i64, Option<String>> = [(10i64, Some("Alice".to_string()))].into_iter().collect();

    let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
    let assign = actions.iter().find(|a| matches!(a, LabelAction::AssignAll { .. }));
    assert!(assign.is_some(), "should emit AssignAll for unlabeled faces 2 and 3");
    if let Some(LabelAction::AssignAll { faces, subject_id }) = assign {
        assert!(faces.contains(&2) && faces.contains(&3));
        assert_eq!(*subject_id, 10);
    }
}

#[test]
fn label_two_named_subjects_emits_suggestion_no_fuse() {
    let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2, 3])].into_iter().collect();
    let face_subjects: HashMap<i64, i64> = [(1i64, 10i64), (2i64, 20i64)].into_iter().collect();
    let subject_names: HashMap<i64, Option<String>> = [
        (10i64, Some("Alice".to_string())),
        (20i64, Some("Bob".to_string())),
    ].into_iter().collect();

    let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
    assert!(actions.iter().any(|a| matches!(a, LabelAction::SuggestMerge { .. })),
        "two named subjects must emit a suggestion");
    assert!(!actions.iter().any(|a| matches!(a, LabelAction::AssignAll { .. })),
        "unlabeled face 3 must NOT be auto-assigned in a two-subject component");
}

#[test]
fn label_unlabeled_small_component_is_noise() {
    let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64])].into_iter().collect();
    let face_subjects: HashMap<i64, i64> = HashMap::new();
    let subject_names: HashMap<i64, Option<String>> = HashMap::new();

    let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
    assert!(actions.iter().any(|a| matches!(a, LabelAction::Noise { .. })),
        "size-1 unlabeled component must be noise (below MIN_COMPONENT_SIZE=2)");
}

#[test]
fn label_unlabeled_large_enough_component_gets_new_subject() {
    let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2])].into_iter().collect();
    let face_subjects: HashMap<i64, i64> = HashMap::new();
    let subject_names: HashMap<i64, Option<String>> = HashMap::new();

    let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
    assert!(actions.iter().any(|a| matches!(a, LabelAction::NewSubject { .. })),
        "size>=2 unlabeled component must trigger new subject creation");
}

#[test]
fn label_user_labeled_size_one_is_not_noise() {
    // A user-assigned face in a size-1 component must NOT be classified as noise.
    let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64])].into_iter().collect();
    let face_subjects: HashMap<i64, i64> = [(1i64, 10i64)].into_iter().collect();
    let subject_names: HashMap<i64, Option<String>> = [(10i64, Some("Alice".to_string()))].into_iter().collect();

    let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
    assert!(!actions.iter().any(|a| matches!(a, LabelAction::Noise { faces } if faces.contains(&1))),
        "user-labeled face must not become noise even at size 1");
}

#[test]
fn label_suggestion_requires_at_least_one_named_subject() {
    // Two UNNAMED subjects in same component → no suggestion emitted (neither is named)
    let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2])].into_iter().collect();
    let face_subjects: HashMap<i64, i64> = [(1i64, 10i64), (2i64, 20i64)].into_iter().collect();
    let subject_names: HashMap<i64, Option<String>> = [
        (10i64, None::<String>),
        (20i64, None::<String>),
    ].into_iter().collect();

    let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
    assert!(!actions.iter().any(|a| matches!(a, LabelAction::SuggestMerge { .. })),
        "two unnamed subjects must not generate a merge suggestion");
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cd src-tauri && cargo test label_ 2>&1 | tail -15
```

- [ ] **Step 3: Implement `LabelAction` enum and `compute_label_actions`**

```rust
#[derive(Debug)]
enum LabelAction {
    AssignAll { faces: Vec<i64>, subject_id: i64 },
    NewSubject { faces: Vec<i64> },
    Noise { faces: Vec<i64> },
    SuggestMerge { subject_ids: Vec<i64> },
}

fn compute_label_actions(
    components: &HashMap<i64, Vec<i64>>,
    face_subjects: &HashMap<i64, i64>,
    subject_names: &HashMap<i64, Option<String>>,
    min_component_size: usize,
) -> Vec<LabelAction> {
    let mut actions = Vec::new();
    for faces in components.values() {
        let mut subject_to_faces: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut unlabeled: Vec<i64> = Vec::new();
        for &fid in faces {
            match face_subjects.get(&fid) {
                Some(&sid) => subject_to_faces.entry(sid).or_default().push(fid),
                None => unlabeled.push(fid),
            }
        }
        match subject_to_faces.len() {
            0 => {
                if faces.len() >= min_component_size {
                    actions.push(LabelAction::NewSubject { faces: faces.clone() });
                } else {
                    actions.push(LabelAction::Noise { faces: faces.clone() });
                }
            }
            1 => {
                let &sid = subject_to_faces.keys().next().unwrap();
                if !unlabeled.is_empty() {
                    actions.push(LabelAction::AssignAll { faces: unlabeled, subject_id: sid });
                }
            }
            _ => {
                let any_named = subject_to_faces.keys().any(|sid| {
                    subject_names.get(sid).and_then(|n| n.as_ref()).is_some()
                });
                if any_named {
                    actions.push(LabelAction::SuggestMerge {
                        subject_ids: subject_to_faces.keys().copied().collect(),
                    });
                }
                // Unlabeled faces in multi-subject component stay unassigned (precision-leaning)
            }
        }
    }
    actions
}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cd src-tauri && cargo test label_ 2>&1 | tail -15
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(TT-50): LabelAction enum + compute_label_actions precision-leaning rules"
```

---

### Task 7: Rewrite `cluster_unassigned_faces` + integration tests

**Files:**
- Modify: `src-tauri/src/clustering.rs`

This task replaces the old HDBSCAN function body with the graph algorithm. The function signature is unchanged so callers in `pipeline/mod.rs` compile without modification.

- [ ] **Step 1: Write integration tests (in-memory SQLite + sqlite-vec)**

Add to the test module in `clustering.rs`. These replace the old `recluster_does_not_reassign_removed_face_to_forbidden_subject` and `dismissed_pair_not_re_suggested_after_find_merge_suggestions` tests — delete those two old tests now, before adding these:

```rust
fn emb_bytes(vals: &[f32]) -> Vec<u8> {
    vals.iter().flat_map(|v| v.to_le_bytes()).collect()
}

async fn make_integration_pool() -> sqlx::SqlitePool {
    crate::db::ensure_sqlite_vec_registered();
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    // Minimal schema matching post-migration-6 state
    for stmt in [
        "CREATE TABLE subjects (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, type TEXT NOT NULL DEFAULT 'person', thumbnail_face_id INTEGER, added_at INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE faces (id INTEGER PRIMARY KEY AUTOINCREMENT, image_id INTEGER NOT NULL DEFAULT 0, subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL, bbox_x REAL NOT NULL DEFAULT 0, bbox_y REAL NOT NULL DEFAULT 0, bbox_w REAL NOT NULL DEFAULT 0.5, bbox_h REAL NOT NULL DEFAULT 0.5, added_at INTEGER NOT NULL DEFAULT 0)",
        "CREATE VIRTUAL TABLE face_vectors USING vec0(embedding float[3])",
        "CREATE TABLE constraints (face_a INTEGER NOT NULL, face_b INTEGER NOT NULL, kind TEXT NOT NULL, source TEXT NOT NULL, created_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (face_a, face_b, kind))",
        "CREATE TABLE face_edges (face_a INTEGER NOT NULL, face_b INTEGER NOT NULL, weight REAL NOT NULL, PRIMARY KEY (face_a, face_b))",
        "CREATE TABLE merge_suggestions (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_id_a INTEGER NOT NULL, subject_id_b INTEGER NOT NULL, score REAL NOT NULL, created_at INTEGER NOT NULL DEFAULT 0)",
        "CREATE UNIQUE INDEX idx_merge_pair ON merge_suggestions(CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END, CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END)",
        "CREATE TABLE dismissed_pairs (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_id_a INTEGER NOT NULL, subject_id_b INTEGER NOT NULL, dismissed_at INTEGER NOT NULL DEFAULT 0)",
        "CREATE UNIQUE INDEX idx_dismissed_pair ON dismissed_pairs(subject_id_a, subject_id_b)",
    ] {
        sqlx::query(stmt).execute(&pool).await.unwrap();
    }
    pool
}

#[tokio::test]
async fn integration_remove_face_then_recluster_not_reassigned() {
    let pool = make_integration_pool().await;

    // Subject S (named) — anchor near [1,0,0]
    let subject_s: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('S', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();

    let anchor_s: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
    ).bind(subject_s).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(anchor_s).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();

    // Subject S2 (named) — anchor near [0.998, 0.063, 0] (very close to face_f)
    let subject_s2: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('S2', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let anchor_s2: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
    ).bind(subject_s2).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(anchor_s2).bind(emb_bytes(&[0.998f32, 0.063, 0.0])).execute(&pool).await.unwrap();

    // Face F — unassigned, very close to anchor_s AND anchor_s2
    let face_f: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (3, NULL, 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(face_f).bind(emb_bytes(&[0.999f32, 0.045, 0.0])).execute(&pool).await.unwrap();

    // Simulate: face_f was removed from S → cannot_link(face_f, anchor_s)
    crate::db::add_cannot_link(&pool, face_f, anchor_s, "removal").await.unwrap();

    cluster_unassigned_faces(&pool).await.unwrap();

    let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
        .bind(face_f).fetch_one(&pool).await.unwrap();

    assert!(assigned != Some(subject_s),
        "face_f must NOT be reassigned to forbidden subject S (got {:?})", assigned);
}

#[tokio::test]
async fn integration_merge_distant_groups_stays_merged_after_recluster() {
    let pool = make_integration_pool().await;

    // Group A: faces 1,2 near [1,0,0]
    let subject_a: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('A', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let fa1: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
    ).bind(subject_a).fetch_one(&pool).await.unwrap();
    let fa2: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
    ).bind(subject_a).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fa1).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fa2).bind(emb_bytes(&[0.99f32, 0.14, 0.0])).execute(&pool).await.unwrap();

    // Group B: faces 3,4 near [0,1,0] — embedding-distant from A
    let subject_b: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('B', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let fb1: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (3, ?, 0) RETURNING id"
    ).bind(subject_b).fetch_one(&pool).await.unwrap();
    let fb2: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (4, ?, 0) RETURNING id"
    ).bind(subject_b).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fb1).bind(emb_bytes(&[0.0f32, 1.0, 0.0])).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fb2).bind(emb_bytes(&[0.14f32, 0.99, 0.0])).execute(&pool).await.unwrap();

    // User merges A into B (must_link across groups)
    crate::db::add_must_link(&pool, fa1, fb1, "merge").await.unwrap();
    crate::db::add_must_link(&pool, fa1, fb2, "merge").await.unwrap();
    crate::db::add_must_link(&pool, fa2, fb1, "merge").await.unwrap();
    crate::db::add_must_link(&pool, fa2, fb2, "merge").await.unwrap();
    // Merge subjects in DB: move all B faces to A, delete B
    sqlx::query("UPDATE faces SET subject_id = ? WHERE subject_id = ?")
        .bind(subject_a).bind(subject_b).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM subjects WHERE id = ?")
        .bind(subject_b).execute(&pool).await.unwrap();

    cluster_unassigned_faces(&pool).await.unwrap();

    // All four faces must end up in the same subject
    let subjects: Vec<Option<i64>> = sqlx::query_scalar("SELECT subject_id FROM faces ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    let distinct: HashSet<Option<i64>> = subjects.into_iter().collect();
    assert_eq!(distinct.len(), 1, "all four faces must share one subject after recluster (must_link is durable)");
    assert!(distinct.iter().next().unwrap().is_some(), "subject must not be NULL");
}
```

- [ ] **Step 2: Run — expect FAIL** (function not implemented yet)

```bash
cd src-tauri && cargo test integration_ 2>&1 | tail -15
```

- [ ] **Step 3: Replace the body of `cluster_unassigned_faces`**

Replace the entire existing `cluster_unassigned_faces` function (keep the signature and `ReclusterResult` struct) with:

```rust
pub async fn cluster_unassigned_faces(pool: &SqlitePool) -> Result<ReclusterResult> {
    // 1. Rebuild similarity edge graph
    db::clear_all_face_edges(pool).await?;
    let all_face_ids = db::get_all_face_ids_with_vectors(pool).await?;

    // Build knn map: face_id → Vec<(neighbor_id, cosine_sim)>
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    for &fid in &all_face_ids {
        let neighbors = crate::face_store::knn_cosine_sim(pool, fid, K_NEAREST).await?;
        all_knn.insert(fid, neighbors);
    }

    // Compute mutual sim edges and persist
    let sim_edges = compute_mutual_sim_edges(&all_knn, TAU_SIM);
    for &(fa, fb, weight) in &sim_edges {
        db::upsert_face_edge(pool, fa, fb, weight).await?;
    }

    // 2. Load constraints
    let must_links = db::get_all_must_link_pairs(pool).await?;
    let cannot_links = db::get_all_cannot_link_pairs(pool).await?;

    // 3. Build Union-Find with constraint enforcement
    let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &all_face_ids);

    // 4. Compute components and load current assignments
    let components = uf.components(&all_face_ids);
    let face_subjects = db::get_assigned_face_subject_map(pool).await?;
    let subject_rows = sqlx::query("SELECT id, name FROM subjects")
        .fetch_all(pool).await?;
    let subject_names: HashMap<i64, Option<String>> = subject_rows.into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<Option<String>, _>("name")))
        .collect();

    // 5. Apply label rules
    let actions = compute_label_actions(&components, &face_subjects, &subject_names, MIN_COMPONENT_SIZE);
    let mut new_clusters_count = 0usize;
    let mut noise_count = 0usize;

    for action in actions {
        match action {
            LabelAction::AssignAll { faces, subject_id } => {
                for fid in faces {
                    db::update_face_subject(pool, fid, Some(subject_id)).await?;
                }
            }
            LabelAction::NewSubject { faces } => {
                let sid = db::insert_subject(pool, None, "person").await?;
                for fid in &faces {
                    db::update_face_subject(pool, *fid, Some(sid)).await?;
                }
                new_clusters_count += 1;
            }
            LabelAction::Noise { faces } => {
                for fid in &faces {
                    db::update_face_subject(pool, *fid, None).await?;
                }
                noise_count += faces.len();
            }
            LabelAction::SuggestMerge { .. } => {}  // handled by refresh_merge_suggestions
        }
    }

    // 6. Cleanup
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

Also add to top of `clustering.rs`:
```rust
use crate::db;
```
(keep existing `use crate::db;` if already present — check and merge)

- [ ] **Step 4: Run integration tests — expect PASS**

```bash
cd src-tauri && cargo test integration_ --test-threads=1 2>&1 | tail -15
```

- [ ] **Step 5: Run all tests to check for regressions**

```bash
cd src-tauri && cargo test --test-threads=1 2>&1 | tail -30
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/clustering.rs src-tauri/src/db.rs
git commit -m "feat(TT-50): graph-based cluster_unassigned_faces with Union-Find + constraint enforcement"
```

---

### Task 8: Graph-native `find_merge_suggestions`

**Files:**
- Modify: `src-tauri/src/clustering.rs`

The existing `find_merge_suggestions` uses centroid-cosine comparison. Replace it with a SQL scan over `face_edges` for cross-subject similarity edges.

- [ ] **Step 1: Write integration test for graph-native suggestions**

Add to test module:

```rust
#[tokio::test]
async fn graph_suggestions_emitted_for_cross_subject_edges() {
    let pool = make_integration_pool().await;

    // Alice and Bob — similar embeddings → mutual knn edge after recluster
    let alice: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let bob: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();

    let fa: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
    ).bind(alice).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fa).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();

    let fb: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
    ).bind(bob).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fb).bind(emb_bytes(&[0.99f32, 0.14, 0.0])).execute(&pool).await.unwrap();

    // Manually insert a face_edge between them (as recluster would)
    // cos_sim ≈ 1 - (L2²/2). L2² = (0.01² + 0.14²) ≈ 0.02. sim ≈ 0.99. Above TAU.
    db::upsert_face_edge(&pool, fa, fb, 0.99).await.unwrap();

    find_merge_suggestions(&pool).await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1, "one suggestion expected for Alice-Bob cross edge");
}

#[tokio::test]
async fn graph_suggestions_skipped_for_cannot_link_pair() {
    let pool = make_integration_pool().await;

    let alice: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let bob: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();

    let fa: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
    ).bind(alice).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fa).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();

    let fb: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
    ).bind(bob).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(fb).bind(emb_bytes(&[0.99f32, 0.14, 0.0])).execute(&pool).await.unwrap();

    db::upsert_face_edge(&pool, fa, fb, 0.99).await.unwrap();
    // Dismissed → cannot_link
    crate::db::add_cannot_link(&pool, fa, fb, "dismiss").await.unwrap();

    find_merge_suggestions(&pool).await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0, "dismissed pair (cannot_link) must not be suggested");
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cd src-tauri && cargo test graph_suggestions 2>&1 | tail -15
```

- [ ] **Step 3: Replace `find_merge_suggestions` body**

Replace the entire existing `find_merge_suggestions` function with:

```rust
pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    db::clear_merge_suggestions(pool).await?;

    let now = chrono::Utc::now().timestamp();

    // Cross-subject similarity edges: face_a and face_b belong to different subjects,
    // at least one subject is named, and the face pair is not cannot-linked.
    let rows = sqlx::query(
        r#"SELECT
               MIN(f1.subject_id, f2.subject_id) AS sid_a,
               MAX(f1.subject_id, f2.subject_id) AS sid_b,
               MAX(fe.weight)                    AS score
           FROM face_edges fe
           JOIN faces f1 ON f1.id = fe.face_a
           JOIN faces f2 ON f2.id = fe.face_b
           JOIN subjects s1 ON s1.id = f1.subject_id
           JOIN subjects s2 ON s2.id = f2.subject_id
           WHERE f1.subject_id IS NOT NULL
             AND f2.subject_id IS NOT NULL
             AND f1.subject_id != f2.subject_id
             AND (s1.name IS NOT NULL OR s2.name IS NOT NULL)
             AND NOT EXISTS (
                 SELECT 1 FROM constraints c
                 WHERE c.kind = 'cannot_link'
                   AND c.face_a = MIN(fe.face_a, fe.face_b)
                   AND c.face_b = MAX(fe.face_a, fe.face_b)
             )
           GROUP BY MIN(f1.subject_id, f2.subject_id), MAX(f1.subject_id, f2.subject_id)"#,
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        let sid_a: i64 = row.get("sid_a");
        let sid_b: i64 = row.get("sid_b");
        let score: f64 = row.get("score");

        // Skip dismissed pairs (still track via dismissed_pairs table for UI)
        let dismissed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dismissed_pairs
             WHERE (subject_id_a = ? AND subject_id_b = ?) OR (subject_id_a = ? AND subject_id_b = ?)"
        )
        .bind(sid_a).bind(sid_b).bind(sid_b).bind(sid_a)
        .fetch_one(pool).await?;
        if dismissed > 0 { continue; }

        sqlx::query(
            "INSERT OR IGNORE INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at)
             VALUES (?, ?, ?, ?)"
        )
        .bind(sid_a).bind(sid_b).bind(score).bind(now)
        .execute(pool).await?;
    }

    Ok(())
}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cd src-tauri && cargo test graph_suggestions --test-threads=1 2>&1 | tail -15
```

- [ ] **Step 5: Run all tests**

```bash
cd src-tauri && cargo test --test-threads=1 2>&1 | tail -30
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(TT-50): graph-native find_merge_suggestions via face_edges cross-subject scan"
```

---

### Task 9: Wire user actions to write constraints

**Files:**
- Modify: `src-tauri/src/db.rs` (update `merge_subjects`, `dismiss_merge_suggestion`)
- Modify: `src-tauri/src/commands.rs` (update `unassign_face`, `assign_face_to_subject`, `create_subject_for_face`)

Each user action must write durable constraints so future reclusters honor user intent.

- [ ] **Step 1: Update `merge_subjects` in `db.rs` to write must_link constraints**

In `db.rs`, find `pub async fn merge_subjects` and add constraint writes after the name-resolution block, BEFORE reassigning faces. Insert between the name-update and the `UPDATE faces SET subject_id` query:

```rust
// Write must_link between all faces of target and all faces of source (durable merge)
let target_faces = get_face_ids_for_subject(pool, target_id).await?;
let source_faces = get_face_ids_for_subject(pool, source_id).await?;
let now_c = chrono::Utc::now().timestamp();
for &tf in &target_faces {
    for &sf in &source_faces {
        let (a, b) = if tf < sf { (tf, sf) } else { (sf, tf) };
        sqlx::query(
            "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) VALUES (?, ?, 'must_link', 'merge', ?)"
        ).bind(a).bind(b).bind(now_c).execute(pool).await?;
    }
}
```

- [ ] **Step 2: Write test for merge must_link**

Add to `db.rs` tests:

```rust
#[tokio::test]
async fn merge_subjects_writes_must_link_constraints() {
    let pool = init_test_pool().await;

    let alice: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let bob: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();

    let fa: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, ?, 0,0,1,1,0) RETURNING id"
    ).bind(alice).fetch_one(&pool).await.unwrap();
    let fb: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (2, ?, 0,0,1,1,0) RETURNING id"
    ).bind(bob).fetch_one(&pool).await.unwrap();

    merge_subjects(&pool, alice, bob).await.unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM constraints WHERE kind = 'must_link' AND source = 'merge'"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1, "one must_link expected for fa-fb cross-group pair");

    // Verify the pair is stored with face_a < face_b
    let (stored_a, stored_b): (i64, i64) = sqlx::query_as(
        "SELECT face_a, face_b FROM constraints WHERE kind = 'must_link'"
    ).fetch_one(&pool).await.unwrap();
    let expected_a = fa.min(fb);
    let expected_b = fa.max(fb);
    assert_eq!(stored_a, expected_a);
    assert_eq!(stored_b, expected_b);
}
```

- [ ] **Step 3: Run test — expect PASS**

```bash
cd src-tauri && cargo test merge_subjects_writes_must_link 2>&1 | tail -10
```

- [ ] **Step 4: Update `dismiss_merge_suggestion` in `db.rs` to write cannot_link**

In `db.rs`, find `pub async fn dismiss_merge_suggestion` and add cannot_link writes after the `dismissed_pairs` insert. Insert between the `INSERT OR IGNORE INTO dismissed_pairs` and the `DELETE FROM merge_suggestions` query:

```rust
// Add cannot_link between one representative face from each subject (source='dismiss')
let rep_a: Option<i64> = sqlx::query_scalar(
    "SELECT id FROM faces WHERE subject_id = ? LIMIT 1"
).bind(sid_a).fetch_optional(pool).await?;
let rep_b: Option<i64> = sqlx::query_scalar(
    "SELECT id FROM faces WHERE subject_id = ? LIMIT 1"
).bind(sid_b).fetch_optional(pool).await?;
if let (Some(fa), Some(fb)) = (rep_a, rep_b) {
    let (a, b) = if fa < fb { (fa, fb) } else { (fb, fa) };
    sqlx::query(
        "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) VALUES (?, ?, 'cannot_link', 'dismiss', ?)"
    ).bind(a).bind(b).bind(now).execute(pool).await?;
}
```

Note: `now` is already defined in the function. `sid_a`/`sid_b` refer to the row variables already fetched in the function.

- [ ] **Step 5: Update `unassign_face` command in `commands.rs`**

Replace:
```rust
#[tauri::command]
pub async fn unassign_face(
    face_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::unassign_face(&state.pool, face_id)
        .await
        .map_err(map_err)?;
    let _ = db::auto_assign_missing_thumbnails(&state.pool).await;
    let _ = db::delete_subjects_with_no_faces(&state.pool).await;
    Ok(())
}
```

With:
```rust
#[tauri::command]
pub async fn unassign_face(
    face_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let pool = &state.pool;
    // Record cannot_link between face_id and all current sibling faces in the subject
    if let Ok(Some(face)) = db::get_face_by_id(pool, face_id).await {
        if let Some(subject_id) = face.subject_id {
            if let Ok(siblings) = db::get_face_ids_for_subject(pool, subject_id).await {
                for sibling_id in siblings {
                    if sibling_id != face_id {
                        let _ = db::add_cannot_link(pool, face_id, sibling_id, "removal").await;
                    }
                }
            }
        }
    }
    db::unassign_face(pool, face_id).await.map_err(map_err)?;
    let _ = db::auto_assign_missing_thumbnails(pool).await;
    let _ = db::delete_subjects_with_no_faces(pool).await;
    Ok(())
}
```

- [ ] **Step 6: Update `assign_face_to_subject` command in `commands.rs`**

Replace:
```rust
#[tauri::command]
pub async fn assign_face_to_subject(
    face_id: i64,
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::assign_face_to_subject(&state.pool, face_id, subject_id)
        .await
        .map_err(map_err)
}
```

With:
```rust
#[tauri::command]
pub async fn assign_face_to_subject(
    face_id: i64,
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let pool = &state.pool;
    // must_link between face_id and existing faces in target subject
    if let Ok(existing) = db::get_face_ids_for_subject(pool, subject_id).await {
        for existing_face in existing {
            let _ = db::add_must_link(pool, face_id, existing_face, "manual_assign").await;
        }
    }
    db::assign_face_to_subject(pool, face_id, subject_id).await.map_err(map_err)
}
```

- [ ] **Step 7: Run all tests**

```bash
cd src-tauri && cargo test --test-threads=1 2>&1 | tail -30
```

Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/commands.rs
git commit -m "feat(TT-50): wire user actions (merge/unassign/assign/dismiss) to write constraints"
```

---

### Task 10: Delete old code + remove `hdbscan` dependency

**Files:**
- Modify: `src-tauri/src/clustering.rs` (delete dead functions + dead tests)
- Modify: `src-tauri/Cargo.toml` (remove `hdbscan`)

- [ ] **Step 1: Remove `hdbscan` from `Cargo.toml`**

In `src-tauri/Cargo.toml`, delete the line:
```
hdbscan = "0.12"
```

- [ ] **Step 2: Remove dead code from `clustering.rs`**

Delete these items from `clustering.rs`:
- The `use hdbscan::{...}` import at the top
- The `compute_anchor_centroids` function
- The `find_nearest_anchor` function
- The constant `ANCHOR_MATCH_THRESHOLD`
- The constant `MERGE_CENTROID_SIMILARITY_THRESHOLD`

Also delete these tests from the `mod tests` block:
- `anchor_centroid_is_mean_of_manual_faces`
- `anchor_centroid_falls_back_to_all_faces_when_no_manual`
- `manual_faces_take_priority_over_all_faces`
- `nearest_anchor_assigns_cluster_to_matching_subject`
- `nearest_anchor_returns_none_when_below_threshold`
- `anchor_guided_assignment_prefers_anchor_over_majority`
- `unnamed_unnamed_pair_is_skipped`
- `find_nearest_anchor_skips_forbidden_subject`

(The two old integration tests `recluster_does_not_reassign_removed_face_to_forbidden_subject` and `dismissed_pair_not_re_suggested_after_find_merge_suggestions` were already deleted in Task 7 Step 1.)

- [ ] **Step 3: Remove dead DB functions from `db.rs`**

In `db.rs`, delete:
- `get_subject_face_embeddings` — no longer called (replaced by graph approach)
- `get_subject_embeddings` — no longer called

(Check first with grep to confirm they are only used by the old clustering code.)

```bash
grep -rn "get_subject_face_embeddings\|get_subject_embeddings" src-tauri/src/
```

If grep shows callers outside clustering.rs, keep the function. If only in the deleted clustering code (or in tests that reference it), delete it.

- [ ] **Step 4: Verify the build compiles cleanly**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: no errors. Fix any compilation errors before proceeding.

- [ ] **Step 5: Run the full test suite**

```bash
cd src-tauri && cargo test --test-threads=1 2>&1 | tail -40
```

Expected: all remaining tests pass. All integration tests green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/clustering.rs src-tauri/src/db.rs src-tauri/Cargo.toml
git commit -m "feat(TT-50): remove HDBSCAN/centroid code, hdbscan dep, dead tests — B2 complete"
```

---

## Self-Review

### Spec coverage check

| Spec requirement | Task |
|---|---|
| Node per face; similarity edges = mutual top-k ≥ τ_sim | Task 4 |
| Must-link edges injected from constraints, never dropped | Task 5 |
| Persist edges in `face_edges` | Task 1 (schema) + Task 7 (recluster writes edges) |
| Clusters = connected components (Union-Find) | Task 3 |
| Cannot-link = forbidden co-membership; drop weakest sim edge | Task 5 |
| Must-link/cannot-link direct contradiction → flag, not silent | Task 5 (`eprintln!` warning) |
| Label rule 1: one labeled subject → auto-assign unlabeled | Task 6 |
| Label rule 2: two labeled subjects via sim edge → suggestion, no fuse | Task 6 |
| Label rule 3: no label, size ≥ MIN → new unnamed subject | Task 6 |
| Label rule 4: no label, size < MIN → noise | Task 6 |
| User label override: creates subject at size 1 | Task 6 (`label_user_labeled_size_one_is_not_noise`) |
| Merge(A,B) → must_link A's & B's faces | Task 9 |
| Remove face F from S → cannot_link F ↔ S's faces | Task 9 |
| Manual assign F→S → must_link F to S's faces | Task 9 |
| Dismiss suggestion → cannot_link representative faces, source='dismiss' | Task 9 |
| Merge suggestions = graph-native cross-edges | Task 8 |
| Delete `compute_anchor_centroids`, `find_nearest_anchor`, HDBSCAN, `hdbscan` dep | Task 10 |
| Integration: remove face, recluster → not back in that subject | Task 7 |
| Integration: merge distant groups, recluster → still one subject | Task 7 |
| All unit tests listed in spec | Tasks 3–6 |

### Constants match spec
- `TAU_SIM = 0.55` ✓ · `K_NEAREST = 5` ✓ · `MIN_COMPONENT_SIZE = 2` ✓

### Type consistency
- `compute_mutual_sim_edges` takes `&HashMap<i64, Vec<(i64, f32)>>` → used exactly that way in Task 7 (`all_knn`)
- `build_components_with_constraints` takes `Vec<(i64, i64, f32)>` — matches output of `compute_mutual_sim_edges` ✓
- `compute_label_actions` takes `&HashMap<i64, Vec<i64>>` (from `uf.components`) → `uf.components` returns exactly that ✓
- `LabelAction` variants used identically in Task 6 and Task 7 ✓
- `knn_cosine_sim` returns `Vec<(i64, f32)>` (cosine sim descending) → Task 7 stores these in `all_knn` ✓
