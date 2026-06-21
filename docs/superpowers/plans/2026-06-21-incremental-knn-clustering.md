# Incremental KNN Clustering with Idle Backstop — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the inference pipeline from freezing during imports by moving the expensive full-library KNN sweep off the per-batch critical path — do cheap incremental edge updates per batch, and run one authoritative full sweep only when the queue goes idle.

**Architecture:** Split `cluster_unassigned_faces` into three pieces: a KNN-free in-memory back half (`relabel_from_edges`), a cheap per-batch edge updater for new faces only (`update_edges_incremental`), and the full sweep (`cluster_unassigned_faces`) which now runs KNN *first* (so `face_edges` stays populated), supports mid-sweep cancellation, and serves as the idle backstop. The pipeline loop calls the incremental path per batch and the cancellable full sweep when both queues drain. State (`last_clustered_face_id`, `clustering_dirty`) is persisted in the `settings` table so it survives restarts.

**Tech Stack:** Rust, Tauri, sqlx + SQLite, sqlite-vec (vector KNN), tokio.

## Global Constraints

- Backend is organized into vertical domain slices; **domain SQL lives in the slice `repo.rs`**, never in `db/`. (CLAUDE.md)
- Cross-slice access goes only through the target slice's public API (e.g. `crate::people::repo::*`), never into another slice's internals. (CLAUDE.md)
- Clustering **semantics are unchanged**: mutual kNN + `TAU_SIM = 0.45` + `K_NEAREST = 5` + union-find + `MIN_COMPONENT_SIZE = 2` + constraints all behave exactly as today. (spec Non-goals)
- `#[tauri::command]` handlers (none added here) must be referenced at their definition site in `app/mod.rs`. (CLAUDE.md)
- Exact, transient approximation is acceptable *during* an import; the idle full sweep reconciles drift. Byte-for-byte equivalence at every instant is a non-goal. (spec Non-goals)

## Key existing facts (verified in the codebase)

- `cluster_unassigned_faces(pool)` lives at `src-tauri/src/people/clustering.rs:295`. Its only non-test caller is `src-tauri/src/pipeline/mod.rs:553`. Tests call it at `clustering.rs:823, 932, 1141, 1206`.
- `build_subject_aware_knn` is a private fn at `clustering.rs:116`.
- `compute_mutual_sim_edges` (`clustering.rs:77`), `build_components_with_constraints` (`clustering.rs:242`), `compute_label_actions` (`clustering.rs:189`), `find_merge_suggestions` (`clustering.rs:397`), and `ReclusterResult` (`clustering.rs:456`) already exist.
- `people::repo` already has: `get_all_face_ids_with_vectors` (`repo.rs:785`), `get_assigned_face_subject_map` (`repo.rs:767`), `get_all_similarity_edges` (`repo.rs:729`, currently `#[allow(dead_code)]`), `get_all_must_link_pairs` (`repo.rs:739`), `get_all_cannot_link_pairs` (`repo.rs:749`), `clear_all_face_edges` (`repo.rs:723`), `upsert_face_edge` (`repo.rs:703`), `delete_subjects_with_no_faces`, `auto_assign_missing_thumbnails`, `upgrade_subject_thumbnails`, `get_face_with_image`.
- `face_store::knn_cosine_sim(pool, face_id, k)` returns `Vec<(i64, f32)>` sorted by descending similarity (`face_store.rs:72`).
- `settings::repo` (`settings/repo.rs`) currently has only `get_setting`; there is **no** `set_setting`. The write pattern used elsewhere is `INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)` (`settings/commands.rs:149`).
- `pipeline::queue::count_pending_inference(pool)` (`queue.rs:130`) returns `Result<i64>` = distinct images still queued.
- The pipeline loop is at `mod.rs:161`; idle branch at `mod.rs:190-193`; per-batch recluster block at `mod.rs:550-589`. `processed_subject_work` is declared per-iteration at `mod.rs:290`.
- `run_pipeline` signature has `pool: sqlx::SqlitePool`, `app: tauri::AppHandle`, `data_dir: std::path::PathBuf` (`mod.rs:100`). `use tauri::Emitter;` is in scope inside `run_pipeline` (`mod.rs:110`).
- The runtime is `tauri::async_runtime` (multi-thread tokio), so `tokio::task::block_in_place` is usable.

## Verification commands (used throughout)

Run from the repo root. The crate package is `nebula`.

- Run one test: `cargo test --manifest-path src-tauri/Cargo.toml --lib <test_name> -- --nocapture`
- Run all clustering tests: `cargo test --manifest-path src-tauri/Cargo.toml --lib clustering -- --nocapture`
- Compile + lints: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`

---

### Task 1: `get_face_ids_with_vectors_above` repo helper (people slice)

High-water-mark query: select face rowids strictly greater than a cursor, ordered.

**Files:**
- Modify: `src-tauri/src/people/repo.rs` (add fn near `get_all_face_ids_with_vectors` at `repo.rs:785`)
- Test: `src-tauri/src/people/repo.rs` (add `#[cfg(test)]` test, or extend existing test module if present)

**Interfaces:**
- Produces: `pub async fn get_face_ids_with_vectors_above(pool: &SqlitePool, after_id: i64) -> anyhow::Result<Vec<i64>>` — returns rowids `> after_id`, ascending.

- [ ] **Step 1: Write the failing test**

Add at the bottom of `src-tauri/src/people/repo.rs`:

