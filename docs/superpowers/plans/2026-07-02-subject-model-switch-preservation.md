# Subject-Model Switching with Data Preservation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `subject_model` actually control which face-recognition preset the pipeline uses, and make switching presets preserve subject names, face→subject assignments, constraints, tags, and thumbnails instead of wiping everything.

**Architecture:** Stamp every `faces` row with the embedder id that produced its vector. Keep face ids stable across a model switch by matching new-model detections to existing `faces` rows by bbox IoU per image (update in place on match, insert on no-match, delete on stale). Filter all clustering reads to the currently-active embedder id so vectors from two different models are never compared. Resolve the active preset per pipeline batch instead of hardcoding it.

**Tech Stack:** Rust, Tauri, sqlx (SQLite), sqlite-vec (`vec0` virtual table), `face_id` crate (ONNX face analyzer).

**Spec:** `docs/superpowers/specs/2026-07-02-subject-model-switch-preservation-design.md`

## Global Constraints

- The migration column is `faces.embedder_id TEXT NOT NULL DEFAULT 'buffalo_s_recognition'` — the value is the embedder's `ModelSpec.id` (`"buffalo_s_recognition"` / `"antelopev2_recognition"`), never the preset id (`"blitz"` / `"precision"`).
- No deletes of `faces`, `face_vectors`, `subjects`, `constraints` anywhere in the new switch flow. Only `merge_suggestions` and `face_edges` are cleared on an embedder change.
- `face_vectors` is a `vec0` virtual table — it has no FK cascade. Every place a `faces` row is deleted, the matching `face_vectors` row must be deleted explicitly in the same code path.
- Domain queries stay in `src-tauri/src/people/repo.rs` (or `src-tauri/src/pipeline/queue.rs` for queue-table SQL); no query logic goes into `src-tauri/src/db/mod.rs` beyond the migration string itself. Cross-slice access goes through the target slice's public API.
- `#[tauri::command]` handlers are referenced at their definition site in `app/mod.rs`; no new commands are added by this plan (spec introduces no new UI-triggered actions).
- All new `sqlx` calls follow the existing repo style: functions take `&SqlitePool` directly (not a generic `Executor`) and do not open a transaction unless the existing function already did (`reset_all_subject_data`, `merge_subjects` are the only precedents).
- Run all commands from `src-tauri/` (the crate root for this workspace member) unless stated otherwise. `cargo test <name> -- --nocapture` runs a single test by substring; `cargo test` runs the full suite.

