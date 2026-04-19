# Indexer Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Overhaul the file indexing system to eliminate startup re-hashing, fix the re-analysis bug, add FS event debouncing, centralize indexing logic in a single module, and replace the fragile migration system with a versioned one.

**Architecture:** A new `indexer.rs` module owns all indexing logic — startup rescan, live FS event processing, and folder management. `watcher.rs` becomes a thin adapter that receives raw `notify` events and runs a debounce loop before forwarding batches to the indexer. `AppState` replaces `watcher: Arc<Mutex<FolderWatcher>>` with `indexer: Arc<Indexer>`. The migration system moves from ad-hoc `CREATE TABLE IF NOT EXISTS` + silently-swallowed `POST_MIGRATIONS` to a versioned system with a `schema_version` table.

**Tech Stack:** Rust, Tauri 2, SQLite (sqlx), `notify` 6 (FS events), `rayon` (parallel stat), `sha2` (hashing).

**Note:** This is a multi-file refactor. The project will not compile between individual tasks. A final `cargo check` verification step is included after all tasks are complete. No test infrastructure exists in this project; verification is via compilation.

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src-tauri/Cargo.toml` | Modify | Add `rayon` dependency |
| `src-tauri/src/models.rs` | Modify | Rename `date_file`→`mtime`, add `file_size`, add new event types |
| `src-tauri/src/db.rs` | Modify | Rewrite schema, versioned migrations, update all queries, add new indexer queries |
| `src-tauri/src/indexer.rs` | **Create** | All indexing logic: process_file, start_rescan, folder management, event handling |
| `src-tauri/src/watcher.rs` | Modify | Strip to thin FS adapter + debounce loop |
| `src-tauri/src/lib.rs` | Modify | Wire indexer, simplify startup |
| `src-tauri/src/commands.rs` | Modify | Delegate add_folder/remove_folder to indexer |
| `src-tauri/src/search.rs` | Modify | Rename `date_file`→`mtime` reference |

---

### Task 1: Add rayon Dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add rayon to dependencies**

Add `rayon = "1.10"` to the `[dependencies]` section in `src-tauri/Cargo.toml`, after the `once_cell` line:

```toml
rayon = "1.10"
```

- [ ] **Step 2: Verify dependency resolves**

Run: `cargo check` in `src-tauri/`
Expected: Success (no code uses rayon yet)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml
git commit -m "chore: add rayon dependency for parallel stat walk"
```

---

### Task 2: Update models.rs

**Files:**
- Modify: `src-tauri/src/models.rs`

This task renames `date_file` → `mtime` across all model structs, adds `file_size` to `Image`, and adds new types for the indexer and debounce system.

- [ ] **Step 1: Rename `date_file` → `mtime` in `Image` struct and add `file_size`**

Replace the `Image` struct (lines 10–24) with:

```rust
pub struct Image {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub file_hash: String,
    pub file_size: i64,
    pub date_taken: Option<i64>,
    pub mtime: i64,
    pub thumbnail_path: Option<String>,
    pub semantic_analysis_done: bool,
    pub subject_analysis_done: bool,
    pub added_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}
```

- [ ] **Step 2: Rename `date_file` → `mtime` in `SearchResult` struct**

Replace `SearchResult` (lines 77–87) with:

```rust
pub struct SearchResult {
    pub image_id: i64,
    pub path: String,
    pub thumbnail_path: Option<String>,
    pub score: f32,
    pub date_taken: Option<i64>,
    pub mtime: i64,
    pub semantic_analysis_done: bool,
    pub subject_analysis_done: bool,
}
```

- [ ] **Step 3: Add new event payload types and debounce types after `ImageRemovedPayload`**

After the `ImageRemovedPayload` struct (around line 72), add:

```rust
#[derive(Clone, serde::Serialize)]
pub struct SyncProgressPayload {
    pub done: u32,
    pub total: u32,
}

#[derive(Clone, serde::Serialize)]
pub struct SyncCompletePayload {}

#[derive(Clone, Debug)]
pub enum DebouncedEventKind {
    Create,
    Modify,
    Remove,
}

#[derive(Clone, Debug)]
pub struct DebouncedEvent {
    pub path: std::path::PathBuf,
    pub kind: DebouncedEventKind,
}
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/models.rs
git commit -m "refactor: rename date_file to mtime, add file_size and indexer types"
```

---

### Task 3: Rewrite db.rs — Schema, Migrations, and All Queries

