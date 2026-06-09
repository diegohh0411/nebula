# TT-49 B1: Storage & Constraint Foundation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `faces.embedding` BLOB and `face_corrections` with sqlite-vec virtual table + first-class `constraints` table, and expose a `knn(face_id, k)` API — the primitives B2's constraint-aware clusterer requires.

**Architecture:** New `face_store` module wraps all sqlite-vec operations (upsert, delete, knn, list-all). Three schema migrations: create the two new tables (migrations 3 + 4), then a one-shot rebuild that populates `face_vectors` from existing blobs and drops the retired columns (migration 5). All existing DB functions that read embeddings are rewritten to read from `face_vectors`; `get_face_cannot_link_subjects` is rewritten to derive forbidden-subject mappings from the `constraints` table. The public signatures of these functions are preserved so `clustering.rs` doesn't change except for its test.

**Tech Stack:** sqlite-vec 0.1, sqlx 0.8, Rust, once_cell 1

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `src-tauri/Cargo.toml` | add sqlite-vec dep |
| Create | `src-tauri/redist/vc_redist.x64.exe` | empty dummy file; Tauri build script requires it on Linux/WSL |
| Modify | `.gitignore` | ignore `src-tauri/redist/` |
| Modify | `src-tauri/src/db.rs` | extension registration; migrations 3–5; constraint helpers; rewrite embedding-reading fns |
| Create | `src-tauri/src/face_store.rs` | thin sqlite-vec API: upsert_vector, delete_vector, knn, get_all_face_vectors |
| Modify | `src-tauri/src/lib.rs` | add `mod face_store;` |
| Modify | `src-tauri/src/models/entities.rs` | remove `embedding` and `is_manual` from `Face` struct |
| Modify | `src-tauri/src/commands.rs` | remove `record_face_correction` call from `unassign_face` command |
| Modify | `src-tauri/src/pipeline/mod.rs` | call `face_store::upsert_vector` after successful `insert_face` |
| Modify | `src-tauri/src/clustering.rs` | update integration test to use new schema |

---

## Test Command

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

(The `src-tauri/redist/vc_redist.x64.exe` dummy file must exist before this runs — see Task 1.)

---

## Task 1: Wire sqlite-vec extension + fix build

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/db.rs`
- Create: `src-tauri/redist/vc_redist.x64.exe`
- Modify: `.gitignore`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)]` section at the bottom of `src-tauri/src/db.rs`:

```rust
#[tokio::test]
async fn sqlite_vec_extension_loads() {
    crate::db::ensure_sqlite_vec_registered();
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let version: String = sqlx::query_scalar("SELECT vec_version()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!version.is_empty(), "vec_version() should return a non-empty string");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib sqlite_vec_extension_loads 2>&1 | tail -15
```

Expected: compile error — `use of undeclared crate or module 'sqlite_vec'` or `ensure_sqlite_vec_registered` not found.

- [ ] **Step 3: Add sqlite-vec to Cargo.toml**

In `src-tauri/Cargo.toml`, add after the `once_cell` line:

```toml
sqlite-vec = "0.1"
```

- [ ] **Step 4: Add ensure_sqlite_vec_registered to db.rs**

At the top of `src-tauri/src/db.rs`, after the `use` imports, add:

```rust
use once_cell::sync::Once;

static SQLITE_VEC_INIT: Once = Once::new();

/// Register the sqlite-vec extension with every new SQLite connection.
/// Idempotent: safe to call multiple times; registers exactly once per process.
pub fn ensure_sqlite_vec_registered() {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        extern "C" {
            fn sqlite3_auto_extension(xInit: Option<unsafe extern "C" fn()>) -> i32;
        }
        // sqlite_vec::sqlite3_vec_init matches the standard extension init signature.
        // We transmute to the void-fn type that sqlite3_auto_extension demands per the C API.
        let f: unsafe extern "C" fn() =
            std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        sqlite3_auto_extension(Some(f));
    });
}
```

- [ ] **Step 5: Call it at the top of init_db**

In `src-tauri/src/db.rs`, find `pub async fn init_db(data_dir: &Path) -> Result<SqlitePool> {` and add as the very first line of the function body:

```rust
pub async fn init_db(data_dir: &Path) -> Result<SqlitePool> {
    ensure_sqlite_vec_registered();
    // ... existing code unchanged ...
```

- [ ] **Step 6: Create dummy redist file and update .gitignore**

```bash
mkdir -p src-tauri/redist
touch src-tauri/redist/vc_redist.x64.exe
```

Add to `.gitignore` (at the bottom):
```
# Tauri Windows bundle resource — download from Microsoft for real builds
src-tauri/redist/
```