```rust
#[cfg(test)]
mod hwm_tests {
    use super::*;

    async fn vec_pool() -> SqlitePool {
        crate::db::ensure_sqlite_vec_registered();
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE VIRTUAL TABLE face_vectors USING vec0(embedding float[3])")
            .execute(&pool)
            .await
            .unwrap();
        for rowid in [1i64, 2, 5, 9] {
            sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
                .bind(rowid)
                .bind(vec![0u8; 12]) // 3 × f32
                .execute(&pool)
                .await
                .unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn returns_only_strictly_greater_rowids_ordered() {
        let pool = vec_pool().await;
        let got = get_face_ids_with_vectors_above(&pool, 2).await.unwrap();
        assert_eq!(got, vec![5, 9], "must exclude 2 and return ascending");
    }

    #[tokio::test]
    async fn zero_cursor_returns_all() {
        let pool = vec_pool().await;
        let got = get_face_ids_with_vectors_above(&pool, 0).await.unwrap();
        assert_eq!(got, vec![1, 2, 5, 9]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib returns_only_strictly_greater_rowids_ordered -- --nocapture`
Expected: FAIL to compile — `get_face_ids_with_vectors_above` not found.

- [ ] **Step 3: Write minimal implementation**

Add after `get_all_face_ids_with_vectors` (around `repo.rs:790`):

```rust
/// Face rowids vectorized since `after_id`. Face IDs are autoincrement rowids,
/// so `> after_id` reliably selects faces added after the last incremental pass.
pub async fn get_face_ids_with_vectors_above(
    pool: &SqlitePool,
    after_id: i64,
) -> Result<Vec<i64>> {
    let rows = sqlx::query("SELECT rowid FROM face_vectors WHERE rowid > ? ORDER BY rowid")
        .bind(after_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.get::<i64, _>("rowid")).collect())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib hwm_tests -- --nocapture`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/people/repo.rs