**Files:**
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/search.rs`
- Modify: `src-tauri/src/commands.rs`

This is the largest single task. It rewrites the schema, migration system, updates all queries for `mtime`/`file_size`, and adds new queries needed by the indexer.

**Important:** Read the current `src-tauri/src/db.rs` in full before starting. All function signatures from line 139 onward that aren't explicitly changed here should be kept as-is (with `date_file`→`mtime` in SQL strings).

#### Part A: Replace Schema Constants

- [ ] **Step 1: Replace the `MIGRATIONS` and `POST_MIGRATIONS` constants with `BASE_SCHEMA` and `VERSIONED_MIGRATIONS`**

Delete the `MIGRATIONS` constant (lines 7–91) and `POST_MIGRATIONS` constant (lines 93–105). Replace with:

```rust
const LATEST_VERSION: u32 = 3;

const BASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    path     TEXT UNIQUE NOT NULL,
    added_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS images (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_id              INTEGER NOT NULL REFERENCES folders(id),
    path                   TEXT UNIQUE NOT NULL,
    file_hash              TEXT NOT NULL,
    file_size              INTEGER NOT NULL DEFAULT 0,
    date_taken             INTEGER,
    mtime                  INTEGER NOT NULL,
    thumbnail_path         TEXT,
    semantic_analysis_done INTEGER NOT NULL DEFAULT 0,
    subject_analysis_done  INTEGER NOT NULL DEFAULT 0,
    embedding              BLOB,
    added_at               INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    deleted_at             INTEGER
);