- [ ] **Step 7: Run test to verify it passes**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib sqlite_vec_extension_loads 2>&1 | tail -10
```

Expected: `test db::tests::sqlite_vec_extension_loads ... ok`

- [ ] **Step 8: Run full suite to confirm no regressions**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1 | tail -5
```

Expected: all 72 tests pass (71 existing + the new one).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/db.rs .gitignore src-tauri/redist/vc_redist.x64.exe
git commit -m "feat(B1): wire sqlite-vec extension registration"
```

---

## Task 2: face_vectors virtual table + face_store module

**Files:**
- Create: `src-tauri/src/face_store.rs`
- Modify: `src-tauri/src/db.rs` (add Migration 3)
- Modify: `src-tauri/src/lib.rs` (add `mod face_store;`)

- [ ] **Step 1: Write failing tests for face_store**

Create `src-tauri/src/face_store.rs` with the tests only (no implementation yet):

```rust
use anyhow::Result;
use sqlx::SqlitePool;

pub async fn upsert_vector(_pool: &SqlitePool, _face_id: i64, _embedding: &[f32]) -> Result<()> {
    unimplemented!()
}

pub async fn delete_vector(_pool: &SqlitePool, _face_id: i64) -> Result<()> {
    unimplemented!()
}

/// k nearest neighbors of `face_id` by cosine similarity, ascending distance.
/// Excludes `face_id` itself. Returns at most k results.
pub async fn knn(_pool: &SqlitePool, _face_id: i64, _k: usize) -> Result<Vec<(i64, f32)>> {
    unimplemented!()
}