**Known out-of-scope caveat (flagging, not fixing):** any install where a user already selected "Standard" under the current (buggy) code will have `subject_model` persisted as `"precision"` while every existing `faces` row is actually a `buffalo_s_recognition` embedding (the migration's backfill default is factually correct for these too). Because this plan only triggers `mark_subject_data_stale` from the `update_setting` change-detection branch, such an install will not automatically get its stale faces re-migrated after upgrading — the clustering guard (Task 6) will simply stop clustering those pre-existing faces until the user re-selects a preset (any value) through the UI, which forces a real `update_setting` call. The spec does not ask for a startup reconciliation pass, so this plan does not add one; call this out to the spec owner before shipping if a self-healing migration is wanted.

---

### Task 1: Schema migration — `faces.embedder_id`

**Files:**
- Modify: `src-tauri/src/db/mod.rs:90-101` (BASE_SCHEMA `faces` table), `src-tauri/src/db/mod.rs:185-198` (`VERSIONED_MIGRATIONS`)
- Test: `src-tauri/src/db/tests.rs`

**Interfaces:**
- Produces: `faces.embedder_id` column (`TEXT NOT NULL DEFAULT 'buffalo_s_recognition'`), present on both fresh installs (BASE_SCHEMA) and migrated installs (VERSIONED_MIGRATIONS version 4).

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/db/tests.rs`, near `faces_table_has_quality_columns` (around line 601):

```rust
#[tokio::test]
async fn faces_table_has_embedder_id_column_defaulted() {
    let pool = init_test_pool().await;
    let cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('faces')")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        cols.contains(&"embedder_id".to_string()),
        "faces must have embedder_id; got {cols:?}"
    );

    sqlx::query(
        "INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (100, 1, 0, 0, 1, 1, 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let embedder_id: String = sqlx::query_scalar("SELECT embedder_id FROM faces WHERE id = 100")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        embedder_id, "buffalo_s_recognition",
        "legacy rows (and any row inserted without an explicit embedder_id) must default to buffalo_s_recognition"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test faces_table_has_embedder_id_column_defaulted -- --nocapture`
Expected: FAIL — `no such column: embedder_id`.

- [ ] **Step 3: Implement the migration**

In `src-tauri/src/db/mod.rs`, update the `faces` table inside `BASE_SCHEMA` (lines 90-101):

```rust
CREATE TABLE IF NOT EXISTS faces (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id    INTEGER NOT NULL,
    subject_id  INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    bbox_x      REAL NOT NULL,
    bbox_y      REAL NOT NULL,
    bbox_w      REAL NOT NULL,
    bbox_h      REAL NOT NULL,
    added_at    INTEGER NOT NULL,
    det_score      REAL,
    quality_score  REAL,
    embedder_id    TEXT NOT NULL DEFAULT 'buffalo_s_recognition'
);
```

Append a new entry to `VERSIONED_MIGRATIONS` (lines 185-198):

```rust
const VERSIONED_MIGRATIONS: &[(u32, &str)] = &[
    (
        1,
        "CREATE INDEX IF NOT EXISTS idx_images_done ON images(semantic_analysis_done, subject_analysis_done) WHERE deleted_at IS NULL",
    ),
    (
        2,
        "CREATE INDEX IF NOT EXISTS idx_queue_image ON embedding_queue(image_id)",
    ),
    (
        3,
        "CREATE TABLE IF NOT EXISTS saved_reports (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE, added_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS saved_report_tags (report_id INTEGER NOT NULL REFERENCES saved_reports(id) ON DELETE CASCADE, tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE, PRIMARY KEY (report_id, tag_id));"
    ),
    (
        4,
        "ALTER TABLE faces ADD COLUMN embedder_id TEXT NOT NULL DEFAULT 'buffalo_s_recognition'",
    ),
];
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test faces_table_has_embedder_id_column_defaulted -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run the full existing test suite to confirm no regressions**

Run: `cd src-tauri && cargo test`
Expected: PASS (existing tests untouched by this task alone; `insert_face` call sites are not yet updated so this must still compile and pass as-is since the column has a `DEFAULT` and no other code references it yet).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/mod.rs src-tauri/src/db/tests.rs
git commit -m "feat: add faces.embedder_id column with migration for legacy rows"
```

---

### Task 2: IoU bbox-matching utility

**Files:**
- Create: `src-tauri/src/people/bbox.rs`
- Modify: `src-tauri/src/people/mod.rs:1-9`

**Interfaces:**
- Produces: `pub fn iou(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64` — boxes given as relative `(x, y, w, h)`, matching the `faces.bbox_x/y/w/h` convention used throughout the codebase (see `people::repo::insert_face`, `get_face_with_image`).

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/people/bbox.rs`:

```rust
//! Pure geometry helpers for matching face detections across a model switch.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_boxes_have_iou_one() {
        let a = (0.1, 0.1, 0.2, 0.2);
        assert!((iou(a, a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disjoint_boxes_have_iou_zero() {
        let a = (0.0, 0.0, 0.1, 0.1);
        let b = (0.5, 0.5, 0.1, 0.1);
        assert_eq!(iou(a, b), 0.0);
    }

    #[test]
    fn half_overlap_matches_known_value() {
        // a: [0,1]x[0,1] area=1; b: [0.5,1.5]x[0,1] area=1; intersection: [0.5,1]x[0,1] area=0.5
        // union = 1 + 1 - 0.5 = 1.5; iou = 0.5/1.5 = 1/3
        let a = (0.0, 0.0, 1.0, 1.0);
        let b = (0.5, 0.0, 1.0, 1.0);
        assert!((iou(a, b) - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn zero_area_box_has_iou_zero() {
        let a = (0.1, 0.1, 0.0, 0.2);
        let b = (0.1, 0.1, 0.2, 0.2);
        assert_eq!(iou(a, b), 0.0);
    }

    #[test]
    fn touching_edges_have_iou_zero() {
        // Boxes share only a boundary line -> zero-area intersection.
        let a = (0.0, 0.0, 0.5, 0.5);
        let b = (0.5, 0.0, 0.5, 0.5);
        assert_eq!(iou(a, b), 0.0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test people::bbox:: -- --nocapture`
Expected: FAIL to compile — `cannot find function iou in this scope`.

- [ ] **Step 3: Implement `iou`**

Add above the `#[cfg(test)]` block in `src-tauri/src/people/bbox.rs`:

```rust
/// Intersection-over-union of two axis-aligned boxes in relative `(x, y, w, h)`
/// form (the same convention as `faces.bbox_x/y/w/h`). Returns `0.0` for
/// non-overlapping, merely-touching, or degenerate (zero-area) boxes.
pub fn iou(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    if aw <= 0.0 || ah <= 0.0 || bw <= 0.0 || bh <= 0.0 {
        return 0.0;
    }
    let (ax2, ay2) = (ax + aw, ay + ah);
    let (bx2, by2) = (bx + bw, by + bh);

    let ix1 = ax.max(bx);
    let iy1 = ay.max(by);
    let ix2 = ax2.min(bx2);
    let iy2 = ay2.min(by2);

    let iw = (ix2 - ix1).max(0.0);
    let ih = (iy2 - iy1).max(0.0);
    let inter = iw * ih;
    if inter <= 0.0 {
        return 0.0;
    }

    let union = aw * ah + bw * bh - inter;
    if union <= 0.0 {
        return 0.0;
    }
    inter / union
}
```

Register the module in `src-tauri/src/people/mod.rs`:

```rust
//! People slice: faces, subjects, clustering, merge suggestions.
pub mod bbox;
pub mod clustering;
pub mod commands;
pub mod face_quality;
pub mod face_store;
pub mod models;
pub mod repo;
pub mod service;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test people::bbox:: -- --nocapture`
Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/people/bbox.rs src-tauri/src/people/mod.rs
git commit -m "feat: add IoU bbox-matching utility for face re-identification"
```

---

### Task 3: Repo layer — stamp `embedder_id`, add update/delete for in-place face reprocessing

**Files:**
- Modify: `src-tauri/src/people/repo.rs:25-50` (`insert_face`)
- Modify: `src-tauri/src/people/repo.rs` (add `update_face_detection`, `delete_face` near `update_face_subject`/`unassign_face`)
- Modify: `src-tauri/src/db/tests.rs` (9 existing `insert_face` call sites + new tests)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub async fn insert_face(pool: &SqlitePool, image_id: i64, subject_id: Option<i64>, bbox: (f64,f64,f64,f64), det_score: Option<f64>, quality_score: Option<f64>, embedder_id: &str) -> Result<i64>` (adds trailing `embedder_id` param to the existing function).
  - `pub async fn update_face_detection(pool: &SqlitePool, face_id: i64, bbox: (f64,f64,f64,f64), det_score: f64, quality_score: f64, embedder_id: &str) -> Result<()>`
  - `pub async fn delete_face(pool: &SqlitePool, face_id: i64) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/src/db/tests.rs`, near `insert_face_persists_quality_scores` (around line 812):

```rust
#[tokio::test]
async fn insert_face_persists_embedder_id() {
    let pool = init_test_pool().await;
    let face_id = insert_face(
        &pool,
        1,
        None,
        (0.1, 0.1, 0.2, 0.2),
        Some(0.9),
        Some(0.75),
        "antelopev2_recognition",
    )
    .await
    .unwrap();
    let embedder_id: String = sqlx::query_scalar("SELECT embedder_id FROM faces WHERE id = ?")
        .bind(face_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(embedder_id, "antelopev2_recognition");
}

#[tokio::test]
async fn update_face_detection_overwrites_bbox_scores_and_embedder_preserving_id_and_subject() {
    let pool = init_test_pool().await;
    let sid: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let face_id = insert_face(
        &pool,
        1,
        Some(sid),
        (0.0, 0.0, 0.2, 0.2),
        Some(0.5),
        Some(0.4),
        "buffalo_s_recognition",
    )
    .await
    .unwrap();

    update_face_detection(
        &pool,
        face_id,
        (0.05, 0.06, 0.25, 0.26),
        0.95,
        0.88,
        "antelopev2_recognition",
    )
    .await
    .unwrap();

    let row: (f64, f64, f64, f64, f64, f64, String, Option<i64>) = sqlx::query_as(
        "SELECT bbox_x, bbox_y, bbox_w, bbox_h, det_score, quality_score, embedder_id, subject_id FROM faces WHERE id = ?",
    )
    .bind(face_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, (0.05, 0.06, 0.25, 0.26, 0.95, 0.88, "antelopev2_recognition".to_string(), Some(sid)));
}

#[tokio::test]
async fn delete_face_removes_row_and_cascades_constraints_and_edges() {
    let pool = init_test_pool().await;
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 1, 0,0,1,1,0), (2, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();
    add_cannot_link(&pool, 1, 2, "removal").await.unwrap();
    upsert_face_edge(&pool, 1, 2, 0.5).await.unwrap();

    delete_face(&pool, 1).await.unwrap();

    let face_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM faces WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(face_count, 0);
    let constraint_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(constraint_count, 0, "FK cascade must remove constraints referencing the deleted face");
    let edge_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_edges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(edge_count, 0, "FK cascade must remove face_edges referencing the deleted face");
}
```

Add `update_face_detection` and `delete_face` to the existing `use crate::people::repo::{...}` import block at the top of `src-tauri/src/db/tests.rs` (line 8-13).

Update the other 8 existing `insert_face(...)` call sites in `src-tauri/src/db/tests.rs` to append `, "buffalo_s_recognition"` as the trailing argument (this is required for the crate to compile once Step 3 lands — do it in the same commit as Step 3, not before, so the intermediate state never fails to build for unrelated reasons):
- line ~801: `insert_face(&pool, 1, None, (0.1, 0.1, 0.2, 0.2), Some(0.9), Some(0.75))` → add `"buffalo_s_recognition"`
- line ~824: `insert_face(&pool, 1, Some(sid), (0.0, 0.0, 0.2, 0.2), Some(0.5), Some(0.2))` → add `"buffalo_s_recognition"`
- line ~845: `insert_face(&pool, 2, Some(sid), (0.0, 0.0, 0.3, 0.3), Some(0.9), Some(0.9))` → add `"buffalo_s_recognition"`
- line ~889: `insert_face(&pool, img_id, None, (0.1, 0.2, 0.3, 0.4), Some(0.8), Some(0.7))` → add `"buffalo_s_recognition"`
- line ~1326: `insert_face(&pool, img_a, Some(subject), (0.1, 0.1, 0.2, 0.2), Some(0.9), Some(0.9))` → add `"buffalo_s_recognition"`
- line ~1336: `insert_face(&pool, img_a, Some(subject), (0.5, 0.5, 0.3, 0.3), Some(0.9), Some(0.9))` → add `"buffalo_s_recognition"`
- line ~1346: `insert_face(&pool, img_b, Some(subject), (0.4, 0.6, 0.1, 0.1), Some(0.9), Some(0.9))` → add `"buffalo_s_recognition"`
- line ~1357: `insert_face(&pool, img_c, Some(subject), (0.0, 0.0, 0.1, 0.1), Some(0.9), Some(0.9))` → add `"buffalo_s_recognition"`
- line ~1367: `insert_face(&pool, img_b, Some(other), (0.0, 0.0, 0.1, 0.1), Some(0.9), Some(0.9))` → add `"buffalo_s_recognition"`

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo build 2>&1 | head -50`
Expected: FAIL — compile errors (`update_face_detection`/`delete_face` unresolved, and `insert_face` call-site arity mismatches once the signature changes; before Step 3 lands, the new tests alone fail to resolve `update_face_detection`/`delete_face`).

- [ ] **Step 3: Implement**

In `src-tauri/src/people/repo.rs`, replace `insert_face` (lines 25-50):

```rust
pub async fn insert_face(
    pool: &SqlitePool,
    image_id: i64,
    subject_id: Option<i64>,
    bbox: (f64, f64, f64, f64),
    det_score: Option<f64>,
    quality_score: Option<f64>,
    embedder_id: &str,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at, det_score, quality_score, embedder_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(image_id)
    .bind(subject_id)
    .bind(bbox.0)
    .bind(bbox.1)
    .bind(bbox.2)
    .bind(bbox.3)
    .bind(now)
    .bind(det_score)
    .bind(quality_score)
    .bind(embedder_id)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}
```

Add `update_face_detection` and `delete_face` near `update_face_subject` (after line 305):

```rust
/// Overwrite an existing face row's detection output in place — bbox, scores,
/// and the embedder that produced its (separately updated) vector — while
/// preserving `id`, `subject_id`, and `added_at`. Used when a re-detected face
/// IoU-matches an existing row across a model switch, so `subject_id`,
/// `constraints`, and `thumbnail_face_id` references all survive untouched.
pub async fn update_face_detection(
    pool: &SqlitePool,
    face_id: i64,
    bbox: (f64, f64, f64, f64),
    det_score: f64,
    quality_score: f64,
    embedder_id: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE faces SET bbox_x = ?, bbox_y = ?, bbox_w = ?, bbox_h = ?, det_score = ?, quality_score = ?, embedder_id = ?
         WHERE id = ?",
    )
    .bind(bbox.0)
    .bind(bbox.1)
    .bind(bbox.2)
    .bind(bbox.3)
    .bind(det_score)
    .bind(quality_score)
    .bind(embedder_id)
    .bind(face_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a face row. FK `ON DELETE CASCADE` removes its `constraints` and
/// `face_edges` rows, but NOT its `face_vectors` row (a `vec0` virtual table
/// has no FK support) — callers must also call `face_store::delete_vector`.
pub async fn delete_face(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM faces WHERE id = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

Now update the one production call site, `src-tauri/src/pipeline/mod.rs:97-104` (inside `save_faces`), to pass a literal embedder id for now — this is a placeholder that Task 9 will replace with the resolved preset's embedder id; for this task, just make the crate compile by threading through the already-hardcoded preset:

```rust
        match crate::people::repo::insert_face(
            pool,
            image_id,
            None,
            (rel_x, rel_y, rel_w, rel_h),
            Some(detection.score as f64),
            Some(quality as f64),
            crate::models::registry::BUFFALO_S_PRESET.embedder.id,
        )
        .await
```

Apply the 8 call-site edits listed in Step 1 to `src-tauri/src/db/tests.rs`, and add `update_face_detection, delete_face` to that file's `use crate::people::repo::{...}` block.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test insert_face_persists_embedder_id update_face_detection_overwrites delete_face_removes_row -- --nocapture`
Expected: PASS (3 new tests)

Run: `cd src-tauri && cargo test`
Expected: PASS (full suite, including the 8 updated call sites)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/people/repo.rs src-tauri/src/db/tests.rs src-tauri/src/pipeline/mod.rs
git commit -m "feat: stamp faces.embedder_id on insert; add update/delete for in-place reprocessing"
```

---

### Task 4: Repo layer — `mark_subject_data_stale`

**Files:**
- Modify: `src-tauri/src/people/repo.rs` (add function near `reset_all_subject_data`)
- Modify: `src-tauri/src/db/tests.rs` (new tests)

**Interfaces:**
- Produces: `pub async fn mark_subject_data_stale(pool: &SqlitePool) -> Result<()>`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/db/tests.rs`:

```rust
#[tokio::test]
async fn mark_subject_data_stale_preserves_people_data_clears_edges_and_requeues() {
    let pool = init_test_pool().await;

    let folder_id: i64 =
        sqlx::query_scalar("INSERT INTO folders (path, added_at) VALUES ('/tmp', 0) RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let img_id: i64 = sqlx::query_scalar(
        "INSERT INTO images (folder_id, path, file_hash, mtime, added_at, updated_at, subject_analysis_done)
         VALUES (?, '/tmp/x.jpg', 'hash', 0, 0, 0, 1) RETURNING id",
    )
    .bind(folder_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let sid: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let sid2: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let f1 = insert_face(&pool, img_id, Some(sid), (0.0, 0.0, 0.2, 0.2), Some(0.9), Some(0.9), "buffalo_s_recognition").await.unwrap();
    let f2 = insert_face(&pool, img_id, Some(sid), (0.3, 0.3, 0.2, 0.2), Some(0.9), Some(0.9), "buffalo_s_recognition").await.unwrap();
    let vec_bytes: Vec<u8> = vec![0.0f32; 512].iter().flat_map(|v| v.to_le_bytes()).collect();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(f1)
        .bind(&vec_bytes)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(f2)
        .bind(&vec_bytes)
        .execute(&pool)
        .await
        .unwrap();
    add_must_link(&pool, f1, f2, "merge").await.unwrap();
    upsert_face_edge(&pool, f1, f2, 0.9).await.unwrap();
    sqlx::query("INSERT INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at) VALUES (?, ?, 0.5, 0)")
        .bind(sid)
        .bind(sid2)
        .execute(&pool)
        .await
        .unwrap();

    crate::people::repo::mark_subject_data_stale(&pool).await.unwrap();

    let subject_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subjects").fetch_one(&pool).await.unwrap();
    assert_eq!(subject_count, 2, "subjects (Alice + Bob) must be preserved");
    let face_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM faces").fetch_one(&pool).await.unwrap();
    assert_eq!(face_count, 2, "faces must be preserved");
    let constraint_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints").fetch_one(&pool).await.unwrap();
    assert_eq!(constraint_count, 1, "constraints must be preserved");
    let vector_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_vectors WHERE rowid IN (?, ?)")
        .bind(f1)
        .bind(f2)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(vector_count, 2, "face_vectors rows must be preserved");

    let edge_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_edges").fetch_one(&pool).await.unwrap();
    assert_eq!(edge_count, 0, "face_edges must be cleared");
    let suggestion_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions").fetch_one(&pool).await.unwrap();
    assert_eq!(suggestion_count, 0, "merge_suggestions must be cleared");

    let done: i64 = sqlx::query_scalar("SELECT subject_analysis_done FROM images WHERE id = ?")
        .bind(img_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(done, 0, "image must be marked not-yet-analyzed for the subject pipeline");
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM embedding_queue WHERE image_id = ? AND pipeline = 'subject'")
        .bind(img_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(queued, 1, "image must be re-enqueued on the subject pipeline");
}

#[tokio::test]
async fn mark_subject_data_stale_does_not_duplicate_existing_queue_rows() {
    let pool = init_test_pool().await;
    let folder_id: i64 =
        sqlx::query_scalar("INSERT INTO folders (path, added_at) VALUES ('/tmp', 0) RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let img_id: i64 = sqlx::query_scalar(
        "INSERT INTO images (folder_id, path, file_hash, mtime, added_at, updated_at)
         VALUES (?, '/tmp/x.jpg', 'hash', 0, 0, 0) RETURNING id",
    )
    .bind(folder_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at) VALUES (?, 'subject', 0, 0)")
        .bind(img_id)
        .execute(&pool)
        .await
        .unwrap();

    crate::people::repo::mark_subject_data_stale(&pool).await.unwrap();

    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM embedding_queue WHERE image_id = ? AND pipeline = 'subject'")
        .bind(img_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(queued, 1, "an already-queued image must not get a duplicate queue row");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test mark_subject_data_stale -- --nocapture`
Expected: FAIL to compile — `mark_subject_data_stale` unresolved.

- [ ] **Step 3: Implement**

Add to `src-tauri/src/people/repo.rs`, after `reset_all_subject_data`:

```rust
/// Invalidate only the data that is genuinely stale after an embedder switch:
/// clustering edges and cross-subject merge suggestions computed from the old
/// model's vectors, plus a re-enqueue of every non-deleted image on the
/// `'subject'` pipeline so it gets re-detected and re-embedded. Unlike
/// `reset_all_subject_data`, this never touches `subjects`, `faces`,
/// `face_vectors`, or `constraints` — those survive by id (see
/// `people::service::reprocess_image_faces`).
pub async fn mark_subject_data_stale(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM merge_suggestions")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM face_edges")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE images SET subject_analysis_done = 0 WHERE deleted_at IS NULL")
        .execute(&mut *tx)
        .await?;

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at)
         SELECT id, 'subject', 0, ? FROM images
         WHERE deleted_at IS NULL
           AND id NOT IN (SELECT image_id FROM embedding_queue WHERE pipeline = 'subject')",
    )
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
```

Add `mark_subject_data_stale` to the `use crate::people::repo::{...}` import block at the top of `src-tauri/src/db/tests.rs`, and `add_must_link` if not already imported (it already is, per the existing import list).

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test mark_subject_data_stale -- --nocapture`
Expected: PASS (2 new tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/people/repo.rs src-tauri/src/db/tests.rs
git commit -m "feat: add mark_subject_data_stale as the non-destructive switch-flow invalidation"
```

---

### Task 5: `people::service::reprocess_image_faces` — the IoU matcher

**Files:**
- Create: `src-tauri/src/people/service.rs` (currently a 1-line stub — replace it)

**Interfaces:**
- Consumes: `people::repo::{list_faces_for_image, insert_face, update_face_detection, delete_face}` (Task 3), `people::face_store::{upsert_vector, delete_vector}` (existing), `people::bbox::iou` (Task 2), `people::models::Face` (existing).
- Produces: `pub struct DetectedFaceInput { pub bbox: (f64,f64,f64,f64), pub det_score: f64, pub quality_score: f64, pub embedding: Vec<f32> }`, `pub const MATCH_IOU_THRESHOLD: f64 = 0.5`, `pub async fn reprocess_image_faces(pool: &SqlitePool, image_id: i64, embedder_id: &str, detections: Vec<DetectedFaceInput>, existing: Vec<crate::people::models::Face>) -> anyhow::Result<Vec<i64>>` — returns the ids of every face now holding an `embedder_id`-tagged vector (matched-updated and freshly-inserted), for the caller to feed into incremental clustering. This is consumed by `pipeline::save_faces` in Task 9.

- [ ] **Step 1: Write the failing tests**

Replace `src-tauri/src/people/service.rs` with:

```rust
//! People service: assignment / merge orchestration, and per-image face
//! reprocessing that preserves ids across a face-recognition model switch.

use crate::people::repo as people_repo;
use anyhow::Result;
use sqlx::SqlitePool;

/// One face detected in the current reprocessing pass: relative bbox
/// (matching `faces.bbox_x/y/w/h`), detector confidence, composite quality
/// score, and the embedding vector produced by the active embedder.
pub struct DetectedFaceInput {
    pub bbox: (f64, f64, f64, f64),
    pub det_score: f64,
    pub quality_score: f64,
    pub embedding: Vec<f32>,
}

/// IoU threshold above which a new detection is considered the same physical
/// face as an existing row, so its id (and therefore `subject_id`,
/// constraints, and thumbnail references) survives a model switch.
pub const MATCH_IOU_THRESHOLD: f64 = 0.5;

/// Reconcile one image's face detections against its existing `faces` rows:
/// greedy highest-IoU-first matching (threshold `MATCH_IOU_THRESHOLD`), then:
/// - matched -> update the existing row in place (bbox/scores/embedder_id) and
///   replace its vector; the face id, `subject_id`, and any constraints survive.
/// - unmatched detection -> insert a fresh unassigned row + vector.
/// - unmatched existing row -> delete it (FK cascade removes constraints/edges;
///   its `face_vectors` row is deleted explicitly since vec0 has no FK support).
///
/// Serves both first-time analysis (`existing` empty -> everything inserts)
/// and re-analysis after a switch — no separate migration mode. Safe to retry:
/// each call re-reads `existing` from the database, so a detection that was
/// inserted by a prior failed attempt is matched (not duplicated) on retry.
pub async fn reprocess_image_faces(
    pool: &SqlitePool,
    image_id: i64,
    embedder_id: &str,
    detections: Vec<DetectedFaceInput>,
    existing: Vec<crate::people::models::Face>,
) -> Result<Vec<i64>> {
    let mut candidates: Vec<(usize, usize, f64)> = Vec::new();
    for (di, d) in detections.iter().enumerate() {
        for (ei, e) in existing.iter().enumerate() {
            let ebbox = (e.bbox_x, e.bbox_y, e.bbox_w, e.bbox_h);
            let iou = crate::people::bbox::iou(d.bbox, ebbox);
            if iou >= MATCH_IOU_THRESHOLD {
                candidates.push((di, ei, iou));
            }
        }
    }
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut used_detections: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut used_existing: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut matches: Vec<(usize, usize)> = Vec::new();
    for (di, ei, _iou) in candidates {
        if used_detections.contains(&di) || used_existing.contains(&ei) {
            continue;
        }
        used_detections.insert(di);
        used_existing.insert(ei);
        matches.push((di, ei));
    }

    let mut touched: Vec<i64> = Vec::new();

    for (di, ei) in &matches {
        let d = &detections[*di];
        let face_id = existing[*ei].id;
        people_repo::update_face_detection(pool, face_id, d.bbox, d.det_score, d.quality_score, embedder_id)
            .await?;
        crate::people::face_store::upsert_vector(pool, face_id, &d.embedding).await?;
        touched.push(face_id);
    }

    for (di, d) in detections.iter().enumerate() {
        if used_detections.contains(&di) {
            continue;
        }
        let face_id = people_repo::insert_face(
            pool,
            image_id,
            None,
            d.bbox,
            Some(d.det_score),
            Some(d.quality_score),
            embedder_id,
        )
        .await?;
        crate::people::face_store::upsert_vector(pool, face_id, &d.embedding).await?;
        touched.push(face_id);
    }

    for (ei, e) in existing.iter().enumerate() {
        if used_existing.contains(&ei) {
            continue;
        }
        people_repo::delete_face(pool, e.id).await?;
        crate::people::face_store::delete_vector(pool, e.id).await?;
    }

    Ok(touched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::people::models::Face;

    async fn init_test_pool() -> SqlitePool {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        crate::db::ensure_sqlite_vec_registered();
        let tmp = std::env::temp_dir().join(format!("nebula_service_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&tmp).unwrap();
        crate::db::init_db(&tmp).await.unwrap()
    }

    async fn seed_image(pool: &SqlitePool) -> i64 {
        let folder_id: i64 =
            sqlx::query_scalar("INSERT INTO folders (path, added_at) VALUES ('/tmp', 0) RETURNING id")
                .fetch_one(pool)
                .await
                .unwrap();
        sqlx::query_scalar(
            "INSERT INTO images (folder_id, path, file_hash, mtime, added_at, updated_at)
             VALUES (?, '/tmp/x.jpg', 'hash', 0, 0, 0) RETURNING id",
        )
        .bind(folder_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn emb(seed: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 512];
        v[0] = seed;
        v
    }

    fn det(bbox: (f64, f64, f64, f64), seed: f32) -> DetectedFaceInput {
        DetectedFaceInput {
            bbox,
            det_score: 0.9,
            quality_score: 0.8,
            embedding: emb(seed),
        }
    }

    #[tokio::test]
    async fn first_time_analysis_inserts_all_detections_unassigned() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;

        let touched = reprocess_image_faces(
            &pool,
            image_id,
            "buffalo_s_recognition",
            vec![det((0.1, 0.1, 0.2, 0.2), 1.0), det((0.6, 0.6, 0.2, 0.2), 2.0)],
            vec![],
        )
        .await
        .unwrap();

        assert_eq!(touched.len(), 2);
        let rows: Vec<(Option<i64>, String)> =
            sqlx::query_as("SELECT subject_id, embedder_id FROM faces ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        for (subject_id, embedder_id) in rows {
            assert_eq!(subject_id, None, "first-time faces must be unassigned");
            assert_eq!(embedder_id, "buffalo_s_recognition");
        }
    }

    #[tokio::test]
    async fn matched_detection_preserves_face_id_and_subject_and_updates_embedder() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;
        let sid: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let face_id = crate::people::repo::insert_face(
            &pool,
            image_id,
            Some(sid),
            (0.10, 0.10, 0.20, 0.20),
            Some(0.5),
            Some(0.4),
            "buffalo_s_recognition",
        )
        .await
        .unwrap();
        crate::people::face_store::upsert_vector(&pool, face_id, &emb(1.0))
            .await
            .unwrap();
        let existing = vec![Face {
            id: face_id,
            image_id,
            subject_id: Some(sid),
            bbox_x: 0.10,
            bbox_y: 0.10,
            bbox_w: 0.20,
            bbox_h: 0.20,
            added_at: 0,
        }];

        // Slightly shifted bbox from the new model, but same physical face (IoU > 0.5).
        let touched = reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.11, 0.11, 0.20, 0.20), 9.0)],
            existing,
        )
        .await
        .unwrap();

        assert_eq!(touched, vec![face_id], "must reuse the existing face id, not insert a new one");
        let (subject_id, embedder_id, bbox_x): (Option<i64>, String, f64) =
            sqlx::query_as("SELECT subject_id, embedder_id, bbox_x FROM faces WHERE id = ?")
                .bind(face_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(subject_id, Some(sid), "subject_id must survive the match");
        assert_eq!(embedder_id, "antelopev2_recognition");
        assert_eq!(bbox_x, 0.11, "bbox must be updated to the new detection");
        let vec_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_vectors WHERE rowid = ?")
            .bind(face_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(vec_count, 1, "exactly one vector row must remain for the matched face");
    }

    #[tokio::test]
    async fn matched_face_preserves_constraint_rows() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;
        let face_a = crate::people::repo::insert_face(
            &pool, image_id, None, (0.0, 0.0, 0.2, 0.2), Some(0.5), Some(0.4), "buffalo_s_recognition",
        ).await.unwrap();
        let face_b = crate::people::repo::insert_face(
            &pool, image_id, None, (0.5, 0.5, 0.2, 0.2), Some(0.5), Some(0.4), "buffalo_s_recognition",
        ).await.unwrap();
        crate::people::repo::add_must_link(&pool, face_a, face_b, "merge").await.unwrap();

        let existing = vec![
            Face { id: face_a, image_id, subject_id: None, bbox_x: 0.0, bbox_y: 0.0, bbox_w: 0.2, bbox_h: 0.2, added_at: 0 },
            Face { id: face_b, image_id, subject_id: None, bbox_x: 0.5, bbox_y: 0.5, bbox_w: 0.2, bbox_h: 0.2, added_at: 0 },
        ];

        reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.0, 0.0, 0.2, 0.2), 1.0), det((0.5, 0.5, 0.2, 0.2), 2.0)],
            existing,
        )
        .await
        .unwrap();

        let constraint_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(constraint_count, 1, "must_link between two matched faces must survive by id");
    }

    #[tokio::test]
    async fn unmatched_existing_face_is_deleted_with_its_vector() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;
        let stale_face = crate::people::repo::insert_face(
            &pool, image_id, None, (0.9, 0.9, 0.05, 0.05), Some(0.5), Some(0.4), "buffalo_s_recognition",
        ).await.unwrap();
        crate::people::face_store::upsert_vector(&pool, stale_face, &emb(1.0)).await.unwrap();
        let existing = vec![Face { id: stale_face, image_id, subject_id: None, bbox_x: 0.9, bbox_y: 0.9, bbox_w: 0.05, bbox_h: 0.05, added_at: 0 }];

        // New detection is nowhere near the stale face's bbox -> no match.
        reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.0, 0.0, 0.2, 0.2), 1.0)],
            existing,
        )
        .await
        .unwrap();

        let face_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM faces WHERE id = ?")
            .bind(stale_face)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(face_count, 0, "unmatched existing face must be deleted");
        let vec_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_vectors WHERE rowid = ?")
            .bind(stale_face)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(vec_count, 0, "its face_vectors row must be deleted explicitly (no FK cascade on vec0)");
    }

    #[tokio::test]
    async fn retry_after_partial_insert_matches_instead_of_duplicating() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;

        // Simulate a prior partially-successful attempt: one detection already inserted.
        let already_inserted = crate::people::repo::insert_face(
            &pool, image_id, None, (0.1, 0.1, 0.2, 0.2), Some(0.9), Some(0.8), "antelopev2_recognition",
        ).await.unwrap();
        crate::people::face_store::upsert_vector(&pool, already_inserted, &emb(1.0)).await.unwrap();
        let existing = vec![Face { id: already_inserted, image_id, subject_id: None, bbox_x: 0.1, bbox_y: 0.1, bbox_w: 0.2, bbox_h: 0.2, added_at: 0 }];

        // Retry re-runs detection from scratch and finds the same face again.
        let touched = reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.1, 0.1, 0.2, 0.2), 1.0)],
            existing,
        )
        .await
        .unwrap();

        assert_eq!(touched, vec![already_inserted], "retry must match the already-inserted row, not duplicate it");
        let face_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM faces")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(face_count, 1);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test people::service:: -- --nocapture`
Expected: FAIL to compile (module body was just the stub comment before this step).

- [ ] **Step 3: Confirm implementation compiles and is correct**

The implementation was written together with the tests in Step 1 (this task's production code is the non-test portion of the same file). No separate step needed — proceed to verification.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test people::service:: -- --nocapture`
Expected: PASS (6 tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/people/service.rs
git commit -m "feat: add reprocess_image_faces — IoU-matched in-place face reconciliation"
```

---

### Task 6: Clustering guard — filter by `embedder_id`

**Files:**
- Modify: `src-tauri/src/people/repo.rs:805-810` (`get_all_face_ids_with_vectors`)
- Modify: `src-tauri/src/people/clustering.rs` (`build_subject_aware_knn`, `relabel_from_edges`, `update_edges_incremental`, `cluster_unassigned_faces`, and every test call site + `make_integration_pool`)

**Interfaces:**
- Consumes: `faces.embedder_id` (Task 1).
- Produces (signature changes — all downstream pipeline call sites are updated in Task 10):
  - `pub async fn get_all_face_ids_with_vectors(pool: &SqlitePool, embedder_id: &str) -> Result<Vec<i64>>`
  - `pub async fn relabel_from_edges(pool: &SqlitePool, embedder_id: &str) -> Result<ReclusterResult>`
  - `pub async fn update_edges_incremental(pool: &SqlitePool, new_face_ids: &[i64], embedder_id: &str) -> Result<()>`
  - `pub async fn cluster_unassigned_faces(pool: &SqlitePool, embedder_id: &str, cancel: Option<&AtomicBool>) -> Result<Option<ReclusterResult>>`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/people/clustering.rs`'s `#[cfg(test)] mod tests` block (near `crowded_subject_still_yields_cross_subject_merge_suggestion`):

```rust
    #[tokio::test]
    async fn faces_with_differing_embedder_id_are_never_joined_by_an_edge() {
        let pool = make_integration_pool().await;

        // Two faces, near-identical embeddings (would normally cluster together),
        // but one is tagged with a different (stale) embedder_id.
        let current_face: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at, embedder_id) VALUES (1, NULL, 0, 'antelopev2_recognition') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(current_face)
            .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let stale_face: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at, embedder_id) VALUES (2, NULL, 0, 'buffalo_s_recognition') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(stale_face)
            .bind(emb_bytes(&[0.999f32, 0.045, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        cluster_unassigned_faces(&pool, "antelopev2_recognition", None)
            .await
            .unwrap();

        let edge_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM face_edges WHERE face_a = ? OR face_b = ? OR face_a = ? OR face_b = ?",
        )
        .bind(current_face)
        .bind(current_face)
        .bind(stale_face)
        .bind(stale_face)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(edge_count, 0, "a stale-embedder face must never gain an edge to a current-embedder face");

        let stale_subject: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(stale_face)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stale_subject, None, "the stale face must be left untouched by clustering, not reassigned");
    }

    #[tokio::test]
    async fn get_all_face_ids_with_vectors_filters_by_embedder_id() {
        let pool = make_integration_pool().await;
        let a: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, added_at, embedder_id) VALUES (1, 0, 'antelopev2_recognition') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(a)
            .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();
        let b: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, added_at, embedder_id) VALUES (2, 0, 'buffalo_s_recognition') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(b)
            .bind(emb_bytes(&[0.0f32, 1.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let current = people_repo::get_all_face_ids_with_vectors(&pool, "antelopev2_recognition")
            .await
            .unwrap();
        assert_eq!(current, vec![a]);
    }

    #[tokio::test]
    async fn named_subjects_across_embedder_ids_never_get_a_merge_suggestion() {
        let pool = make_integration_pool().await;

        // Alice (current embedder) and Bob (stale embedder) have near-identical
        // embeddings — if embedder_id were not filtered, this is exactly the
        // shape that would normally produce a cross-subject merge suggestion.
        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let alice_face: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at, embedder_id) VALUES (1, ?, 0, 'antelopev2_recognition') RETURNING id",
        )
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(alice_face)
            .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let bob: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let bob_face: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at, embedder_id) VALUES (2, ?, 0, 'buffalo_s_recognition') RETURNING id",
        )
        .bind(bob)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(bob_face)
            .bind(emb_bytes(&[0.999f32, 0.045, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        cluster_unassigned_faces(&pool, "antelopev2_recognition", None)
            .await
            .unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "a stale-embedder subject must never be suggested for merge with a current-embedder one"
        );
    }
```

Update `make_integration_pool` (add `embedder_id` column to the hand-rolled `faces` table):

```rust
            "CREATE TABLE faces (id INTEGER PRIMARY KEY AUTOINCREMENT, image_id INTEGER NOT NULL DEFAULT 0, subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL, bbox_x REAL NOT NULL DEFAULT 0, bbox_y REAL NOT NULL DEFAULT 0, bbox_w REAL NOT NULL DEFAULT 0.5, bbox_h REAL NOT NULL DEFAULT 0.5, added_at INTEGER NOT NULL DEFAULT 0, embedder_id TEXT NOT NULL DEFAULT 'buffalo_s_recognition')",
```

Update every existing call site in `src-tauri/src/people/clustering.rs`'s test module to the new 3-arg/2-arg/3-arg signatures (all existing test faces default to `'buffalo_s_recognition'` via the fixture's new column default, so passing that literal keeps every existing assertion unchanged):
- `cluster_unassigned_faces(&pool, None)` → `cluster_unassigned_faces(&pool, "buffalo_s_recognition", None)` (4 call sites: `integration_remove_face_then_recluster_not_reassigned`, `integration_merge_distant_groups_stays_merged_after_recluster`'s trailing call, `crowded_subject_still_yields_cross_subject_merge_suggestion`, `unassigned_face_still_assigned_to_crowded_subject`)
- `cluster_unassigned_faces(&inc, None)` / `cluster_unassigned_faces(&full, None)` in `incremental_then_idle_converges_to_full_sweep` → add `"buffalo_s_recognition"` as the second arg to both calls
- `cluster_unassigned_faces(&pool, Some(&cancel))` (2 call sites: `full_sweep_cancelled_returns_none_and_leaves_edges_untouched`, `full_sweep_with_uncancelled_flag_completes_normally`) → `cluster_unassigned_faces(&pool, "buffalo_s_recognition", Some(&cancel))`
- `update_edges_incremental(&pool, &[new_face])` → `update_edges_incremental(&pool, &[new_face], "buffalo_s_recognition")`
- `update_edges_incremental(&inc, &[f1, f2])` / `update_edges_incremental(&inc, &[f3, f4])` → add `, "buffalo_s_recognition"` to both
- `relabel_from_edges(&pool)` (2 call sites: inside `update_edges_incremental_links_new_face_into_existing_cluster`, and `relabel_from_edges_assigns_unlabeled_in_single_subject_component`) → `relabel_from_edges(&pool, "buffalo_s_recognition")`
- `relabel_from_edges(&inc)` (2 call sites inside `incremental_then_idle_converges_to_full_sweep`) → `relabel_from_edges(&inc, "buffalo_s_recognition")`

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo build 2>&1 | head -80`
Expected: FAIL — arity mismatches on every updated call site, plus the two new tests failing to resolve the extra argument.

- [ ] **Step 3: Implement**

In `src-tauri/src/people/repo.rs`, replace `get_all_face_ids_with_vectors` (around line 805):

```rust
pub async fn get_all_face_ids_with_vectors(pool: &SqlitePool, embedder_id: &str) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT fv.rowid AS id FROM face_vectors fv
         JOIN faces f ON f.id = fv.rowid
         WHERE f.embedder_id = ?",
    )
    .bind(embedder_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.get::<i64, _>("id")).collect())
}
```

In `src-tauri/src/people/clustering.rs`, update `build_subject_aware_knn` (lines 117-169) to filter neighbor results against the (already embedder-filtered) `all_face_ids`:

```rust
async fn build_subject_aware_knn(
    pool: &SqlitePool,
    all_face_ids: &[i64],
    faces_to_query: &[i64],
    face_subjects: &HashMap<i64, i64>,
    k: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Option<HashMap<i64, Vec<(i64, f32)>>>> {
    // `all_face_ids` is already filtered to the current preset's embedder_id by
    // the caller (via `get_all_face_ids_with_vectors`). Every neighbor result
    // is filtered against this set too, so a stale-embedder face can never
    // become a mutual-kNN edge endpoint regardless of raw vector similarity.
    let valid_ids: HashSet<i64> = all_face_ids.iter().copied().collect();

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
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                debug!("[clustering] knn cancelled at {i}/{total} faces");
                return Ok(None);
            }
        }
        if i > 0 && i % 250 == 0 {
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
                .filter(|(nid, _)| valid_ids.contains(nid))
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

Update `relabel_from_edges` (line 299) to accept and thread through `embedder_id`:

```rust
pub async fn relabel_from_edges(pool: &SqlitePool, embedder_id: &str) -> Result<ReclusterResult> {
    let started = Instant::now();
    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool, embedder_id).await?;
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

Update `update_edges_incremental` (line 378) similarly:

```rust
pub async fn update_edges_incremental(
    pool: &SqlitePool,
    new_face_ids: &[i64],
    embedder_id: &str,
) -> Result<()> {
    if new_face_ids.is_empty() {
        return Ok(());
    }

    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool, embedder_id).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;

    let mut affected: HashSet<i64> = new_face_ids.iter().copied().collect();
    for &fid in new_face_ids {
        let neighbors = crate::people::face_store::knn_cosine_sim(pool, fid, K_NEAREST + 1).await?;
        for (nid, _) in neighbors {
            affected.insert(nid);
        }
    }
    let faces_to_query: Vec<i64> = affected.into_iter().collect();

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

Update `cluster_unassigned_faces` (line 429):

```rust
pub async fn cluster_unassigned_faces(
    pool: &SqlitePool,
    embedder_id: &str,
    cancel: Option<&AtomicBool>,
) -> Result<Option<ReclusterResult>> {
    let started = Instant::now();
    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool, embedder_id).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;
    info!(
        "[clustering] recluster start: {} vectorized faces, {} already assigned",
        all_face_ids.len(),
        face_subjects.len()
    );

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

    let sim_edges = compute_mutual_sim_edges(&all_knn, TAU_SIM);
    people_repo::replace_all_face_edges(pool, &sim_edges).await?;

    let result = relabel_from_edges(pool, embedder_id).await?;
    info!(
        "[clustering] recluster done in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    Ok(Some(result))
}
```

Apply all the test call-site edits listed in Step 1.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test people::clustering:: people::repo:: -- --nocapture`
Expected: PASS (all existing clustering tests plus the 2 new ones)

Run: `cd src-tauri && cargo test`
Expected: PASS (full suite — note `pipeline/mod.rs` production call sites of these three functions still compile only because Task 3's placeholder and the not-yet-updated call sites; if `cargo build` fails here because `pipeline/mod.rs` calls these with the old arity, fix those 3 call sites now by passing `crate::models::registry::BUFFALO_S_PRESET.embedder.id` as a temporary literal — Task 10 replaces it with the resolved preset).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/people/repo.rs src-tauri/src/people/clustering.rs src-tauri/src/pipeline/mod.rs
git commit -m "feat: filter clustering reads to the active preset's embedder_id"
```

---

### Task 7: Settings switch flow — preserve data, resolve preset

**Files:**
- Modify: `src-tauri/src/settings/commands.rs`

**Interfaces:**
- Consumes: `people::repo::mark_subject_data_stale` (Task 4).
- Produces: `pub(crate) fn resolve_subject_preset(value: Option<&str>) -> &'static crate::models::registry::FaceIdPreset` — reused by `pipeline::run_pipeline` in Task 8.

**Testing scope note:** `update_setting` is a `#[tauri::command]` taking `State<'_, AppState>` and `tauri::AppHandle`, neither of which is practical to construct in a unit test (it needs a real `ModelManager` and would call `ensure_ready`, which touches the filesystem/network). The existing test module already avoids testing `update_setting` directly for the same reason. This task instead extracts the one piece of `update_setting`'s logic that decides staleness — resolving a stored setting value to a preset — into `resolve_subject_preset`, which is pure and fully unit-testable, and covers spec testing item 4 ("changing to a preset with the same embedder id does not mark data stale") at that level.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/settings/commands.rs` (after `default_subject_model_matches_first_preset`):

```rust
    #[test]
    fn resolve_subject_preset_defaults_to_blitz_when_unset() {
        use super::resolve_subject_preset;
        let resolved = resolve_subject_preset(None);
        assert_eq!(resolved.id, "blitz");
    }

    #[test]
    fn resolve_subject_preset_falls_back_to_blitz_for_unknown_id() {
        use super::resolve_subject_preset;
        let resolved = resolve_subject_preset(Some("not-a-real-preset"));
        assert_eq!(resolved.id, "blitz");
    }

    #[test]
    fn resolve_subject_preset_returns_the_matching_preset() {
        use super::resolve_subject_preset;
        assert_eq!(resolve_subject_preset(Some("precision")).id, "precision");
    }

    #[test]
    fn unset_setting_and_explicit_blitz_resolve_to_the_same_embedder() {
        // Confirms selecting Blitz when nothing was previously set is a no-op
        // for staleness purposes: both resolve to the same embedder id, since
        // the wiring bug meant every pre-fix embedding was already buffalo_s.
        use super::resolve_subject_preset;
        assert_eq!(
            resolve_subject_preset(None).embedder.id,
            resolve_subject_preset(Some("blitz")).embedder.id
        );
    }

    #[test]
    fn switching_between_presets_changes_the_resolved_embedder_id() {
        use super::resolve_subject_preset;
        assert_ne!(
            resolve_subject_preset(Some("blitz")).embedder.id,
            resolve_subject_preset(Some("precision")).embedder.id
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test resolve_subject_preset -- --nocapture`
Expected: FAIL to compile — `resolve_subject_preset` unresolved.

- [ ] **Step 3: Implement**

In `src-tauri/src/settings/commands.rs`, add above `update_setting`:

```rust
/// Resolve a possibly-unset `subject_model` setting value to its preset,
/// defaulting to Blitz — the preset actually used for every face embedded
/// before the §1 wiring fix, regardless of what the setting said. Also the
/// fallback for a value that no longer matches a known preset id.
pub(crate) fn resolve_subject_preset(value: Option<&str>) -> &'static FaceIdPreset {
    value
        .and_then(FaceIdPreset::find_by_id)
        .unwrap_or(&crate::models::registry::BUFFALO_S_PRESET)
}
```

Replace the `if key == "subject_model"` block inside `update_setting` (lines 121-147):

```rust
    if key == "subject_model" {
        let current = crate::settings::repo::get_setting(pool, &key)
            .await
            .unwrap_or(None);
        if current.as_ref() != Some(&value) {
            let preset = crate::models::registry::FaceIdPreset::find_by_id(&value)
                .ok_or_else(|| format!("Unknown preset: {}", value))?;
            state
                .model_manager
                .ensure_ready(&app, preset.detector)
                .await
                .map_err(|e| e.to_string())?;
            state
                .model_manager
                .ensure_ready(&app, preset.embedder)
                .await
                .map_err(|e| e.to_string())?;
            state
                .model_manager
                .ensure_ready(&app, preset.gender_age)
                .await
                .map_err(|e| e.to_string())?;

            let old_preset = resolve_subject_preset(current.as_deref());
            if old_preset.embedder.id != preset.embedder.id {
                crate::people::repo::mark_subject_data_stale(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test resolve_subject_preset -- --nocapture`
Expected: PASS (5 tests)

Run: `cd src-tauri && cargo test`
Expected: PASS (full suite)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/settings/commands.rs
git commit -m "feat: preserve people data on subject_model switch when the embedder is unchanged"
```

---

### Task 8: Pipeline wiring fix — resolve preset per batch

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs:137-197` (`run_pipeline` setup + loop top)

**Interfaces:**
- Consumes: `settings::commands::resolve_subject_preset` (Task 7, made `pub(crate)`), `vision::engine::VisionEngine::get_face_analyzer` (existing, already caches by `preset.id` internally), `pipeline::face_actor::spawn_face_actor` (existing).
- Produces: the loop now holds a mutable `(String, mpsc::Sender<face_actor::FaceRequest>)` pair for the active preset, re-resolved once per loop iteration; `preset` is no longer hardcoded to `BUFFALO_S_PRESET`.

This task has no new automated test — `run_pipeline` requires a live Tauri `AppHandle`, a real `ModelManager`, and downloaded ONNX models, none of which are unit-testable in this crate today (see `vision::engine`'s tests, all gated on `NEBULA_TEST_DATA_DIR` and skipped otherwise). Verification is a `cargo build` pass plus the manual smoke test in Task 11.

- [ ] **Step 1: Implement**

In `src-tauri/src/pipeline/mod.rs`, replace lines 137-197 (the `run_pipeline` signature through the initial face-analyzer setup, stopping just before `info!("[pipeline] Pipeline background loop started...")`):

```rust
#[allow(clippy::too_many_arguments)]
pub async fn run_pipeline(
    pool: sqlx::SqlitePool,
    app: tauri::AppHandle,
    engine: Arc<crate::vision::engine::VisionEngine>,
    manager: Arc<crate::models::ModelManager>,
    index: crate::search::vector_index::IndexStore,
    data_dir: std::path::PathBuf,
    config: PipelineConfig,
    requested_spec: &'static crate::models::registry::ModelSpec,
) {
    use tauri::Emitter;

    // Require split towers; fall back to SIGLIP_BASE if the chosen model lacks them.
    let spec: &'static crate::models::registry::ModelSpec = if requested_spec.vision_file.is_some()
    {
        requested_spec
    } else {
        warn!(
            "[pipeline] model '{}' has no split towers; falling back to SIGLIP_BASE",
            requested_spec.id
        );
        &crate::models::registry::SIGLIP_BASE
    };

    info!("[pipeline] Ensuring embed model is ready...");
    if let Err(e) = manager.ensure_ready(&app, spec).await {
        error!("[pipeline] embed model not ready: {e}");
        return;
    }
    info!("[pipeline] Embed model ready.");

    let initial_preset = resolve_subject_preset(&pool).await;
    let initial_analyzer = match ensure_face_preset(&app, &engine, &manager, initial_preset).await {
        Ok(a) => a,
        Err(e) => {
            error!("[pipeline] face analyzer init failed: {e}");
            return;
        }
    };
    info!("[pipeline] Face analyzer initialized ('{}').", initial_preset.id);
    let mut subject_preset = initial_preset;
    let mut face_tx = face_actor::spawn_face_actor(initial_analyzer, config.infer_channel_depth);

    let embed_tx = embed_actor::spawn_embed_actor(
        engine.clone(),
        manager.clone(),
        spec,
        config.batch_size,
        config.infer_channel_depth,
    );

    info!("[pipeline] Pipeline background loop started, awaiting tasks...");
```

Add two helpers just above `run_pipeline` (after `save_faces`, before the `#[allow(clippy::too_many_arguments)]` line):

```rust
/// Resolve the `subject_model` setting to its preset, falling back to Blitz
/// for an unset or unrecognized value. Delegates to the settings slice's
/// resolution so the pipeline and the settings command agree on what "the
/// active preset" means.
async fn resolve_subject_preset(
    pool: &sqlx::SqlitePool,
) -> &'static crate::models::registry::FaceIdPreset {
    let value = crate::settings::repo::get_setting(pool, "subject_model")
        .await
        .ok()
        .flatten();
    crate::settings::commands::resolve_subject_preset(value.as_deref())
}

/// Ensure a preset's three models are downloaded and return its (cached or
/// freshly built) `FaceAnalyzer`. `VisionEngine::get_face_analyzer` already
/// caches by `preset.id` internally, so calling this repeatedly with the same
/// preset is cheap — only a preset change triggers a real rebuild.
async fn ensure_face_preset(
    app: &tauri::AppHandle,
    engine: &crate::vision::engine::VisionEngine,
    manager: &crate::models::ModelManager,
    preset: &'static crate::models::registry::FaceIdPreset,
) -> anyhow::Result<Arc<face_id::analyzer::FaceAnalyzer>> {
    for face_spec in [preset.detector, preset.embedder, preset.gender_age] {
        manager
            .ensure_ready(app, face_spec)
            .await
            .map_err(|e| anyhow::anyhow!("face model not ready ({}): {e}", face_spec.id))?;
    }
    engine.get_face_analyzer(manager, preset).await
}
```

Now, at the top of the `loop { ... }` body (right after `loop {` and before `// Pull both queues`, i.e. just before the existing `let sem_batch = ...` line), insert the per-iteration re-resolution:

```rust
    loop {
        // Per-batch preset resolution (§1 wiring fix): a mid-session
        // subject_model change takes effect on the next iteration with no
        // restart or signalling machinery. The analyzer is only rebuilt when
        // the resolved preset id actually differs from the one already loaded.
        let resolved_preset = resolve_subject_preset(&pool).await;
        if resolved_preset.id != subject_preset.id {
            match ensure_face_preset(&app, &engine, &manager, resolved_preset).await {
                Ok(analyzer) => {
                    face_tx = face_actor::spawn_face_actor(analyzer, config.infer_channel_depth);
                    subject_preset = resolved_preset;
                    info!("[pipeline] subject_model switched to '{}'", subject_preset.id);
                }
                Err(e) => {
                    error!(
                        "[pipeline] failed to switch subject preset to '{}', keeping '{}': {e}",
                        resolved_preset.id, subject_preset.id
                    );
                }
            }
        }

        // Pull both queues
        let sem_batch = match crate::pipeline::queue::get_queue_batch(
```

(The rest of the loop body is unchanged by this task — `preset` as a local variable no longer exists; `subject_preset` is the mutable outer-scope replacement. `face_tx` is now `mut` and reassigned above instead of being a `let`-bound constant from before the loop.)

- [ ] **Step 2: Run the crate build**

Run: `cd src-tauri && cargo build 2>&1 | head -80`
Expected: build errors pointing at the `save_faces` call sites (still using the old 5-arg signature and the Task 3/6 placeholder literal) and at the three clustering call sites inside the loop (still passing the old arity). These are fixed in Tasks 9 and 10 — do not attempt to fix them here; confirm the errors are confined to exactly those known sites (`save_faces(...)` calls around what is now line ~520/~600, and `cluster_unassigned_faces`/`update_edges_incremental`/`relabel_from_edges` calls around what is now line ~270/~660/~665) and stop.

- [ ] **Step 3: Commit**

This task intentionally leaves the crate non-building until Task 9 lands — commit anyway so the diff stays reviewable per-task, matching this plan's granularity; Tasks 9 and 10 restore a green build.

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "feat: resolve subject_model preset per pipeline batch instead of hardcoding Blitz"
```

---

### Task 9: Pipeline — `save_faces` uses `reprocess_image_faces`

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs` (`save_faces` function and its two call sites)

**Interfaces:**
- Consumes: `people::service::reprocess_image_faces`, `people::service::DetectedFaceInput` (Task 5), `people::repo::list_faces_for_image` (existing), `subject_preset` (Task 8, in scope at both call sites).

- [ ] **Step 1: Implement**

Replace the entire `save_faces` function (originally lines 78-135):

```rust
async fn save_faces(
    pool: &sqlx::SqlitePool,
    image_id: i64,
    sub_qid: i64,
    sub_attempts: i32,
    embedder_id: &str,
    faces: Vec<face_actor::FaceResult>,
) -> Vec<i64> {
    let detections: Vec<crate::people::service::DetectedFaceInput> = faces
        .into_iter()
        .map(|(detection, embedding, sharp)| {
            let bbox = detection.bbox;
            let rel = (
                bbox.x1 as f64,
                bbox.y1 as f64,
                (bbox.x2 - bbox.x1) as f64,
                (bbox.y2 - bbox.y1) as f64,
            );
            let frontality =
                crate::people::face_quality::frontality(detection.landmarks.as_deref());
            let quality = crate::people::face_quality::composite(detection.score, frontality, sharp);
            crate::people::service::DetectedFaceInput {
                bbox: rel,
                det_score: detection.score as f64,
                quality_score: quality as f64,
                embedding,
            }
        })
        .collect();

    let existing = match crate::people::repo::list_faces_for_image(pool, image_id).await {
        Ok(rows) => rows,
        Err(e) => {
            error!("[pipeline] list_faces_for_image failed for image {image_id}: {e}");
            let _ =
                crate::pipeline::queue::mark_failed(pool, sub_qid, sub_attempts, &e.to_string())
                    .await;
            return Vec::new();
        }
    };

    match crate::people::service::reprocess_image_faces(pool, image_id, embedder_id, detections, existing)
        .await
    {
        Ok(touched) => {
            let _ = crate::pipeline::queue::mark_subject_analysis_done(pool, sub_qid, image_id).await;
            touched
        }
        Err(e) => {
            error!("[pipeline] reprocess_image_faces failed for image {image_id}: {e}");
            let _ =
                crate::pipeline::queue::mark_failed(pool, sub_qid, sub_attempts, &e.to_string())
                    .await;
            Vec::new()
        }
    }
}
```

Update the two call sites (in the `(Some(erx), Some(frx))` branch and the `(None, Some(frx))` branch of the main loop, originally around lines 519 and 594) to pass `subject_preset.embedder.id`:

```rust
                                let new_ids = save_faces(
                                    &pool,
                                    image_id,
                                    sub_qid,
                                    sub_attempts,
                                    subject_preset.embedder.id,
                                    faces,
                                )
                                .await;
```

and

```rust
                            save_faces(
                                &pool,
                                image_id,
                                sub_qid,
                                sub_attempts,
                                subject_preset.embedder.id,
                                faces,
                            )
                            .await;
```

(matching each branch's existing surrounding structure — the first assigns to `new_ids` and extends `batch_new_face_ids`; the second discards the return value, unchanged from today.)

- [ ] **Step 2: Run the crate build**

Run: `cd src-tauri && cargo build 2>&1 | head -80`
Expected: remaining errors confined to the three clustering call sites inside the idle/incremental branches (fixed in Task 10).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "feat: route face persistence through reprocess_image_faces for id-stable reprocessing"
```

---

### Task 10: Pipeline — thread `embedder_id` into clustering calls

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs` (idle full-sweep branch and incremental-clustering branch)

**Interfaces:**
- Consumes: `people::clustering::{cluster_unassigned_faces, update_edges_incremental, relabel_from_edges}` (Task 6 signatures), `subject_preset` (Task 8, in scope).

- [ ] **Step 1: Implement**

Update the idle full-sweep call (originally lines 263-267):

```rust
                let result = crate::people::clustering::cluster_unassigned_faces(
                    &pool,
                    subject_preset.embedder.id,
                    Some(cancel_flag.as_ref()),
                )
                .await;
```

Update the incremental-clustering block (originally lines 652-663):

```rust
        if processed_subject_work {
            let incremental_result: anyhow::Result<()> = async {
                if !batch_new_face_ids.is_empty() {
                    crate::people::clustering::update_edges_incremental(
                        &pool,
                        &batch_new_face_ids,
                        subject_preset.embedder.id,
                    )
                    .await?;
                }
                // Constraints/assignments may have changed even with no new
                // vectors, so always relabel.
                crate::people::clustering::relabel_from_edges(&pool, subject_preset.embedder.id).await?;
                Ok(())
            }
            .await;
```

- [ ] **Step 2: Run the crate build and full test suite**

Run: `cd src-tauri && cargo build 2>&1 | tail -40`
Expected: PASS — clean build.

Run: `cd src-tauri && cargo test 2>&1 | tail -60`
Expected: PASS — full suite green.

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | tail -60`
Expected: no new warnings introduced by this plan (pre-existing warnings, if any, are out of scope).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "feat: scope clustering calls to the active subject preset's embedder_id"
```

---

### Task 11: Final verification and manual smoke test

**Files:** none (verification only)

- [ ] **Step 1: Full automated verification**

Run from `src-tauri/`:

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

Expected: all three succeed. If `clippy -D warnings` fails on pre-existing warnings unrelated to this plan's diff, re-run without `-D warnings` and confirm no *new* warnings appear in files touched by this plan (`db/mod.rs`, `db/tests.rs`, `people/bbox.rs`, `people/mod.rs`, `people/repo.rs`, `people/service.rs`, `people/clustering.rs`, `settings/commands.rs`, `pipeline/mod.rs`).

- [ ] **Step 2: Manual smoke test — wiring fix**

Per spec testing item 5 ("Pipeline test (or manual verification): changing subject_model mid-session causes the next batch to use the new preset's analyzer"):

1. Launch the app (`/run` skill or `pnpm tauri dev` from repo root) against a library with a handful of photos containing faces.
2. Let the subject pipeline finish an initial pass with the default preset (Blitz).
3. In Settings, switch the subject model to "Standard" (`precision`). Confirm the UI shows the model downloading if not already present, then the switch completes without a full people-data wipe (subject names and photo groupings from before the switch are still visible immediately after the switch, before reprocessing catches up).
4. Watch the app logs for `[pipeline] subject_model switched to 'precision'` and confirm no `[pipeline] face analyzer init failed` errors.
5. Wait for the re-queued images to finish the `'subject'` pipeline pass; confirm face crops/subject groupings still look correct and no duplicate subjects appear for people who were previously named.

- [ ] **Step 3: Manual smoke test — same-embedder no-op**

1. With the subject model already on "Standard", re-select "Standard" again (or toggle to a hypothetical same-embedder preset if one exists) via the settings UI.
2. Confirm no re-queueing occurs (`images.subject_analysis_done` stays `1` for already-processed images; check via a sqlite browser against the app's `nebula.db` if a UI signal isn't visible) — this exercises the `old_preset.embedder.id != preset.embedder.id` skip path from Task 7.

- [ ] **Step 4: Report results to the user**

Summarize: automated suite status, and outcome of both manual smoke tests (including any deviations from expected behavior). Flag the out-of-scope caveat from the Global Constraints section if the test library happens to be an install that had "Standard" selected under the pre-fix code.