CREATE INDEX IF NOT EXISTS idx_images_folder   ON images(folder_id);
CREATE INDEX IF NOT EXISTS idx_images_semantic ON images(semantic_analysis_done) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_images_subject  ON images(subject_analysis_done) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS embedding_queue (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id     INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    pipeline     TEXT NOT NULL DEFAULT 'semantic',
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    scheduled_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_queue_scheduled ON embedding_queue(scheduled_at);

CREATE TABLE IF NOT EXISTS subjects (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    name              TEXT,
    thumbnail_face_id INTEGER,
    type              TEXT NOT NULL DEFAULT 'person',
    added_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS faces (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id    INTEGER NOT NULL,
    subject_id  INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    bbox_x      REAL NOT NULL,
    bbox_y      REAL NOT NULL,
    bbox_w      REAL NOT NULL,
    bbox_h      REAL NOT NULL,
    embedding   BLOB,
    added_at    INTEGER NOT NULL,
    is_manual   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_faces_image ON faces(image_id);
CREATE INDEX IF NOT EXISTS idx_faces_subject ON faces(subject_id);

CREATE TABLE IF NOT EXISTS face_corrections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    face_id INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    old_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    new_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_corrections_face ON face_corrections(face_id);

CREATE TABLE IF NOT EXISTS embedding_cache (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cache_key TEXT NOT NULL UNIQUE,
    query_type TEXT NOT NULL CHECK(query_type IN ('text', 'image')),
    embedding BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_cache_key ON embedding_cache(cache_key);

CREATE TABLE IF NOT EXISTS merge_suggestions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    cross_match_count INTEGER NOT NULL,
    total_pairs INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_merge_pair ON merge_suggestions(
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END
);
"#;

const VERSIONED_MIGRATIONS: &[(u32, &str)] = &[
    (1, "ALTER TABLE images RENAME COLUMN date_file TO mtime"),
    (2, "ALTER TABLE images ADD COLUMN file_size INTEGER NOT NULL DEFAULT 0"),
    (3, r#"
ALTER TABLE faces ADD COLUMN is_manual INTEGER NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS face_corrections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    face_id INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    old_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    new_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_corrections_face ON face_corrections(face_id);
"#),
];
```

#### Part B: Rewrite init_db

- [ ] **Step 2: Replace the `init_db` function (lines 107–137) with the versioned migration system**

```rust
pub async fn init_db(data_dir: &Path) -> Result<SqlitePool> {
    let db_path = data_dir.join("nebula.db");
    let opts = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    sqlx::query("PRAGMA journal_mode=WAL;").execute(&pool).await?;
    sqlx::query("PRAGMA synchronous=NORMAL;").execute(&pool).await?;
    sqlx::query("PRAGMA foreign_keys=ON;").execute(&pool).await?;

    for stmt in BASE_SCHEMA.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            sqlx::query(s).execute(&pool).await?;
        }
    }

    sqlx::query("INSERT OR IGNORE INTO schema_version (rowid, version) VALUES (1, 0)")
        .execute(&pool)
        .await?;

    let current: Option<u32> = sqlx::query_scalar("SELECT version FROM schema_version WHERE rowid = 1")
        .fetch_optional(&pool)
        .await?;

    let current = match current {
        Some(v) => v,
        None => {
            let has_old_column: bool = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('images') WHERE name = 'date_file'",
            )
            .fetch_one::<i64, _>(&pool)
            .await?
                > 0;
            let version = if has_old_column { 0u32 } else { LATEST_VERSION };
            sqlx::query("UPDATE schema_version SET version = ? WHERE rowid = 1")
                .bind(version)
                .execute(&pool)
                .await?;
            version
        }
    };

    for &(version, sql) in VERSIONED_MIGRATIONS {
        if current < version {
            for stmt in sql.split(';') {
                let s = stmt.trim();
                if !s.is_empty() {
                    sqlx::query(s).execute(&pool).await?;
                }
            }
            sqlx::query("UPDATE schema_version SET version = ? WHERE rowid = 1")
                .bind(version)
                .execute(&pool)
                .await?;
        }
    }

    Ok(pool)
}
```

#### Part C: Update row_to_image and Existing Queries

- [ ] **Step 3: Update `row_to_image` helper (line 210–225)**

Replace the function with:

```rust
fn row_to_image(r: &sqlx::sqlite::SqliteRow) -> Image {
    Image {
        id: r.get("id"),
        folder_id: r.get("folder_id"),
        path: r.get("path"),
        file_hash: r.get("file_hash"),
        file_size: r.get::<i64, _>("file_size"),
        date_taken: r.get("date_taken"),
        mtime: r.get("mtime"),
        thumbnail_path: r.get("thumbnail_path"),
        semantic_analysis_done: r.get::<i32, _>("semantic_analysis_done") != 0,
        subject_analysis_done: r.get::<i32, _>("subject_analysis_done") != 0,
        added_at: r.get("added_at"),
        updated_at: r.get("updated_at"),
        deleted_at: r.get("deleted_at"),
    }
}
```

- [ ] **Step 4: Replace `upsert_image` (lines 227–277) with new primitive functions**

Delete the entire `upsert_image` function. Add these four functions in its place:

```rust
pub async fn insert_image(
    pool: &SqlitePool,
    folder_id: i64,
    path: &str,
    file_hash: &str,
    file_size: i64,
    mtime: i64,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO images (folder_id, path, file_hash, file_size, mtime, added_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(folder_id)
    .bind(path)
    .bind(file_hash)
    .bind(file_size)
    .bind(mtime)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn update_image_hash_changed(
    pool: &SqlitePool,
    image_id: i64,
    file_hash: &str,
    file_size: i64,
    mtime: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE images SET file_hash = ?, file_size = ?, mtime = ?,
         semantic_analysis_done = 0, subject_analysis_done = 0, embedding = NULL,
         updated_at = ?, deleted_at = NULL WHERE id = ?",
    )
    .bind(file_hash)
    .bind(file_size)
    .bind(mtime)
    .bind(now)
    .bind(image_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_image_metadata(
    pool: &SqlitePool,
    image_id: i64,
    file_size: i64,
    mtime: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE images SET file_size = ?, mtime = ?, updated_at = ?, deleted_at = NULL WHERE id = ?",
    )
    .bind(file_size)
    .bind(mtime)
    .bind(now)
    .bind(image_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_image_deleted(pool: &SqlitePool, image_id: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE images SET deleted_at = NULL, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(image_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 5: Add `DbImage` struct, `get_all_images_for_rescan`, `get_image_metadata_by_path`, and `soft_delete_image_by_id`**

Add after the `clear_image_deleted` function:

```rust
pub struct DbImage {
    pub id: i64,
    pub path: String,
    pub mtime: i64,
    pub file_size: i64,
    pub file_hash: String,
    pub deleted_at: Option<i64>,
}

pub async fn get_all_images_for_rescan(pool: &SqlitePool) -> Result<Vec<DbImage>> {
    let rows = sqlx::query(
        "SELECT id, path, mtime, file_size, file_hash, deleted_at FROM images",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| DbImage {
            id: r.get("id"),
            path: r.get("path"),
            mtime: r.get("mtime"),
            file_size: r.get::<i64, _>("file_size"),
            file_hash: r.get("file_hash"),
            deleted_at: r.get("deleted_at"),
        })
        .collect())
}

pub async fn get_image_metadata_by_path(pool: &SqlitePool, path: &str) -> Result<Option<DbImage>> {
    let row = sqlx::query(
        "SELECT id, path, mtime, file_size, file_hash, deleted_at FROM images WHERE path = ?",
    )
    .bind(path)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| DbImage {
        id: r.get("id"),
        path: r.get("path"),
        mtime: r.get("mtime"),
        file_size: r.get::<i64, _>("file_size"),
        file_hash: r.get("file_hash"),
        deleted_at: r.get("deleted_at"),
    }))
}

pub async fn soft_delete_image_by_id(pool: &SqlitePool, id: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE images SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 6: Update all SQL strings referencing `date_file` → `mtime` and add `file_size` to SELECTs**

These existing functions need their SQL updated. For each, find the SQL string containing `date_file` and replace with `mtime`, and add `file_size` to the SELECT column list. The functions to update:

**`list_images`** — two SQL branches (with folder_id and without). Both SELECTs change from:
```sql
SELECT id, folder_id, path, file_hash, date_taken, date_file, thumbnail_path,
       semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
```
to:
```sql
SELECT id, folder_id, path, file_hash, file_size, date_taken, mtime, thumbnail_path,
       semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
```
And the `ORDER BY COALESCE(date_taken, date_file)` becomes `ORDER BY COALESCE(date_taken, mtime)`.

**`get_image_by_path`** — same column list change.

**`get_image_by_id`** — same column list change.

**`list_images_for_subject`** — same column list change, same ORDER BY change.

- [ ] **Step 7: Update `search.rs`**

In `src-tauri/src/search.rs`, line 85, change `date_file: img.date_file` to `mtime: img.mtime`.

- [ ] **Step 8: Update `commands.rs` `date_file` references**

In `src-tauri/src/commands.rs`, search for `date_file: img.date_file` and replace with `mtime: img.mtime`. There are two occurrences (around lines 111 and 350).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/search.rs src-tauri/src/commands.rs
git commit -m "refactor: rewrite db schema, versioned migrations, new indexer queries"
```

---

### Task 4: Create indexer.rs — Struct, Init, Helpers

**Files:**
- Create: `src-tauri/src/indexer.rs`

- [ ] **Step 1: Create the file with struct definition, init, and helper functions**

```rust
use anyhow::Result;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::{
    db,
    models::{DebouncedEvent, DebouncedEventKind, SyncProgressPayload, SyncCompletePayload},
    thumbnail,
    watcher::FolderWatcher,
};

pub struct Indexer {
    pool: SqlitePool,
    data_dir: PathBuf,
    folder_map: RwLock<Vec<(PathBuf, i64)>>,
    app: AppHandle,
    watcher: Arc<Mutex<FolderWatcher>>,
    hash_semaphore: Arc<Semaphore>,
}

fn is_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png")
    )
}

fn stat_file(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)?;
    let file_size = meta.len() as i64;
    Some((mtime, file_size))
}

async fn compute_sha256(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await??;
    Ok(hash)
}

fn find_folder_id(folder_map: &[(PathBuf, i64)], path: &Path) -> Option<i64> {
    let path_str = path.to_str()?;
    folder_map
        .iter()
        .filter(|(p, _)| path_str.starts_with(p.to_str().unwrap_or("")))
        .max_by_key(|(p, _)| p.as_os_str().len())
        .map(|(_, id)| *id)
}

fn walk_dir(dir: &Path, results: &mut Vec<(PathBuf, i64, i64, i64)>, folder_id: i64) {
    for entry in std::fs::read_dir(dir).ok() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, results, folder_id);
        } else if is_image(&path) {
            if let Some((mtime, file_size)) = stat_file(&path) {
                results.push((path, folder_id, mtime, file_size));
            }
        }
    }
}

fn walk_dir_for_scan(dir: &Path, results: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).ok() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            walk_dir_for_scan(&path, results);
        } else if is_image(&path) {
            results.push(path);
        }
    }
}

impl Indexer {
    pub async fn init(pool: SqlitePool, data_dir: PathBuf, app: AppHandle) -> Result<Arc<Self>> {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher = Arc::new(Mutex::new(FolderWatcher::new(event_tx)?));

        let mut folder_map = Vec::new();
        {
            let folders = db::list_all_folders(&pool).await?;
            let mut w = watcher.lock().await;
            for folder in &folders {
                let path = PathBuf::from(&folder.path);
                if path.exists() {
                    let _ = w.watch(path.clone(), folder.id);
                }
                folder_map.push((path, folder.id));
            }
        }
        folder_map.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));

        let indexer = Arc::new(Self {
            pool,
            data_dir,
            folder_map: RwLock::new(folder_map),
            app,
            watcher,
            hash_semaphore: Arc::new(Semaphore::new(4)),
        });

        let debounce_indexer = indexer.clone();
        tokio::spawn(async move {
            crate::watcher::run_debounce_loop(event_rx, debounce_indexer).await;
        });

        Ok(indexer)
    }

    async fn sync_folder_map(&self) {
        if let Ok(folders) = db::list_all_folders(&self.pool).await {
            let mut map: Vec<(PathBuf, i64)> = folders
                .into_iter()
                .map(|f| (PathBuf::from(f.path), f.id))
                .collect();
            map.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));
            *self.folder_map.write().await = map;
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/indexer.rs
git commit -m "feat: create indexer module with struct, init, and helpers"
```

---

### Task 5: Add process_file to Indexer

**Files:**
- Modify: `src-tauri/src/indexer.rs`

This is the core unified change detection logic used by both rescan and live events.

- [ ] **Step 1: Add `process_file` and `spawn_thumbnail` methods to the `impl Indexer` block**

Add these methods inside `impl Indexer`, after `sync_folder_map`:

```rust
    async fn process_file(
        &self,
        path: &Path,
        folder_id: i64,
        known: Option<db::DbImage>,
    ) {
        if !is_image(path) {
            return;
        }
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => return,
        };

        let (mtime, file_size) = match stat_file(path) {
            Some(ms) => ms,
            None => return,
        };

        let existing = match known {
            Some(img) => Some(img),
            None => db::get_image_metadata_by_path(&self.pool, &path_str)
                .await
                .ok()
                .flatten(),
        };

        match existing {
            None => {
                let _permit = self.hash_semaphore.acquire().await;
                let hash = match compute_sha256(path).await {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("Failed to hash {}: {}", path_str, e);
                        return;
                    }
                };

                let image_id = match db::insert_image(
                    &self.pool,
                    folder_id,
                    &path_str,
                    &hash,
                    file_size,
                    mtime,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Failed to insert image {}: {}", path_str, e);
                        return;
                    }
                };

                let _ = db::enqueue_image(&self.pool, image_id).await;
                self.spawn_thumbnail(image_id, path.to_path_buf());

                let _ = self.app.emit(
                    "image_added",
                    crate::models::ImageAddedPayload {
                        image_id,
                        path: path_str,
                    },
                );
            }
            Some(existing) => {
                if mtime == existing.mtime && file_size == existing.file_size {
                    if existing.deleted_at.is_some() {
                        let _ = db::clear_image_deleted(&self.pool, existing.id).await;
                        let _ = self.app.emit(
                            "image_added",
                            crate::models::ImageAddedPayload {
                                image_id: existing.id,
                                path: path_str,
                            },
                        );
                    }
                    return;
                }

                let _permit = self.hash_semaphore.acquire().await;
                let hash = match compute_sha256(path).await {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("Failed to hash {}: {}", path_str, e);
                        return;
                    }
                };

                if hash == existing.file_hash {
                    let _ =
                        db::update_image_metadata(&self.pool, existing.id, file_size, mtime).await;
                    if existing.deleted_at.is_some() {
                        let _ = self.app.emit(
                            "image_added",
                            crate::models::ImageAddedPayload {
                                image_id: existing.id,
                                path: path_str,
                            },
                        );
                    }
                } else {
                    let _ = db::update_image_hash_changed(
                        &self.pool,
                        existing.id,
                        &hash,
                        file_size,
                        mtime,
                    )
                    .await;
                    let _ = db::enqueue_image(&self.pool, existing.id).await;
                    self.spawn_thumbnail(existing.id, path.to_path_buf());
                    let _ = self.app.emit(
                        "image_updated",
                        crate::models::ImageUpdatedPayload {
                            image_id: existing.id,
                        },
                    );
                }
            }
        }
    }

    fn spawn_thumbnail(&self, image_id: i64, src: PathBuf) {
        let thumb_path = thumbnail::thumbnail_path_for(&self.data_dir, image_id);
        let thumb_path_str = thumb_path.to_string_lossy().to_string();
        let pool = self.pool.clone();
        let app = self.app.clone();
        tokio::spawn(async move {
            if let Err(e) = thumbnail::generate_thumbnail(src, thumb_path).await {
                eprintln!("Thumbnail generation failed: {}", e);
                return;
            }
            if db::update_thumbnail_path(&pool, image_id, &thumb_path_str)
                .await
                .is_ok()
            {
                let _ = app.emit(
                    "image_updated",
                    crate::models::ImageUpdatedPayload { image_id },
                );
            }
        });
    }
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/indexer.rs
git commit -m "feat: add process_file with unified change detection logic"
```

---

### Task 6: Add start_rescan to Indexer

**Files:**
- Modify: `src-tauri/src/indexer.rs`

This implements the background startup rescan with parallel stat() walk and three-way diff.

- [ ] **Step 1: Add `start_rescan` method to `impl Indexer`**

Add after `spawn_thumbnail`:

```rust
    pub async fn start_rescan(&self) {
        let folder_map = self.folder_map.read().await;
        let folders: Vec<(PathBuf, i64)> = folder_map
            .iter()
            .filter(|(p, _)| p.exists())
            .cloned()
            .collect();
        drop(folder_map);

        let disk_files: Vec<(PathBuf, i64, i64, i64)> = folders
            .par_iter()
            .flat_map(|(folder_path, folder_id)| {
                let mut results = Vec::new();
                walk_dir(folder_path, &mut results, *folder_id);
                results
            })
            .collect();

        let db_images = match db::get_all_images_for_rescan(&self.pool).await {
            Ok(imgs) => imgs,
            Err(e) => {
                eprintln!("Rescan: failed to load DB images: {}", e);
                return;
            }
        };

        let mut db_map: HashMap<String, db::DbImage> = HashMap::with_capacity(db_images.len());
        for img in db_images {
            db_map.insert(img.path.clone(), img);
        }

        let disk_set: std::collections::HashSet<String> = disk_files
            .iter()
            .map(|(p, _, _, _)| p.to_string_lossy().to_string())
            .collect();

        for (path, img) in &db_map {
            if img.deleted_at.is_none() && !disk_set.contains(path.as_str()) {
                let _ = db::soft_delete_image_by_id(&self.pool, img.id).await;
            }
        }

        let total = disk_files.len() as u32;
        let mut done: u32 = 0;

        for (path, folder_id, _mtime, _file_size) in &disk_files {
            let path_str = path.to_string_lossy().to_string();
            let known = db_map.get(&path_str).cloned();

            self.process_file(path, *folder_id, known).await;

            done += 1;
            if done % 100 == 0 || done == total {
                let _ = self.app.emit(
                    "sync_progress",
                    SyncProgressPayload { done, total },
                );
            }
        }

        let _ = self.app.emit("sync_complete", SyncCompletePayload {});
    }
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/indexer.rs
git commit -m "feat: add start_rescan with parallel stat walk and three-way diff"
```

---

### Task 7: Add Folder Management and Event Handling to Indexer

**Files:**
- Modify: `src-tauri/src/indexer.rs`

- [ ] **Step 1: Add `add_folder`, `remove_folder`, and `handle_event_batch` to `impl Indexer`**

Add after `start_rescan`:

```rust
    pub async fn add_folder(&self, path: String) -> Result<crate::models::FolderWithCount> {
        let folder_id = db::insert_folder(&self.pool, &path).await?;

        {
            let mut w = self.watcher.lock().await;
            let _ = w.watch(PathBuf::from(&path), folder_id);
        }

        self.sync_folder_map().await;

        let folder_path = PathBuf::from(&path);
        self.start_folder_scan(&folder_path, folder_id).await;

        let folders = db::list_folders_with_counts(&self.pool).await?;
        folders
            .into_iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| anyhow::anyhow!("Folder not found after insert"))
    }

    pub async fn remove_folder(&self, id: i64) -> Result<()> {
        let folders = db::list_folders_with_counts(&self.pool).await?;
        if let Some(folder) = folders.iter().find(|f| f.id == id) {
            let path = PathBuf::from(&folder.path);
            let mut w = self.watcher.lock().await;
            let _ = w.unwatch(&path);
        }

        db::delete_folder(&self.pool, id).await?;
        self.sync_folder_map().await;
        Ok(())
    }

    pub async fn handle_event_batch(&self, events: Vec<DebouncedEvent>) {
        let folder_map = self.folder_map.read().await;
        for event in events {
            match event.kind {
                DebouncedEventKind::Create | DebouncedEventKind::Modify => {
                    if let Some(folder_id) = find_folder_id(&folder_map, &event.path) {
                        self.process_file(&event.path, folder_id, None).await;
                    }
                }
                DebouncedEventKind::Remove => {
                    let path_str = event.path.to_string_lossy().to_string();
                    if db::soft_delete_image(&self.pool, &path_str).await.is_ok() {
                        let _ = self.app.emit(
                            "image_removed",
                            crate::models::ImageRemovedPayload { path: path_str },
                        );
                    }
                }
            }
        }
    }

    async fn start_folder_scan(&self, folder_path: &Path, folder_id: i64) {
        let folder_path_owned = folder_path.to_path_buf();
        let entries: Vec<PathBuf> = match tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            walk_dir_for_scan(&folder_path_owned, &mut results);
            results
        })
        .await
        {
            Ok(Ok(e)) => e,
            _ => return,
        };

        for path in entries {
            self.process_file(&path, folder_id, None).await;
        }
    }
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/indexer.rs
git commit -m "feat: add folder management and event handling to indexer"
```

---

### Task 8: Rewrite watcher.rs

**Files:**
- Modify: `src-tauri/src/watcher.rs`

Strip to a thin FS adapter (FolderWatcher struct) plus the debounce loop. Remove all indexing logic (`handle_new_file`, `handle_modified_file`, `scan_folder`, `find_folder_for_path`, `compute_sha256`, `get_mtime`, `is_image`, `collect_image_paths`, `collect_recursive`).

- [ ] **Step 1: Replace the entire contents of `src-tauri/src/watcher.rs`**

```rust
use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::indexer::Indexer;
use crate::models::{DebouncedEvent, DebouncedEventKind};

pub struct FolderWatcher {
    inner: RecommendedWatcher,
}

impl FolderWatcher {
    pub fn new(event_tx: mpsc::UnboundedSender<Event>) -> Result<Self> {
        let inner = notify::recommended_watcher(move |res: notify::Result<Event>| {
            match res {
                Ok(event) => {
                    let _ = event_tx.send(event);
                }
                Err(e) => eprintln!("file watcher backend error: {e}"),
            }
        })?;
        Ok(Self { inner })
    }

    pub fn watch(&mut self, path: PathBuf, _folder_id: i64) -> Result<()> {
        self.inner.watch(&path, RecursiveMode::Recursive)?;
        Ok(())
    }

    pub fn unwatch(&mut self, path: &Path) -> Result<()> {
        self.inner.unwatch(path)?;
        Ok(())
    }
}

pub async fn run_debounce_loop(
    mut rx: mpsc::UnboundedReceiver<Event>,
    indexer: Arc<Indexer>,
) {
    let mut debounce_map: HashMap<PathBuf, (DebouncedEventKind, Instant)> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_millis(200));

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                for path in event.paths {
                    let kind = match event.kind {
                        EventKind::Create(_) => DebouncedEventKind::Create,
                        EventKind::Modify(_) => DebouncedEventKind::Modify,
                        EventKind::Remove(_) => DebouncedEventKind::Remove,
                        _ => continue,
                    };
                    coalesce(&mut debounce_map, path, kind);
                }
            }
            _ = interval.tick() => {
                let now = Instant::now();
                let expired: Vec<DebouncedEvent> = debounce_map
                    .iter()
                    .filter(|(_, (_, instant))| now.duration_since(*instant) > Duration::from_millis(500))
                    .map(|(path, (kind, _))| DebouncedEvent {
                        path: path.clone(),
                        kind: kind.clone(),
                    })
                    .collect();

                for event in &expired {
                    debounce_map.remove(&event.path);
                }

                if !expired.is_empty() {
                    indexer.handle_event_batch(expired).await;
                }
            }
        }
    }
}

fn coalesce(
    map: &mut HashMap<PathBuf, (DebouncedEventKind, Instant)>,
    path: PathBuf,
    incoming: DebouncedEventKind,
) {
    let now = Instant::now();
    match map.get(&path) {
        Some((DebouncedEventKind::Create, _)) => match incoming {
            DebouncedEventKind::Remove => {
                map.remove(&path);
            }
            DebouncedEventKind::Modify | DebouncedEventKind::Create => {
                map.insert(path, (DebouncedEventKind::Create, now));
            }
        },
        Some((DebouncedEventKind::Modify, _)) => match incoming {
            DebouncedEventKind::Remove => {
                map.insert(path, (DebouncedEventKind::Remove, now));
            }
            DebouncedEventKind::Create => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
            DebouncedEventKind::Modify => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
        },
        Some((DebouncedEventKind::Remove, _)) => match incoming {
            DebouncedEventKind::Create => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
            DebouncedEventKind::Modify => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
            DebouncedEventKind::Remove => {
                map.insert(path, (DebouncedEventKind::Remove, now));
            }
        },
        None => {
            map.insert(path, (incoming, now));
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/watcher.rs
git commit -m "refactor: strip watcher to thin adapter with debounce loop"
```

---

### Task 9: Update lib.rs — Wire Indexer

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `mod indexer` and update imports**

Add `mod indexer;` after the existing module declarations (after line 10). Remove the `use watcher::FolderWatcher;` import (line 18).

- [ ] **Step 2: Update `AppState` struct**

Replace the `AppState` struct (lines 20–26) with:

```rust
pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub api_key: Arc<Mutex<Option<String>>>,
    pub indexer: Arc<indexer::Indexer>,
    pub vision_engine: Arc<vision_engine::VisionEngine>,
}
```

- [ ] **Step 3: Simplify startup wiring in the `setup` closure**

Replace the entire `setup` closure body (lines 33–155). The new startup:

```rust
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(thumbnail::thumbnail_cache_dir(&data_dir))?;
            std::fs::create_dir_all(thumbnail::face_crop_cache_dir(&data_dir))?;

            let pool = tauri::async_runtime::block_on(db::init_db(&data_dir))?;

            let api_key = config::read_api_key(&data_dir);
            let api_key = Arc::new(Mutex::new(api_key));

            let vision_engine = Arc::new(vision_engine::VisionEngine::new(data_dir.clone()));

            let indexer = tauri::async_runtime::block_on(
                indexer::Indexer::init(pool.clone(), data_dir.clone(), app.handle().clone())
            )?;

            app.manage(AppState {
                pool: pool.clone(),
                data_dir: data_dir.clone(),
                api_key: api_key.clone(),
                indexer,
                vision_engine: vision_engine.clone(),
            });

            let indexer_rescan = app.state::<AppState>().indexer.clone();
            tauri::async_runtime::spawn(async move {
                indexer_rescan.start_rescan().await;
            });

            let vision_engine_model = Arc::clone(&vision_engine);
            let app_handle_model = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = vision_engine_model.ensure_model_ready(&app_handle_model).await {
                    eprintln!("Model setup failed: {}", e);
                    let _ = app_handle_model.emit(
                        "model_download_progress",
                        crate::models::ModelDownloadPayload {
                            file: String::new(),
                            bytes_done: 0,
                            bytes_total: None,
                            done: false,
                            error: Some(e.to_string()),
                        },
                    );
                }
            });

            let pool_semantic = pool.clone();
            let app_handle_semantic = app.handle().clone();
            let vision_engine_semantic = Arc::clone(&vision_engine);
            tauri::async_runtime::spawn(async move {
                embedder::run_semantic_worker(pool_semantic, app_handle_semantic, vision_engine_semantic).await;
            });

            let pool_subject = pool.clone();
            let app_handle_subject = app.handle().clone();
            let vision_engine_subject = Arc::clone(&vision_engine);
            tauri::async_runtime::spawn(async move {
                embedder::run_subject_worker(pool_subject, app_handle_subject, vision_engine_subject).await;
            });

            Ok(())
        })
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "refactor: wire indexer in lib.rs, simplify startup"
```

---

### Task 10: Update commands.rs — Delegate to Indexer

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Replace `add_folder` command**

Replace the `add_folder` function (lines 16–48) with:

```rust
#[tauri::command]
pub async fn add_folder(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<FolderWithCount, String> {
    state
        .indexer
        .add_folder(path)
        .await
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Replace `remove_folder` command**

Replace the `remove_folder` function (lines 50–68) with:

```rust
#[tauri::command]
pub async fn remove_folder(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .indexer
        .remove_folder(id)
        .await
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Update imports**

At the top of `commands.rs`, remove `watcher` from the `use crate::{...}` import since we no longer call `watcher::scan_folder`. Also remove `use sha2::{Sha256, Digest};` from line 2 if no other code in the file uses it (check the `search` command — it uses `base64`, not `sha2`). The `use` block becomes:

```rust
use crate::{
    config, db,
    models::{ProcessingStatus, FolderWithCount, Image, SearchResult, SearchQuery, Subject, Face, MergeSuggestion, NameSubjectResult},
    search, thumbnail, AppState,
};
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "refactor: delegate folder commands to indexer"
```

---

### Task 11: Final Verification

**Files:**
- All modified files

- [ ] **Step 1: Run `cargo check`**

Run: `cargo check` in `src-tauri/`
Expected: Clean compilation with no errors

If errors occur:
- Search for any remaining `date_file` references: `grep -r "date_file" src-tauri/src/`
- Verify all imports are correct
- Verify the `DbImage` struct in `db.rs` is `pub`
- Check that `indexer` module is declared in `lib.rs`
- Check that `FolderWatcher` fields and methods match what `indexer.rs` expects

- [ ] **Step 2: Run `cargo build`**

Run: `cargo build` in `src-tauri/`
Expected: Successful build

- [ ] **Step 3: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "fix: resolve compilation errors from indexer overhaul"
```

---

## Self-Review Checklist

### Spec Coverage

| Spec Requirement | Task |
|---|---|
| Startup re-hashes eliminated (parallel stat + mtime/size check) | Task 4, 6 |
| Re-analysis bug fixed (same hash preserves embedding) | Task 5 (process_file) |
| FS event debouncing (200ms interval, 500ms idle) | Task 8 |
| `find_folder_for_path` DB hit eliminated (in-memory folder map) | Task 4, 7 |
| All indexing logic in one module (`indexer.rs`) | Tasks 4–7 |
| Migration system versioned, no `let _ =` | Task 3 |
| `date_file` → `mtime` rename | Tasks 2, 3 |
| `file_size` column added | Tasks 2, 3 |
| `is_manual` and `face_corrections` in base schema | Task 3 |
| `sync_progress` and `sync_complete` events | Tasks 2, 6 |
| Bounded SHA-256 (4 concurrent) | Task 4 (hash_semaphore) |
| Parallel stat() via rayon | Task 6 |
| `AppState` uses `Arc<Indexer>` | Task 9 |
| `watcher.rs` thin adapter (~120 lines) | Task 8 |
| `commands.rs` delegates to indexer | Task 10 |
| `search.rs` only gets `date_file`→`mtime` rename | Task 3 Step 7 |

### Placeholder Scan

No TBD, TODO, or placeholder patterns in this plan. All code is complete.

### Type Consistency

- `DbImage` in `db.rs` has fields: `id`, `path`, `mtime`, `file_size`, `file_hash`, `deleted_at` — matches usage in `indexer.rs` `process_file` and `start_rescan`
- `DebouncedEvent` and `DebouncedEventKind` defined in `models.rs` — used in `watcher.rs` debounce loop and `indexer.rs` `handle_event_batch`
- `Image` struct has `mtime` and `file_size` — matches `row_to_image` and all SQL queries
- `SyncProgressPayload` and `SyncCompletePayload` defined in `models.rs` — used in `indexer.rs` `start_rescan`
- All DB function signatures match their call sites in `indexer.rs`