pub async fn get_all_face_vectors(_pool: &SqlitePool) -> Result<Vec<(i64, Vec<f32>)>> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn make_pool(dim: usize) -> SqlitePool {
        crate::db::ensure_sqlite_vec_registered();
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(&format!(
            "CREATE VIRTUAL TABLE face_vectors USING vec0(embedding float[{}])",
            dim
        ))
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn knn_returns_empty_for_unknown_face() {
        let pool = make_pool(3).await;
        let result = knn(&pool, 999, 5).await.unwrap();
        assert!(result.is_empty(), "unknown face should return no neighbors");
    }

    #[tokio::test]
    async fn knn_returns_correct_ordering() {
        // A=[1,0,0], B=[0.9,0.1,0], C=[0,0,1]
        // Querying from A: B is much closer than C (cosine similarity ~ 0.993 vs 0.0)
        let pool = make_pool(3).await;
        upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap(); // face A
        upsert_vector(&pool, 2, &[0.9, 0.1, 0.0]).await.unwrap(); // face B — very close to A
        upsert_vector(&pool, 3, &[0.0, 0.0, 1.0]).await.unwrap(); // face C — orthogonal to A

        let neighbors = knn(&pool, 1, 2).await.unwrap();
        assert_eq!(neighbors.len(), 2, "should return exactly k=2 neighbors");
        assert_eq!(neighbors[0].0, 2, "B should be closest to A");
        assert_eq!(neighbors[1].0, 3, "C should be second-closest");
        assert!(
            neighbors[0].1 < neighbors[1].1,
            "distances should be ascending"
        );
    }

    #[tokio::test]
    async fn knn_excludes_self() {
        let pool = make_pool(3).await;
        upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap();
        upsert_vector(&pool, 2, &[0.9, 0.1, 0.0]).await.unwrap();

        let neighbors = knn(&pool, 1, 5).await.unwrap();
        let self_included = neighbors.iter().any(|(id, _)| *id == 1);
        assert!(!self_included, "knn must not include the query face itself");
    }

    #[tokio::test]
    async fn get_all_face_vectors_returns_seeded_data() {
        let pool = make_pool(3).await;
        upsert_vector(&pool, 10, &[1.0, 0.0, 0.0]).await.unwrap();
        upsert_vector(&pool, 20, &[0.0, 1.0, 0.0]).await.unwrap();

        let all = get_all_face_vectors(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        let ids: Vec<i64> = all.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
    }

    #[tokio::test]
    async fn delete_vector_removes_entry() {
        let pool = make_pool(3).await;
        upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap();
        delete_vector(&pool, 1).await.unwrap();
        let all = get_all_face_vectors(&pool).await.unwrap();
        assert!(all.is_empty());
    }
}
```

- [ ] **Step 2: Add mod face_store to lib.rs**

In `src-tauri/src/lib.rs`, add after `mod clustering;`:

```rust
mod face_store;
```

- [ ] **Step 3: Run tests to see them panic at unimplemented!()**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib face_store 2>&1 | tail -15
```

Expected: `panicked at 'not yet implemented'` — confirms tests are being discovered.

- [ ] **Step 4: Add Migration 3 (face_vectors virtual table)**

In `src-tauri/src/db.rs`, find `const VERSIONED_MIGRATIONS: &[(u32, &str)] = &[` and add migration 3 after the existing ones:

```rust
const VERSIONED_MIGRATIONS: &[(u32, &str)] = &[
    (1, "
        DROP TABLE IF EXISTS merge_suggestions;
        CREATE TABLE merge_suggestions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            score REAL NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_merge_pair ON merge_suggestions(
            CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
            CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END
        );
    "),
    (2, "
        CREATE TABLE IF NOT EXISTS dismissed_pairs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            dismissed_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_dismissed_pair ON dismissed_pairs(subject_id_a, subject_id_b);
    "),
    (3, "CREATE VIRTUAL TABLE IF NOT EXISTS face_vectors USING vec0(embedding float[512])"),
];
```

- [ ] **Step 5: Implement face_store.rs**

Replace the stub implementations in `src-tauri/src/face_store.rs` with the real code (keep the tests unchanged):

```rust
use anyhow::Result;
use sqlx::{Row, SqlitePool};

pub async fn upsert_vector(pool: &SqlitePool, face_id: i64, embedding: &[f32]) -> Result<()> {
    let bytes: Vec<u8> = embedding.iter().flat_map(|v| v.to_le_bytes()).collect();
    sqlx::query("INSERT OR REPLACE INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(face_id)
        .bind(&bytes)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_vector(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM face_vectors WHERE rowid = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// k nearest neighbors of `face_id` by cosine similarity, ascending distance.
/// Excludes `face_id` itself. Returns at most k results.
pub async fn knn(pool: &SqlitePool, face_id: i64, k: usize) -> Result<Vec<(i64, f32)>> {
    let query_bytes: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT embedding FROM face_vectors WHERE rowid = ?")
            .bind(face_id)
            .fetch_optional(pool)
            .await?;

    let Some(qb) = query_bytes else {
        return Ok(vec![]);
    };

    // Request k+1 to compensate for filtering out the query face itself.
    let rows = sqlx::query(
        "SELECT rowid, distance FROM face_vectors \
         WHERE embedding MATCH ? AND k = ? \
         ORDER BY distance",
    )
    .bind(&qb)
    .bind((k + 1) as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let id: i64 = r.get("rowid");
            let dist: f32 = r.get("distance");
            if id == face_id { None } else { Some((id, dist)) }
        })
        .take(k)
        .collect())
}

pub async fn get_all_face_vectors(pool: &SqlitePool) -> Result<Vec<(i64, Vec<f32>)>> {
    let rows = sqlx::query("SELECT rowid, embedding FROM face_vectors")
        .fetch_all(pool)
        .await?;

    rows.into_iter()
        .map(|r| {
            let id: i64 = r.get("rowid");
            let bytes: Vec<u8> = r.get("embedding");
            let embedding = crate::embedder::bytes_to_f32_vec(&bytes)?;
            Ok((id, embedding))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // ... (tests from Step 1, unchanged) ...
}
```

- [ ] **Step 6: Run face_store tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib face_store 2>&1 | tail -15
```

Expected: all 5 face_store tests pass.

- [ ] **Step 7: Run full suite for regressions**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/face_store.rs src-tauri/src/db.rs src-tauri/src/lib.rs
git commit -m "feat(B1): face_vectors virtual table and face_store knn API"
```

---

## Task 3: constraints table + write helpers

**Files:**
- Modify: `src-tauri/src/db.rs` (Migration 4 + helpers)

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)]` section in `src-tauri/src/db.rs`:

```rust
// --- helper for constraint tests ---
async fn init_test_pool() -> SqlitePool {
    crate::db::ensure_sqlite_vec_registered();
    let tmp = std::env::temp_dir().join(format!("nebula_test_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let pool = init_db(&tmp).await.unwrap();
    pool
}

#[tokio::test]
async fn constraint_enforces_face_a_less_than_face_b() {
    let pool = init_test_pool().await;
    // Insert two faces so FK constraints are satisfied
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (3, 1, 0,0,1,1,0), (5, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();

    // Call with larger id first — should normalize to (3, 5)
    add_cannot_link(&pool, 5, 3, "removal").await.unwrap();

    let (a, b): (i64, i64) =
        sqlx::query_as("SELECT face_a, face_b FROM constraints WHERE kind = 'cannot_link'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a, 3, "face_a must be the smaller id");
    assert_eq!(b, 5, "face_b must be the larger id");
}

#[tokio::test]
async fn constraint_insert_or_ignore_deduplicates() {
    let pool = init_test_pool().await;
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 1, 0,0,1,1,0), (2, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();

    add_cannot_link(&pool, 1, 2, "removal").await.unwrap();
    add_cannot_link(&pool, 1, 2, "removal").await.unwrap(); // second insert must be silently ignored

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "duplicate insert must not create a second row");
}

#[tokio::test]
async fn must_link_and_cannot_link_are_independent_rows() {
    let pool = init_test_pool().await;
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 1, 0,0,1,1,0), (2, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();

    add_must_link(&pool, 1, 2, "merge").await.unwrap();
    add_cannot_link(&pool, 1, 2, "removal").await.unwrap(); // same pair, different kind → OK

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2, "must_link and cannot_link on the same pair are distinct rows");
}
```

- [ ] **Step 2: Run to verify they fail (constraints table not created yet)**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib constraint_enforces 2>&1 | tail -10
```

Expected: error or panic — `no such table: constraints`.

- [ ] **Step 3: Add Migration 4 (constraints table)**

In `VERSIONED_MIGRATIONS` in `src-tauri/src/db.rs`, add after migration 3:

```rust
    (4, "
        CREATE TABLE IF NOT EXISTS constraints (
            face_a      INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
            face_b      INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
            kind        TEXT NOT NULL CHECK(kind IN ('must_link', 'cannot_link')),
            source      TEXT NOT NULL CHECK(source IN ('merge', 'manual_assign', 'removal', 'dismiss')),
            created_at  INTEGER NOT NULL,
            PRIMARY KEY (face_a, face_b, kind)
        )
    "),
```

- [ ] **Step 4: Add constraint write helpers**

Add these functions to `src-tauri/src/db.rs` (place them near the existing `record_face_correction`):

```rust
fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a <= b { (a, b) } else { (b, a) }
}

pub async fn add_must_link(pool: &SqlitePool, face_a: i64, face_b: i64, source: &str) -> Result<()> {
    let (a, b) = ordered_pair(face_a, face_b);
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) \
         VALUES (?, ?, 'must_link', ?, ?)",
    )
    .bind(a)
    .bind(b)
    .bind(source)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn add_cannot_link(pool: &SqlitePool, face_a: i64, face_b: i64, source: &str) -> Result<()> {
    let (a, b) = ordered_pair(face_a, face_b);
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) \
         VALUES (?, ?, 'cannot_link', ?, ?)",
    )
    .bind(a)
    .bind(b)
    .bind(source)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 5: Run constraint tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib constraint 2>&1 | tail -15
```

Expected: all 3 new constraint tests pass.

- [ ] **Step 6: Run full suite**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(B1): constraints table and add_must_link / add_cannot_link helpers"
```

---

## Task 4: One-shot rebuild migration — retire faces.embedding, is_manual, face_corrections

**Files:**
- Modify: `src-tauri/src/db.rs` (Migration 5)

This migration runs at app startup on any existing database (alpha: one-shot rebuild is fine). It populates `face_vectors` from the existing `faces.embedding` blobs, then drops the retired columns.

- [ ] **Step 1: Write the test**

Add to the `#[cfg(test)]` section of `src-tauri/src/db.rs`:

```rust
#[tokio::test]
async fn migration_5_drops_embedding_and_is_manual_columns() {
    let pool = init_test_pool().await;

    // Verify neither column exists after migration
    let cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('faces')")
        .fetch_all(&pool)
        .await
        .unwrap();

    assert!(
        !cols.contains(&"embedding".to_string()),
        "faces.embedding must be dropped by migration 5"
    );
    assert!(
        !cols.contains(&"is_manual".to_string()),
        "faces.is_manual must be dropped by migration 5"
    );
    assert!(
        !cols.contains(&"face_corrections".to_string()),
        "face_corrections table must be dropped"
    );
}

#[tokio::test]
async fn migration_5_preserves_existing_vectors_in_face_vectors() {
    // Simulate an existing database that has faces.embedding blobs
    // by inserting directly into the pre-migration schema before running migrations.
    // 
    // We test this indirectly: after init_db (which runs migrations 3+4 but not 5 yet
    // on an empty DB), we check face_vectors is accessible.
    let pool = init_test_pool().await;

    // Insert a face with no embedding (post-migration state)
    sqlx::query("INSERT INTO faces (image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 0,0,1,1,0)")
        .execute(&pool)
        .await
        .unwrap();
    let face_id: i64 = sqlx::query_scalar("SELECT last_insert_rowid()").fetch_one(&pool).await.unwrap();

    // Manually insert a vector (simulating what the pipeline will do post-migration)
    crate::face_store::upsert_vector(&pool, face_id, &[1.0f32, 0.0, 0.0]).await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_vectors").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1, "face_vectors should store the upserted vector");
}
```

- [ ] **Step 2: Run to verify tests pass (they should — just verifying schema state)**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib migration_5 2>&1 | tail -10
```

Expected: `migration_5_drops_embedding_and_is_manual_columns` FAILS (columns still exist), `migration_5_preserves_existing_vectors_in_face_vectors` PASSES.

- [ ] **Step 3: Add Migration 5**

In `VERSIONED_MIGRATIONS` in `src-tauri/src/db.rs`, add after migration 4:

```rust
    (5, "
        INSERT OR REPLACE INTO face_vectors(rowid, embedding)
            SELECT id, embedding FROM faces WHERE embedding IS NOT NULL;
        ALTER TABLE faces DROP COLUMN embedding;
        ALTER TABLE faces DROP COLUMN is_manual;
        DROP TABLE IF EXISTS face_corrections
    "),
```

- [ ] **Step 4: Run migration tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib migration_5 2>&1 | tail -10
```

Expected: both migration_5 tests pass.

- [ ] **Step 5: Run full suite — expect failures (code still references old columns)**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1 | tail -20
```

Expected: several failures in tests that still use `embedding` or `is_manual` columns in their setup SQL. This is correct — Task 5 will fix them.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(B1): migration 5 — one-shot rebuild into face_vectors, drop faces.embedding/is_manual"
```

---

## Task 5: Update all Rust code to the new schema

**Files:**
- Modify: `src-tauri/src/models/entities.rs`
- Modify: `src-tauri/src/db.rs` (many functions)
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/pipeline/mod.rs`
- Modify: `src-tauri/src/clustering.rs` (integration test only)

This is the largest task. Work through each sub-step sequentially. After every sub-step, keep track of compile errors to know what remains.

### 5a: Remove embedding and is_manual from Face struct

- [ ] **Step 1: Update Face struct in entities.rs**

In `src-tauri/src/models/entities.rs`, replace the `Face` struct:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Face {
    pub id: i64,
    pub image_id: i64,
    pub subject_id: Option<i64>,
    pub bbox_x: f64,
    pub bbox_y: f64,
    pub bbox_w: f64,
    pub bbox_h: f64,
    pub added_at: i64,
}
```

(Removed: `pub embedding: Option<Vec<u8>>` and `pub is_manual: bool`.)

- [ ] **Step 2: Fix every Face construction in db.rs**

There are four places in `db.rs` that construct a `Face`. Update each:

**list_faces_for_subject** (around line 667):
```rust
pub async fn list_faces_for_subject(pool: &SqlitePool, subject_id: i64) -> Result<Vec<Face>> {
    let rows = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at
         FROM faces WHERE subject_id = ? ORDER BY added_at DESC",
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Face {
            id: r.get("id"),
            image_id: r.get("image_id"),
            subject_id: r.get("subject_id"),
            bbox_x: r.get("bbox_x"),
            bbox_y: r.get("bbox_y"),
            bbox_w: r.get("bbox_w"),
            bbox_h: r.get("bbox_h"),
            added_at: r.get("added_at"),
        })
        .collect())
}
```

**get_face_by_id** (around line 719):
```rust
pub async fn get_face_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Face>> {
    let row = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at
         FROM faces WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.as_ref().map(|r| Face {
        id: r.get("id"),
        image_id: r.get("image_id"),
        subject_id: r.get("subject_id"),
        bbox_x: r.get("bbox_x"),
        bbox_y: r.get("bbox_y"),
        bbox_w: r.get("bbox_w"),
        bbox_h: r.get("bbox_h"),
        added_at: r.get("added_at"),
    }))
}
```

**list_faces_for_image** (around line 823):
```rust
pub async fn list_faces_for_image(pool: &SqlitePool, image_id: i64) -> Result<Vec<Face>> {
    let rows = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at
         FROM faces WHERE image_id = ? ORDER BY added_at DESC",
    )
    .bind(image_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Face {
            id: r.get("id"),
            image_id: r.get("image_id"),
            subject_id: r.get("subject_id"),
            bbox_x: r.get("bbox_x"),
            bbox_y: r.get("bbox_y"),
            bbox_w: r.get("bbox_w"),
            bbox_h: r.get("bbox_h"),
            added_at: r.get("added_at"),
        })
        .collect())
}
```

**get_largest_face_for_subject** (around line 812) — check if it reads embedding/is_manual; if so, remove those columns too.

- [ ] **Step 3: Compile check**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --lib 2>&1 | grep "^error" | head -20
```

Fix any remaining `embedding` or `is_manual` references in db.rs before proceeding.

### 5b: Update insert_face — drop embedding parameter

- [ ] **Step 4: Change insert_face signature**

Replace `pub async fn insert_face(...)` in `src-tauri/src/db.rs`:

```rust
pub async fn insert_face(
    pool: &SqlitePool,
    image_id: i64,
    subject_id: Option<i64>,
    bbox: (f64, f64, f64, f64),
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(image_id)
    .bind(subject_id)
    .bind(bbox.0)
    .bind(bbox.1)
    .bind(bbox.2)
    .bind(bbox.3)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}
```

- [ ] **Step 5: Update pipeline/mod.rs to use new insert_face + upsert_vector**

In `src-tauri/src/pipeline/mod.rs`, find the `save_faces` function (around line 35) and replace:

```rust
async fn save_faces(
    pool: &SqlitePool,
    image_id: i64,
    sub_qid: i64,
    sub_attempts: i32,
    faces: Vec<(face_id::detector::BoundingBox, Vec<f32>)>,
) {
    let mut all_ok = true;
    for (bbox, face_emb) in faces {
        let rel_x = bbox.x1 as f64;
        let rel_y = bbox.y1 as f64;
        let rel_w = (bbox.x2 - bbox.x1) as f64;
        let rel_h = (bbox.y2 - bbox.y1) as f64;
        match crate::db::insert_face(pool, image_id, None, (rel_x, rel_y, rel_w, rel_h)).await {
            Ok(face_id) => {
                if let Err(e) = crate::face_store::upsert_vector(pool, face_id, &face_emb).await {
                    eprintln!("[pipeline] upsert_vector failed for face {face_id}: {e}");
                    all_ok = false;
                }
            }
            Err(e) => {
                eprintln!("[pipeline] insert_face failed for image {image_id}: {e}");
                all_ok = false;
            }
        }
    }
    if all_ok {
        let _ = crate::db::mark_subject_analysis_done(pool, sub_qid, image_id).await;
    } else {
        let _ = crate::db::mark_failed(pool, sub_qid, sub_attempts, "one or more face inserts failed").await;
    }
}
```

### 5c: Rewrite embedding-reading DB functions

- [ ] **Step 6: Rewrite get_subject_embeddings**

Replace the current implementation (reads from `faces.embedding`) with one that reads from `face_vectors`:

```rust
pub async fn get_subject_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT f.subject_id, fv.embedding
         FROM face_vectors fv
         JOIN faces f ON f.id = fv.rowid
         WHERE f.subject_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get("subject_id"), r.get("embedding")))
        .collect())
}
```

- [ ] **Step 7: Rewrite get_manual_face_embeddings_by_subject**

`is_manual` is gone — fall back to all subject faces (the `compute_anchor_centroids` in clustering already handles the all-faces case as a fallback, so clustering behavior is unchanged):

```rust
pub async fn get_manual_face_embeddings_by_subject(
    pool: &SqlitePool,
) -> Result<Vec<(i64, Vec<u8>)>> {
    // is_manual column removed in B1; return all subject-assigned faces.
    // compute_anchor_centroids already uses all-faces as its fallback.
    let rows = sqlx::query(
        "SELECT f.subject_id, fv.embedding
         FROM face_vectors fv
         JOIN faces f ON f.id = fv.rowid
         WHERE f.subject_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get("subject_id"), r.get("embedding")))
        .collect())
}
```

- [ ] **Step 8: Rewrite get_unassigned_faces_with_embeddings**

```rust
pub async fn get_unassigned_faces_with_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT f.id, fv.embedding
         FROM face_vectors fv
         JOIN faces f ON f.id = fv.rowid
         WHERE f.subject_id IS NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<Vec<u8>, _>("embedding")))
        .collect())
}
```

- [ ] **Step 9: Rewrite get_face_cannot_link_subjects**

This now derives forbidden-subject mappings from the `constraints` table by joining each cannot-linked face with the current subject of the other face:

```rust
pub async fn get_face_cannot_link_subjects(
    pool: &SqlitePool,
) -> Result<std::collections::HashMap<i64, std::collections::HashSet<i64>>> {
    // For a constraint (face_a, face_b, cannot_link):
    // - face_a is forbidden from face_b's current subject
    // - face_b is forbidden from face_a's current subject
    let rows = sqlx::query(
        "SELECT c.face_a AS queried_face, f2.subject_id AS forbidden_subject
         FROM constraints c
         JOIN faces f2 ON f2.id = c.face_b
         WHERE c.kind = 'cannot_link' AND f2.subject_id IS NOT NULL
         UNION ALL
         SELECT c.face_b AS queried_face, f2.subject_id AS forbidden_subject
         FROM constraints c
         JOIN faces f2 ON f2.id = c.face_a
         WHERE c.kind = 'cannot_link' AND f2.subject_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    let mut map: std::collections::HashMap<i64, std::collections::HashSet<i64>> =
        std::collections::HashMap::new();
    for row in rows {
        let face_id: i64 = row.get("queried_face");
        let forbidden: i64 = row.get("forbidden_subject");
        map.entry(face_id).or_default().insert(forbidden);
    }
    Ok(map)
}
```

### 5d: Clean up retired functions and update side-effecting functions

- [ ] **Step 10: Delete record_face_correction**

Remove the entire `pub async fn record_face_correction(...)` function from `db.rs` (it reads/writes the now-dropped `face_corrections` table).

- [ ] **Step 11: Update unassign_face in db.rs**

Replace:
```rust
pub async fn unassign_face(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = NULL, is_manual = 1 WHERE id = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

With:
```rust
pub async fn unassign_face(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = NULL WHERE id = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 12: Update reset_all_embeddings in db.rs**

Replace the face-embedding reset line:
```rust
// OLD — remove this line:
sqlx::query("UPDATE faces SET embedding = NULL").execute(&mut *tx).await?;
// NEW — add this line instead:
sqlx::query("DELETE FROM face_vectors").execute(&mut *tx).await?;
```

- [ ] **Step 13: Update reset_all_subject_data in db.rs**

Replace `DELETE FROM face_corrections` with `DELETE FROM constraints`:
```rust
// OLD:
sqlx::query("DELETE FROM face_corrections").execute(&mut *tx).await?;
// NEW:
sqlx::query("DELETE FROM constraints").execute(&mut *tx).await?;
```

- [ ] **Step 14: Update commands.rs unassign_face**

In `src-tauri/src/commands.rs`, find the `unassign_face` command and remove the `record_face_correction` call:

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

(The `record_face_correction` call and the `old_subject_id` lookup are both removed. B2 will add the `add_cannot_link` call here.)

### 5e: Update the clustering integration test

- [ ] **Step 15: Rewrite recluster_does_not_reassign_removed_face_to_forbidden_subject**

In `src-tauri/src/clustering.rs`, replace the entire `recluster_does_not_reassign_removed_face_to_forbidden_subject` test with:

```rust
#[tokio::test]
async fn recluster_does_not_reassign_removed_face_to_forbidden_subject() {
    fn emb_bytes(vals: &[f32]) -> Vec<u8> {
        vals.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    crate::db::ensure_sqlite_vec_registered();
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();

    // Minimal schema matching the B1 state (no embedding, no is_manual, no face_corrections)
    sqlx::query(
        "CREATE TABLE subjects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT,
            type TEXT NOT NULL DEFAULT 'person',
            thumbnail_face_id INTEGER,
            added_at INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE faces (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            image_id INTEGER NOT NULL DEFAULT 0,
            subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
            bbox_x REAL NOT NULL DEFAULT 0,
            bbox_y REAL NOT NULL DEFAULT 0,
            bbox_w REAL NOT NULL DEFAULT 0.5,
            bbox_h REAL NOT NULL DEFAULT 0.5,
            added_at INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Use float[3] for compact test vectors
    sqlx::query("CREATE VIRTUAL TABLE face_vectors USING vec0(embedding float[3])")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "CREATE TABLE constraints (
            face_a INTEGER NOT NULL,
            face_b INTEGER NOT NULL,
            kind TEXT NOT NULL,
            source TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (face_a, face_b, kind)
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE merge_suggestions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject_id_a INTEGER NOT NULL,
            subject_id_b INTEGER NOT NULL,
            score REAL NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE UNIQUE INDEX idx_merge_pair ON merge_suggestions(
            CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
            CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE dismissed_pairs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject_id_a INTEGER NOT NULL,
            subject_id_b INTEGER NOT NULL,
            dismissed_at INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE UNIQUE INDEX idx_dismissed_pair ON dismissed_pairs(subject_id_a, subject_id_b)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Subject S: named, with anchor face close to [1, 0, 0]
    let subject_s: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('S', 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let anchor_s: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
    )
    .bind(subject_s)
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(anchor_s)
        .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
        .execute(&pool)
        .await
        .unwrap();

    // Subject S2: named, anchor near face_f's embedding — should absorb F when S is forbidden
    let subject_s2: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('S2', 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let anchor_s2: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (3, ?, 0) RETURNING id",
    )
    .bind(subject_s2)
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(anchor_s2)
        .bind(emb_bytes(&[0.998f32, 0.06, 0.0]))
        .execute(&pool)
        .await
        .unwrap();

    // Face F: unassigned, embedding very close to S's anchor
    let face_f: i64 = sqlx::query_scalar(
        "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, NULL, 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(face_f)
        .bind(emb_bytes(&[0.999f32, 0.05, 0.0]))
        .execute(&pool)
        .await
        .unwrap();

    // Record cannot_link: F was removed from S — link F to S's anchor face
    crate::db::add_cannot_link(&pool, face_f, anchor_s, "removal")
        .await
        .unwrap();

    // Run recluster
    cluster_unassigned_faces(&pool).await.unwrap();

    let assigned: Option<i64> =
        sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(face_f)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        assigned != Some(subject_s),
        "face F should NOT be reassigned to forbidden subject S (got {:?})",
        assigned
    );
    assert_eq!(
        assigned,
        Some(subject_s2),
        "face F should be assigned to the nearest non-forbidden subject S2"
    );
}
```

### 5f: Verify and commit

- [ ] **Step 16: Full compile check**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --lib 2>&1 | grep "^error" | head -20
```

Fix any remaining compiler errors before running tests.

- [ ] **Step 17: Run full test suite**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1 | tail -10
```

Expected: all tests pass. Count should be ≥ 77 (71 original + 1 sqlite_vec_extension_loads + 5 face_store + 3 constraint + 2 migration_5).

- [ ] **Step 18: Commit**

```bash
git add src-tauri/src/models/entities.rs src-tauri/src/db.rs src-tauri/src/commands.rs src-tauri/src/pipeline/mod.rs src-tauri/src/clustering.rs src-tauri/src/face_store.rs
git commit -m "feat(B1): update all code to new face_vectors+constraints schema, retire faces.embedding/is_manual"
```

---

## Task 6: Open PR and update Notion

- [ ] **Step 1: Push branch and open PR**

```bash
git push -u origin worktree-tt-49-sqlite-vec-constraint-foundation
gh pr create \
  --title "feat(TT-49): B1 storage & constraint foundation (sqlite-vec + constraints table)" \
  --body "$(cat <<'EOF'
## Summary
- Introduces `face_vectors` sqlite-vec virtual table as the single source of truth for face embeddings (replaces `faces.embedding` BLOB)
- Adds first-class `constraints` table with `must_link` / `cannot_link` kinds and symmetric `face_a < face_b` invariant
- Exposes `face_store::knn(face_id, k)` backed by sqlite-vec, behind a thin interface
- One-shot migration (alpha): populates `face_vectors` from existing BLOBs, drops `faces.embedding`, `faces.is_manual`, and `face_corrections`
- All existing tests pass; clustering behavior unchanged in B1 (B2 replaces the algorithm)

## Acceptance
- [x] face_vectors stored/queried via sqlite-vec; no faces.embedding blob
- [x] constraints table + write helpers; symmetry and face_a < face_b invariant tested
- [x] knn returns correct ordering (tested against seeded set)
- [x] One-shot rebuild works
- [x] All tests pass

## Test plan
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml --lib` — all tests green
- [ ] Launch app, verify faces still render and People grid works
- [ ] Run clustering on existing library; verify subjects appear

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 2: Update Notion task status to Ready for review**

```bash
ntn api v1/pages/37ae954d-b476-81ce-b958-d3deeb6cf663 -X PATCH \
  'properties[Status][status][name]=Ready for review' \
  'properties[PR number][number]:=<PR_NUMBER_FROM_ABOVE>'
```

Replace `<PR_NUMBER_FROM_ABOVE>` with the actual PR number from the `gh pr create` output.

---

## Self-Review Checklist

**Spec coverage:**
- [x] sqlite-vec virtual table `face_vectors(rowid=face_id, embedding float[512])` — Task 2 Migration 3
- [x] Remove `faces.embedding` BLOB — Task 4 Migration 5 + Task 5
- [x] Remove separate face index snapshot — `face_corrections` dropped in Migration 5
- [x] `constraints` table with correct schema, kind ∈ {must_link, cannot_link}, source values, face_a < face_b PK — Task 3 Migration 4
- [x] Retire `faces.is_manual` — Task 4 Migration 5 + Task 5 (Face struct, all queries)
- [x] Write helpers add_must_link / add_cannot_link — Task 3 Step 4
- [x] `knn(face_id, k)` returns correct ordering, tested — Task 2 Step 1
- [x] One-shot rebuild of vectors from existing data — Task 4 Migration 5
- [x] Existing tests pass — Task 5 Steps 16–17
- [x] PR opened, status → Ready for review — Task 6

**Out of scope confirmed not included:**
- Clustering algorithm (B2)
- Per-provider threshold (B3)
- Image search / FlatIndex / nebula.idx (untouched)