git commit -m "feat(people): get_face_ids_with_vectors_above high-water-mark query"
```

---

### Task 2: `set_setting` helper (settings slice)

The pipeline must persist `clustering_last_face_id` and `clustering_dirty`. `settings::repo` only reads today.

**Files:**
- Modify: `src-tauri/src/settings/repo.rs`
- Test: `src-tauri/src/settings/repo.rs`

**Interfaces:**
- Produces: `pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()>` — upsert; round-trips via existing `get_setting`.

- [ ] **Step 1: Write the failing test**

Add at the bottom of `src-tauri/src/settings/repo.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_then_get_round_trips_and_overwrites() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();

        set_setting(&pool, "clustering_dirty", "true").await.unwrap();
        assert_eq!(
            get_setting(&pool, "clustering_dirty").await.unwrap().as_deref(),
            Some("true")
        );

        set_setting(&pool, "clustering_dirty", "false").await.unwrap();
        assert_eq!(
            get_setting(&pool, "clustering_dirty").await.unwrap().as_deref(),
            Some("false"),
            "second write must overwrite"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib set_then_get_round_trips_and_overwrites -- --nocapture`
Expected: FAIL to compile — `set_setting` not found.

- [ ] **Step 3: Write minimal implementation**

Add to `src-tauri/src/settings/repo.rs` after `get_setting`:

```rust
pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib set_then_get_round_trips_and_overwrites -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/settings/repo.rs
git commit -m "feat(settings): set_setting upsert helper"
```

---

### Task 3: Widen `build_subject_aware_knn` (subset query + cancellation)

Allow querying a subset of faces while still using the full face list for `subject_sizes`, and add a periodic cancellation check. **No behavior change** for the existing full-sweep path (it will pass `faces_to_query == all_face_ids` and `cancel = None`). The return becomes `Option` so the caller can distinguish "cancelled" from "completed".

**Files:**
- Modify: `src-tauri/src/people/clustering.rs` (`build_subject_aware_knn` at `clustering.rs:116`; its sole caller in `cluster_unassigned_faces` at `clustering.rs:311`)

**Interfaces:**
- Produces: `async fn build_subject_aware_knn(pool, all_face_ids: &[i64], faces_to_query: &[i64], face_subjects: &HashMap<i64,i64>, k: usize, cancel: Option<&dyn Fn() -> bool>) -> Result<Option<HashMap<i64, Vec<(i64, f32)>>>>` — `Ok(None)` means a `cancel()` check returned `true` mid-sweep.

- [ ] **Step 1: Replace the function signature and body**

Replace `build_subject_aware_knn` (`clustering.rs:116-169`) with:

```rust
async fn build_subject_aware_knn(
    pool: &SqlitePool,
    all_face_ids: &[i64],
    faces_to_query: &[i64],
    face_subjects: &HashMap<i64, i64>,
    k: usize,
    cancel: Option<&dyn Fn() -> bool>,
) -> Result<Option<HashMap<i64, Vec<(i64, f32)>>>> {
    // Subject sizes are counted from the *full* vectorized set so candidate_k is
    // correct even when we only query a subset (incremental pass).
    let mut subject_sizes: HashMap<i64, usize> = HashMap::new();
    for &fid in all_face_ids {
        if let Some(&sid) = face_subjects.get(&fid) {
            *subject_sizes.entry(sid).or_insert(0) += 1;
        }
    }

    let total = faces_to_query.len();
    let knn_start = Instant::now();
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    for (i, &fid) in faces_to_query.iter().enumerate() {
        if i > 0 && i % 250 == 0 {
            if let Some(c) = cancel {
                if c() {
                    debug!("[clustering] knn cancelled at {i}/{total} faces");
                    return Ok(None);
                }
            }
            debug!(
                "[clustering] knn progress {i}/{total} faces in {:.1}s",
                knn_start.elapsed().as_secs_f32()
            );
        }
        let own_subject = face_subjects.get(&fid).copied();
        let candidate_k = match own_subject {
            Some(sid) => k + subject_sizes.get(&sid).copied().unwrap_or(0),
            None => k,
        };
        let neighbors: Vec<(i64, f32)> =
            crate::people::face_store::knn_cosine_sim(pool, fid, candidate_k)
                .await?
                .into_iter()
                .filter(|(nid, _)| match own_subject {
                    Some(sid) => face_subjects.get(nid).copied() != Some(sid),
                    None => true,
                })
                .take(k)
                .collect();
        all_knn.insert(fid, neighbors);
    }
    Ok(Some(all_knn))
}
```

Keep the existing doc comment block above the function (`clustering.rs:102-115`) unchanged.

- [ ] **Step 2: Update the sole caller in `cluster_unassigned_faces`**

At `clustering.rs:311`, replace:

```rust
    let all_knn = build_subject_aware_knn(pool, &all_face_ids, &face_subjects, K_NEAREST).await?;
```

with:

```rust
    let all_knn = build_subject_aware_knn(
        pool,
        &all_face_ids,
        &all_face_ids,
        &face_subjects,
        K_NEAREST,
        None,
    )
    .await?
    .expect("build_subject_aware_knn returns Some when cancel is None");
```

- [ ] **Step 3: Run the existing clustering tests to verify no behavior change**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib clustering -- --nocapture`
Expected: PASS — all existing tests (`integration_*`, `crowded_subject_*`, `unassigned_face_*`, `graph_suggestions_*`, plus the unit tests) still green.

- [ ] **Step 4: Lints**

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/people/clustering.rs
git commit -m "refactor(people): build_subject_aware_knn supports subset query + cancellation"
```

---

### Task 4: Extract `relabel_from_edges` (the KNN-free back half)

Pull the in-memory "back half" (load persisted edges → union-find with constraints → label actions → assign/new-subject/noise → cleanup → thumbnails → merge suggestions) out of `cluster_unassigned_faces` into a reusable `pub` fn. `cluster_unassigned_faces` keeps its current 1-arg signature in this task and calls the new fn after upserting edges — still no behavior change.

**Files:**
- Modify: `src-tauri/src/people/clustering.rs`
- Modify: `src-tauri/src/people/repo.rs` (remove `#[allow(dead_code)]` from `get_all_similarity_edges` at `repo.rs:728` since it now has a live caller)
- Test: `src-tauri/src/people/clustering.rs` (new integration test)

**Interfaces:**
- Consumes: `people_repo::get_all_similarity_edges`, `get_all_face_ids_with_vectors`, `get_assigned_face_subject_map`, `get_all_must_link_pairs`, `get_all_cannot_link_pairs` (all existing).
- Produces: `pub async fn relabel_from_edges(pool: &SqlitePool) -> Result<ReclusterResult>` — reads `face_edges` + constraints, applies label actions and cleanup, returns counts.

- [ ] **Step 1: Write the failing test**

Add inside the existing `#[cfg(test)] mod tests` in `clustering.rs` (it already has `make_integration_pool`, `emb_bytes`, `unit`):

```rust
    #[tokio::test]
    async fn relabel_from_edges_assigns_unlabeled_in_single_subject_component() {
        let pool = make_integration_pool().await;

        // One named subject with an assigned anchor face.
        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let anchor: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
        // An unlabeled face.
        let orphan: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, NULL, 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // Vectors so both appear in get_all_face_ids_with_vectors.
        for fid in [anchor, orphan] {
            sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
                .bind(fid)
                .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
                .execute(&pool)
                .await
                .unwrap();
        }
        // Seed the edge directly — relabel must consume persisted edges, no KNN.
        people_repo::upsert_face_edge(&pool, anchor, orphan, 0.9)
            .await
            .unwrap();

        let result = relabel_from_edges(&pool).await.unwrap();
        assert_eq!(result.noise, 0);

        let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(orphan)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            assigned,
            Some(alice),
            "orphan in a single-subject component must be assigned to that subject"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib relabel_from_edges_assigns_unlabeled_in_single_subject_component -- --nocapture`
Expected: FAIL to compile — `relabel_from_edges` not found.

- [ ] **Step 3: Add `relabel_from_edges` and refactor `cluster_unassigned_faces` to call it**

Add this new function immediately before `cluster_unassigned_faces` (`clustering.rs:295`):

```rust
/// KNN-free back half of clustering: rebuild components from the *persisted*
/// `face_edges` graph + constraints, apply label actions, then cleanup,
/// thumbnails, and merge suggestions. In-memory union-find over all faces plus a
/// few writes — milliseconds even at ~14k faces.
pub async fn relabel_from_edges(pool: &SqlitePool) -> Result<ReclusterResult> {
    let started = Instant::now();
    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;
    let sim_edges = people_repo::get_all_similarity_edges(pool).await?;
    let must_links = people_repo::get_all_must_link_pairs(pool).await?;
    let cannot_links = people_repo::get_all_cannot_link_pairs(pool).await?;

    let mut uf =
        build_components_with_constraints(sim_edges, &must_links, &cannot_links, &all_face_ids);
    let components = uf.components(&all_face_ids);

    let subject_rows = sqlx::query("SELECT id, name FROM subjects")
        .fetch_all(pool)
        .await?;
    let subject_names: HashMap<i64, Option<String>> = subject_rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<Option<String>, _>("name")))
        .collect();

    let actions = compute_label_actions(
        &components,
        &face_subjects,
        &subject_names,
        MIN_COMPONENT_SIZE,
    );
    let mut new_clusters_count = 0usize;
    let mut noise_count = 0usize;
    for action in actions {
        match action {
            LabelAction::AssignAll { faces, subject_id } => {
                for fid in faces {
                    people_repo::update_face_subject(pool, fid, Some(subject_id)).await?;
                }
            }
            LabelAction::NewSubject { faces } => {
                let sid = people_repo::insert_subject(pool, None, "person").await?;
                for fid in &faces {
                    people_repo::update_face_subject(pool, *fid, Some(sid)).await?;
                }
                new_clusters_count += 1;
            }
            LabelAction::Noise { faces } => {
                for fid in &faces {
                    people_repo::update_face_subject(pool, *fid, None).await?;
                }
                noise_count += faces.len();
            }
            LabelAction::SuggestMerge { .. } => {}
        }
    }

    let deleted = people_repo::delete_subjects_with_no_faces(pool).await?;
    let _ = people_repo::auto_assign_missing_thumbnails(pool).await;
    let _ = find_merge_suggestions(pool).await;

    info!(
        "[clustering] relabel done in {:.1}s: {} new clusters, {} noise faces, {} subjects deleted",
        started.elapsed().as_secs_f32(),
        new_clusters_count,
        noise_count,
        deleted
    );

    Ok(ReclusterResult {
        clusters: new_clusters_count,
        noise: noise_count,
        merged: 0,
        deleted,
    })
}
```

Now in `cluster_unassigned_faces`, replace everything from `// 2. Load constraints` (`clustering.rs:324`) through the end of the function body up to and including the final `Ok(ReclusterResult { ... })` (`clustering.rs:324-394`) with:

```rust
    // Back half (constraints, union-find, labels, cleanup, thumbnails,
    // suggestions) now reads the edges we just persisted.
    let result = relabel_from_edges(pool).await?;
    info!(
        "[clustering] recluster done in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    Ok(result)
```

Leave the top of `cluster_unassigned_faces` (clear edges → knn → `compute_mutual_sim_edges` → `upsert_face_edge` loop, `clustering.rs:295-322`) unchanged in this task.

- [ ] **Step 4: Remove the now-unused `#[allow(dead_code)]`**

In `src-tauri/src/people/repo.rs`, delete the `#[allow(dead_code)]` line directly above `pub async fn get_all_similarity_edges` (`repo.rs:728`).

- [ ] **Step 5: Run tests to verify the new test passes and existing ones are unchanged**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib clustering -- --nocapture`
Expected: PASS — new `relabel_from_edges_*` test green; all existing integration tests still green.

- [ ] **Step 6: Lints**

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings (verifies `get_all_similarity_edges` no longer needs the allow).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/people/clustering.rs src-tauri/src/people/repo.rs
git commit -m "refactor(people): extract relabel_from_edges back half from full sweep"
```

---

### Task 5: Cancellable, deferred-clear full sweep

Change `cluster_unassigned_faces` to: run KNN **first** (so `face_edges` stays populated during the multi-minute KNN), check cancellation, then atomically replace the edge graph in one transaction, then relabel. Signature gains a `cancel` param and returns `Option` (`None` = cancelled). Add a `replace_all_face_edges` repo helper for the atomic swap.

**Files:**
- Modify: `src-tauri/src/people/repo.rs` (add `replace_all_face_edges`)
- Modify: `src-tauri/src/people/clustering.rs` (`cluster_unassigned_faces` signature + body; update 4 test call sites at `clustering.rs:823, 932, 1141, 1206`)

**Interfaces:**
- Consumes: `build_subject_aware_knn` (Task 3), `relabel_from_edges` (Task 4).
- Produces:
  - `pub async fn replace_all_face_edges(pool: &SqlitePool, edges: &[(i64, i64, f32)]) -> Result<()>` — clears then inserts all edges in a single transaction (each pair stored with `face_a < face_b`).
  - `pub async fn cluster_unassigned_faces(pool: &SqlitePool, cancel: Option<&dyn Fn() -> bool>) -> Result<Option<ReclusterResult>>` — `Ok(None)` when cancelled mid-KNN.

- [ ] **Step 1: Add the `replace_all_face_edges` repo helper**

Add to `src-tauri/src/people/repo.rs` after `clear_all_face_edges` (`repo.rs:726`):

```rust
/// Atomically replace the entire edge graph: clear, then insert `edges`, in one
/// transaction. Pairs are normalized to `face_a < face_b` (matching upsert_face_edge).
pub async fn replace_all_face_edges(pool: &SqlitePool, edges: &[(i64, i64, f32)]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM face_edges").execute(&mut *tx).await?;
    for &(fa, fb, weight) in edges {
        let (a, b) = if fa < fb { (fa, fb) } else { (fb, fa) };
        sqlx::query("INSERT OR REPLACE INTO face_edges (face_a, face_b, weight) VALUES (?, ?, ?)")
            .bind(a)
            .bind(b)
            .bind(weight)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 2: Rewrite `cluster_unassigned_faces`**

Replace the whole function body (`clustering.rs:295` through its closing brace, which after Task 4 ends at the `Ok(result)` block) with:

```rust
/// Full authoritative sweep. Serves as the idle backstop. Runs the read-heavy
/// KNN first (so `face_edges` stays populated during the entire multi-minute
/// computation), checks `cancel` periodically, and only swaps the edge graph
/// once KNN completes uncancelled.
///
/// Returns `Ok(None)` if a `cancel()` check fired mid-KNN (new work entered the
/// queue); the caller should leave `clustering_dirty` set and retry later.
pub async fn cluster_unassigned_faces(
    pool: &SqlitePool,
    cancel: Option<&dyn Fn() -> bool>,
) -> Result<Option<ReclusterResult>> {
    let started = Instant::now();
    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;
    info!(
        "[clustering] recluster start: {} vectorized faces, {} already assigned",
        all_face_ids.len(),
        face_subjects.len()
    );

    // KNN first — does NOT touch face_edges, so the table stays valid the whole time.
    let knn_started = Instant::now();
    let all_knn = match build_subject_aware_knn(
        pool,
        &all_face_ids,
        &all_face_ids,
        &face_subjects,
        K_NEAREST,
        cancel,
    )
    .await?
    {
        Some(map) => map,
        None => {
            info!("[clustering] full sweep cancelled — new work entered the queue");
            return Ok(None);
        }
    };
    debug!(
        "[clustering] knn graph built for {} faces in {:.1}s",
        all_face_ids.len(),
        knn_started.elapsed().as_secs_f32()
    );

    // Compute mutual edges and atomically swap the graph.
    let sim_edges = compute_mutual_sim_edges(&all_knn, TAU_SIM);
    people_repo::replace_all_face_edges(pool, &sim_edges).await?;

    // Back half reads the freshly-persisted edges.
    let result = relabel_from_edges(pool).await?;
    info!(
        "[clustering] recluster done in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    Ok(Some(result))
}
```

- [ ] **Step 3: Update the four existing test call sites**

In `clustering.rs`, each of these calls (`clustering.rs:823, 932, 1141, 1206`) currently reads:

```rust
        cluster_unassigned_faces(&pool).await.unwrap();
```

Change each to:

```rust
        cluster_unassigned_faces(&pool, None).await.unwrap();
```

The return is now `Option<ReclusterResult>`; these tests ignore the value, so `.unwrap()` (on the `Result`) still compiles and the assertions are unchanged.

- [ ] **Step 4: Run all clustering tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib clustering -- --nocapture`
Expected: PASS — `integration_remove_face_then_recluster_not_reassigned`, `integration_merge_distant_groups_stays_merged_after_recluster`, `crowded_subject_still_yields_cross_subject_merge_suggestion`, `unassigned_face_still_assigned_to_crowded_subject`, and `relabel_from_edges_*` all green. Behavior of the full sweep is unchanged; only edge-write ordering and the return type changed.

- [ ] **Step 5: Lints**

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/people/clustering.rs src-tauri/src/people/repo.rs
git commit -m "feat(people): cancellable deferred-clear full sweep (cluster_unassigned_faces)"
```

---

### Task 6: `update_edges_incremental` (per-batch edge updates for new faces)

Compute KNN edges for new faces and their immediate neighbors only, and upsert the resulting mutual edges. Does **not** clear edges and does **not** remove stale ones — the idle backstop reconciles drift.

**Files:**
- Modify: `src-tauri/src/people/clustering.rs`
- Test: `src-tauri/src/people/clustering.rs` (two tests: right-edges and incremental+idle convergence)

**Interfaces:**
- Consumes: `people_repo::get_all_face_ids_with_vectors`, `get_assigned_face_subject_map`, `upsert_face_edge`; `face_store::knn_cosine_sim`; `build_subject_aware_knn` (Task 3); `compute_mutual_sim_edges`.
- Produces: `pub async fn update_edges_incremental(pool: &SqlitePool, new_face_ids: &[i64]) -> Result<()>`.

- [ ] **Step 1: Write the failing tests**

Add inside `#[cfg(test)] mod tests` in `clustering.rs`. First, a partition helper and the two tests:

```rust
    /// Group all vectorized faces by subject, returning sorted groups of face ids.
    /// Subject *identity* is ignored — only the partition structure is compared,
    /// which is the right equivalence for from-scratch unassigned imports.
    async fn subject_partition(pool: &sqlx::SqlitePool) -> Vec<Vec<i64>> {
        let rows: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT id, subject_id FROM faces ORDER BY id")
                .fetch_all(pool)
                .await
                .unwrap();
        let mut groups: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut singletons: Vec<Vec<i64>> = Vec::new();
        for (fid, sid) in rows {
            match sid {
                Some(s) => groups.entry(s).or_default().push(fid),
                None => singletons.push(vec![fid]),
            }
        }
        let mut out: Vec<Vec<i64>> = groups.into_values().collect();
        out.extend(singletons);
        for g in &mut out {
            g.sort_unstable();
        }
        out.sort();
        out
    }

    async fn insert_face_with_vector(
        pool: &sqlx::SqlitePool,
        subject_id: Option<i64>,
        v: &[f32],
    ) -> i64 {
        let fid: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(subject_id)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fid)
            .bind(emb_bytes(&unit(v)))
            .execute(pool)
            .await
            .unwrap();
        fid
    }

    #[tokio::test]
    async fn update_edges_incremental_links_new_face_into_existing_cluster() {
        let pool = make_integration_pool().await;
        let alex: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alex', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // Two assigned Alex faces already vectorized.
        let a1 = insert_face_with_vector(&pool, Some(alex), &[1.0, 0.0, 0.0]).await;
        let _a2 = insert_face_with_vector(&pool, Some(alex), &[1.0, 0.02, 0.0]).await;
        // A new, unassigned face inside the cluster.
        let new_face = insert_face_with_vector(&pool, None, &[1.0, 0.01, 0.0]).await;

        update_edges_incremental(&pool, &[new_face]).await.unwrap();

        // An edge between the new face and an Alex face must have been upserted.
        let edge_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM face_edges WHERE face_a = ? OR face_b = ?",
        )
        .bind(new_face)
        .bind(new_face)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(edge_count >= 1, "new face must gain at least one mutual edge");

        // And relabel must then assign it to Alex.
        relabel_from_edges(&pool).await.unwrap();
        let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(new_face)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(assigned, Some(alex));
    }

    #[tokio::test]
    async fn incremental_then_idle_converges_to_full_sweep() {
        // Two well-separated clusters: {A1,A2} near x-axis, {B1,B2} near y-axis.
        let va1 = [1.0f32, 0.0, 0.0];
        let va2 = [0.99f32, 0.14, 0.0];
        let vb1 = [0.0f32, 1.0, 0.0];
        let vb2 = [0.14f32, 0.99, 0.0];

        // Pool 1: incremental in two batches, then a final full sweep.
        let inc = make_integration_pool().await;
        let f1 = insert_face_with_vector(&inc, None, &va1).await;
        let f2 = insert_face_with_vector(&inc, None, &va2).await;
        update_edges_incremental(&inc, &[f1, f2]).await.unwrap();
        relabel_from_edges(&inc).await.unwrap();
        let f3 = insert_face_with_vector(&inc, None, &vb1).await;
        let f4 = insert_face_with_vector(&inc, None, &vb2).await;
        update_edges_incremental(&inc, &[f3, f4]).await.unwrap();
        relabel_from_edges(&inc).await.unwrap();
        cluster_unassigned_faces(&inc, None).await.unwrap();
        let inc_partition = subject_partition(&inc).await;

        // Pool 2: single full sweep over all four faces.
        let full = make_integration_pool().await;
        insert_face_with_vector(&full, None, &va1).await;
        insert_face_with_vector(&full, None, &va2).await;
        insert_face_with_vector(&full, None, &vb1).await;
        insert_face_with_vector(&full, None, &vb2).await;
        cluster_unassigned_faces(&full, None).await.unwrap();
        let full_partition = subject_partition(&full).await;

        assert_eq!(
            inc_partition, full_partition,
            "idle backstop must reconcile incremental state to match a single full sweep"
        );
        // Sanity: the two clusters are distinct.
        assert_eq!(full_partition.len(), 2, "expected two subjects");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib update_edges_incremental_links_new_face_into_existing_cluster -- --nocapture`
Expected: FAIL to compile — `update_edges_incremental` not found.

- [ ] **Step 3: Implement `update_edges_incremental`**

Add to `clustering.rs` immediately after `relabel_from_edges` (and before `cluster_unassigned_faces`). Note `HashSet` is already imported at `clustering.rs:4`.

```rust
/// Cheap per-batch edge update: compute mutual-kNN edges for the *new* faces and
/// their immediate neighbors only, and upsert them. Does NOT clear edges and does
/// NOT remove now-stale edges — the idle full sweep reconciles any drift.
///
/// The affected set `S = new_face_ids ∪ {candidate neighbors of each new face}`
/// is queried so both endpoints of every candidate new edge have a neighbor list,
/// which is what lets `compute_mutual_sim_edges` evaluate mutuality correctly.
pub async fn update_edges_incremental(pool: &SqlitePool, new_face_ids: &[i64]) -> Result<()> {
    if new_face_ids.is_empty() {
        return Ok(());
    }

    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;

    // Build the affected set S.
    let mut affected: HashSet<i64> = new_face_ids.iter().copied().collect();
    for &fid in new_face_ids {
        // Over-fetch by one: knn excludes the query face itself.
        let neighbors =
            crate::people::face_store::knn_cosine_sim(pool, fid, K_NEAREST + 1).await?;
        for (nid, _) in neighbors {
            affected.insert(nid);
        }
    }
    let faces_to_query: Vec<i64> = affected.into_iter().collect();

    // Subject-aware KNN over S only (full id list still drives subject_sizes).
    let local_knn = build_subject_aware_knn(
        pool,
        &all_face_ids,
        &faces_to_query,
        &face_subjects,
        K_NEAREST,
        None,
    )
    .await?
    .expect("build_subject_aware_knn returns Some when cancel is None");

    let edges = compute_mutual_sim_edges(&local_knn, TAU_SIM);
    for &(fa, fb, weight) in &edges {
        people_repo::upsert_face_edge(pool, fa, fb, weight).await?;
    }
    debug!(
        "[clustering] incremental: {} new faces, {} queried, {} edges upserted",
        new_face_ids.len(),
        faces_to_query.len(),
        edges.len()
    );
    Ok(())
}
```

- [ ] **Step 4: Run the new tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib clustering -- --nocapture`
Expected: PASS — both new tests plus all prior tests.

- [ ] **Step 5: Lints**

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/people/clustering.rs
git commit -m "feat(people): update_edges_incremental for per-batch edge updates"
```

---

### Task 7: Wire the pipeline loop (incremental path + cancellable idle backstop)

Replace the inline full sweep on the per-batch critical path with the cheap incremental path, run the cancellable full sweep in the idle branch, persist `last_clustered_face_id` / `clustering_dirty`, and extract the thumbnail-upgrade/emit code into a helper.

This task is verified by compilation, clippy, the existing test suite, and a manual smoke run — the pipeline loop is not unit-testable, but the clustering functions it calls are fully covered by Tasks 4–6.

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`

**Interfaces:**
- Consumes: `crate::people::clustering::{update_edges_incremental, relabel_from_edges, cluster_unassigned_faces}`, `crate::people::repo::get_face_ids_with_vectors_above`, `crate::settings::repo::{get_setting, set_setting}`, `crate::pipeline::queue::count_pending_inference`.
- Produces (module-private helpers in `pipeline/mod.rs`):
  - `async fn upgrade_thumbnails_and_emit(pool: &sqlx::SqlitePool, data_dir: &std::path::Path, app: &tauri::AppHandle)`
  - `fn queue_has_work(pool: &sqlx::SqlitePool) -> bool`

- [ ] **Step 1: Add the two helper functions**

Add near the top of `src-tauri/src/pipeline/mod.rs` (module scope, after the imports — alongside other free functions in the file):

```rust
/// Upgrade each subject's profile crop to its best-quality face, eagerly generate
/// the crop file, then emit `subjects_updated` so the People view refreshes.
async fn upgrade_thumbnails_and_emit(
    pool: &sqlx::SqlitePool,
    data_dir: &std::path::Path,
    app: &tauri::AppHandle,
) {
    use tauri::Emitter;
    if let Ok(changed) = crate::people::repo::upgrade_subject_thumbnails(pool).await {
        debug!("[pipeline] Upgraded thumbnails for {} subjects", changed.len());
        for (_subject_id, face_id) in changed {
            if let Ok(Some((path, bbox))) =
                crate::people::repo::get_face_with_image(pool, face_id).await
            {
                let dest = crate::media::thumbnail::face_crop_path_for(data_dir, face_id);
                if let Err(e) = crate::media::thumbnail::generate_face_crop(
                    std::path::PathBuf::from(path),
                    dest,
                    bbox,
                )
                .await
                {
                    error!("[pipeline] eager crop gen failed for face {face_id}: {e}");
                }
            }
        }
    }
    let _ = app.emit("subjects_updated", ());
}

/// Synchronous "is there pending inference work" probe for the full-sweep cancel
/// closure. Bridges the async queue query from the sync `Fn() -> bool` the
/// clustering API expects. Requires a multi-thread runtime (tauri::async_runtime).
/// On error we report "no work" so an idle sweep is allowed to finish rather than
/// aborting spuriously.
fn queue_has_work(pool: &sqlx::SqlitePool) -> bool {
    tokio::task::block_in_place(|| {
        tauri::async_runtime::block_on(async {
            crate::pipeline::queue::count_pending_inference(pool)
                .await
                .unwrap_or(0)
                > 0
        })
    })
}
```

- [ ] **Step 2: Load persisted clustering state before the loop**

Immediately before `loop {` at `mod.rs:161` (after the "Pipeline background loop started" log at `mod.rs:159`), insert:

```rust
    // Recover incremental-clustering cursor + dirty flag across restarts.
    let mut last_clustered_face_id: i64 = crate::settings::repo::get_setting(&pool, "clustering_last_face_id")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut clustering_dirty: bool = crate::settings::repo::get_setting(&pool, "clustering_dirty")
        .await
        .ok()
        .flatten()
        .map(|s| s == "true")
        .unwrap_or(false);
    info!(
        "[pipeline] clustering state recovered: last_clustered_face_id={last_clustered_face_id}, dirty={clustering_dirty}"
    );
```

- [ ] **Step 3: Run the cancellable full sweep in the idle branch**

Replace the idle branch (`mod.rs:190-193`):

```rust
        if sem_batch.is_empty() && sub_batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
```

with:

```rust
        if sem_batch.is_empty() && sub_batch.is_empty() {
            // Idle backstop: one authoritative full sweep reconciles any drift
            // accumulated by the incremental path. Cancellable so new import work
            // preempts it instead of stalling.
            if clustering_dirty {
                info!("[pipeline] Idle: running authoritative full clustering sweep...");
                let pool_for_cancel = pool.clone();
                let cancel_check = move || queue_has_work(&pool_for_cancel);
                match crate::people::clustering::cluster_unassigned_faces(&pool, Some(&cancel_check))
                    .await
                {
                    Ok(Some(_)) => {
                        upgrade_thumbnails_and_emit(&pool, &data_dir, &app).await;
                        clustering_dirty = false;
                        let _ =
                            crate::settings::repo::set_setting(&pool, "clustering_dirty", "false")
                                .await;
                        info!("[pipeline] Idle full sweep complete.");
                    }
                    Ok(None) => {
                        info!("[pipeline] Idle full sweep cancelled — new work arrived.");
                    }
                    Err(e) => error!("[pipeline] idle full sweep failed: {e}"),
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
```

- [ ] **Step 4: Replace the inline per-batch full sweep with the incremental path**

Replace the entire `if processed_subject_work { ... }` block (`mod.rs:550-589`) with:

```rust
        // Incremental clustering on the critical path: only edges for newly
        // vectorized faces, then an in-memory relabel. Both are cheap, so the loop
        // immediately pulls the next batch. The authoritative full sweep is
        // deferred to the idle branch.
        if processed_subject_work {
            match crate::people::repo::get_face_ids_with_vectors_above(&pool, last_clustered_face_id)
                .await
            {
                Ok(new_ids) => {
                    let max_new = new_ids.iter().copied().max();
                    let incremental_result: anyhow::Result<()> = async {
                        if !new_ids.is_empty() {
                            crate::people::clustering::update_edges_incremental(&pool, &new_ids)
                                .await?;
                        }
                        // Constraints/assignments may have changed even with no new
                        // vectors, so always relabel.
                        crate::people::clustering::relabel_from_edges(&pool).await?;
                        Ok(())
                    }
                    .await;

                    match incremental_result {
                        Ok(()) => {
                            upgrade_thumbnails_and_emit(&pool, &data_dir, &app).await;
                            // Advance the cursor only on success so failed faces retry.
                            if let Some(m) = max_new {
                                last_clustered_face_id = m;
                                let _ = crate::settings::repo::set_setting(
                                    &pool,
                                    "clustering_last_face_id",
                                    &m.to_string(),
                                )
                                .await;
                            }
                            clustering_dirty = true;
                            let _ = crate::settings::repo::set_setting(
                                &pool,
                                "clustering_dirty",
                                "true",
                            )
                            .await;
                        }
                        Err(e) => {
                            error!("[pipeline] incremental clustering failed: {e}");
                        }
                    }
                }
                Err(e) => error!("[pipeline] incremental clustering failed: {e}"),
            }
        }
```

- [ ] **Step 5: Compile and lint the whole crate**

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings, no errors. (Confirms the old inline thumbnail/emit code is fully removed and the extracted helper + new wiring type-check.)

- [ ] **Step 6: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -- --nocapture`
Expected: PASS — all tests, including the clustering integration suite.

- [ ] **Step 7: Manual smoke run**

Build and launch the app against a folder with enough images to enqueue subject work. Confirm in the logs that:
- During the import, `[clustering] incremental: N new faces ...` appears and inference keeps reporting non-zero `img/s` (no multi-minute `knn progress` stall on the critical path).
- The People view updates live as faces are processed.
- After the queue drains, `[pipeline] Idle: running authoritative full clustering sweep...` runs once and then `clustering_dirty` flips to false (no repeated full sweeps while idle).

Document the observed log lines in the PR description.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "feat(pipeline): incremental clustering on critical path + cancellable idle backstop"
```

---

## Self-Review

**Spec coverage:**
- "Refactor into three pieces" → `relabel_from_edges` (Task 4), `update_edges_incremental` (Task 6), cancellable `cluster_unassigned_faces` (Task 5). ✓
- "Refactor `build_subject_aware_knn` (subset + cancel)" → Task 3 with the exact widened signature. ✓
- "New repo helper `get_face_ids_with_vectors_above`" → Task 1. ✓
- "Pipeline loop changes" (state recovery, incremental path, cursor advance on success, dirty flag, cancel-aware idle backstop, extract thumbnail/emit helper) → Task 7. ✓
- "High-water mark semantics" (rowid `>` cursor, recover from settings, out-of-band reconciled at idle) → Tasks 1 + 7. ✓
- "Error handling" (log `[pipeline] incremental clustering failed: {e}` and continue; do not advance cursor on failure; empty `new_ids` still relabels) → Task 7 Step 4. ✓
- "Deferred clear keeps `face_edges` populated during KNN" → Task 5 (KNN first, then `replace_all_face_edges`). ✓
- All five "Testing" bullets → refactor-safety (Tasks 3–5 keep existing tests green), `relabel_from_edges` equivalence (Task 4), `update_edges_incremental` builds right edges (Task 6), incremental+idle convergence (Task 6), high-water mark (Task 1). ✓

**Design decision surfaced (not in spec verbatim):** The spec writes the cancel closure as `Some(&|| queue_has_work(&pool))`, implying a synchronous `bool`. The queue check is async, so `queue_has_work` (Task 7 Step 1) bridges via `tokio::task::block_in_place` + `tauri::async_runtime::block_on`. This requires the multi-thread runtime, which `tauri::async_runtime` provides. The cancel is only polled every 250 faces, so the bridging cost is negligible. Also, `cluster_unassigned_faces` returns `Result<Option<ReclusterResult>>` (`None` = cancelled) to let the caller distinguish cancellation from completion, as the pipeline's "if it completed without cancellation" logic requires.

**Type consistency:** `cluster_unassigned_faces` is `(pool, Option<&dyn Fn() -> bool>) -> Result<Option<ReclusterResult>>` everywhere (def in Task 5, callers in Task 5 tests, Task 6 tests, Task 7). `build_subject_aware_knn` is `(pool, all_face_ids, faces_to_query, face_subjects, k, cancel) -> Result<Option<HashMap<i64, Vec<(i64,f32)>>>>` consistently across Tasks 3, 5, 6. `relabel_from_edges(pool) -> Result<ReclusterResult>` and `update_edges_incremental(pool, &[i64]) -> Result<()>` used consistently. `replace_all_face_edges(pool, &[(i64,i64,f32)])` used only in Task 5. `set_setting`/`get_face_ids_with_vectors_above` signatures match between definition and use. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases"/"similar to" — every code step contains complete code. ✓
