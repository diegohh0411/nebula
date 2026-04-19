# Indexer Overhaul Design

**Date:** 2026-04-18  
**Status:** Approved  
**Scope:** Rust backend (`src-tauri/src/`)

---

## Problem Statement

The current file/folder tracking system has several reliability and performance problems that will compound as the library grows toward thousands of images:

1. **Startup re-hashes every file** — `scan_folder` runs SHA-256 on every image on every boot, reading gigabytes of data before the app is usable.
2. **Re-analysis bug** — `upsert_image` resets `semantic_analysis_done`, `subject_analysis_done`, and `embedding` whenever `was_deleted || hash_changed`. A file that disappears and returns with the same hash gets fully re-embedded unnecessarily.
3. **No debouncing** — the FS event consumer processes every raw notify event serially. A large file being copied fires 50+ events; each triggers a SHA-256 on a potentially half-written file.
4. **`find_folder_for_path` hits the DB on every FS event** — fetches all folders on every Create event to find the parent folder.
5. **Responsibility is scattered** — folder management logic lives across `watcher.rs`, `lib.rs`, and `commands.rs` with no clear owner.
6. **Fragile migration system** — `POST_MIGRATIONS` silently swallows all errors with `let _ =`; no version tracking; relies on `ALTER TABLE` failing idempotently.
7. **Sequential event processing** — events processed one at a time, no batching.

---

## Goals

- App starts and renders the gallery instantly from DB state
- Startup rescan detects changes without reading file content for unchanged files
- File re-appearances with the same content never trigger re-analysis
- FS events are debounced so bulk copies and in-progress writes are handled cleanly
- All indexing logic lives in one module (`indexer.rs`)
- Migration system is versioned, explicit, and never swallows errors

---

## Module Structure

```
watcher.rs   → thin FS adapter only. Receives notify events, feeds debounce loop.
indexer.rs   → owns all indexing logic. Single source of truth for folder/file state.
db.rs        → gains new queries; interface otherwise unchanged.
lib.rs       → wires indexer on startup, nothing else.
commands.rs  → add/remove folder delegates to indexer, not directly to db/watcher.
```

`AppState` replaces `watcher: Arc<Mutex<FolderWatcher>>` with `indexer: Arc<Indexer>`.

---

## DB Schema Changes

### Renamed column
`date_file` → `mtime` (standard Unix term, unambiguous)

### New column on `images`
```sql
file_size INTEGER NOT NULL DEFAULT 0
```

Used as the first dirty-check before computing SHA-256. Combined with `mtime`, this catches the vast majority of unchanged files without any disk read.

### `is_manual` and `face_corrections` move to base schema
Previously added via `POST_MIGRATIONS`; now part of the initial `CREATE TABLE` statements.

### Clean `images` table definition
```sql
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
```

### Migration system
- Add `schema_version` table to base schema
- Replace `MIGRATIONS` + `POST_MIGRATIONS` with a single versioned array: `&[(version: u32, sql: &str)]`
- On init: read current version, apply all pending migrations in order, update version after each
- No `let _ =` anywhere in the migration path — real errors propagate

---

## Startup Rescan Flow

App is immediately interactive. Rescan runs entirely in the background.

```
App starts
│
├─ DB loads → gallery renders from cached state
│
└─ indexer.start_rescan() [background task]
     │
     ├─ 1. Parallel stat() walk via rayon
     │       Collects Vec<(PathBuf, mtime, file_size)> — no file reads
     │
     ├─ 2. Single DB query
     │       Fetches all non-deleted images: (id, path, mtime, file_size, file_hash, analysis_done flags)
     │
     ├─ 3. Three-way diff (in memory)
     │       • In DB, not on disk        → soft-delete
     │       • On disk, not in DB        → new file, insert + enqueue
     │       • In both → compare mtime+size:
     │           same     → skip entirely (no disk read)
     │           different → compute SHA-256
     │               hash same    → update mtime/size, clear deleted_at if needed,
     │                              PRESERVE embedding + analysis flags
     │               hash changed → reset analysis flags, clear embedding, re-enqueue
     │
     ├─ 4. Emit sync_progress events (% complete) to frontend
     │
     └─ 5. Emit sync_complete → frontend clears "syncing" indicator
```

**Why parallel stat() but bounded SHA-256:**

`stat()` reads only filesystem metadata — safe and fast to run on all CPU cores via rayon. SHA-256 is I/O bound. On HDDs, parallel reads cause seek thrashing. The implementation uses a bounded semaphore (4 concurrent hashes) so behaviour is reasonable on both HDDs and SSDs. In practice, for most rescans after the first, 99% of files match on mtime+size and SHA-256 is never called.

**DB access pattern:** One read (all known images), then batched writes for changes only. DB round trips drop from O(n files) to O(1) read + O(changed files) writes.

---

## Change Detection Logic

One function — `process_file()` — used by both the startup rescan and the live FS event handler. No special cases.

In the **rescan path**, all known images are pre-fetched into a `HashMap<String, DbImage>` before the walk begins, so each `process_file` call does an in-memory lookup — zero extra DB round trips. In the **live event path**, the lookup is a single DB query per event (acceptable since events are infrequent).

```
process_file(path, folder_id, known: Option<DbImage>):
  stat(path) → (mtime, file_size)
  existing = known ?? db::get_image_by_path(path)

  if existing is None:
    insert new image, enqueue both pipelines

  else if mtime == existing.mtime && file_size == existing.file_size:
    if stored.deleted_at IS NOT NULL:
      clear deleted_at only (file reappeared, content unchanged → no re-analysis)
    else:
      skip entirely

  else:
    compute SHA-256
    if hash == stored.file_hash:
      update mtime + file_size, clear deleted_at if needed
      preserve semantic_analysis_done, subject_analysis_done, embedding
    else:
      update hash + mtime + file_size
      reset semantic_analysis_done = 0, subject_analysis_done = 0, embedding = NULL
      clear deleted_at
      re-enqueue both pipelines
```

This fixes the re-analysis bug: a file that disappears and returns with the same content is never re-embedded.

---

## FS Event Handling with Debouncing

`watcher.rs` collects raw notify events into a debounce map. Every 200ms, expired entries (idle > 500ms) are drained and passed to `indexer.handle_event_batch()`.

**Debounce map:** `HashMap<PathBuf, (EventKind, Instant)>`

**Coalescing rules:**
| Sequence | Result |
|---|---|
| Create → Modify | Create (already queued as new) |
| Multiple Modify | Single Modify (timer resets each time) |
| Create → Remove | Cancel — nothing written to DB |
| Remove → Create | Treat as Modify (file replaced in-place) |

**`handle_event_batch()`** calls `process_file()` for Create/Modify events and `soft_delete_image()` for Remove events. Same change detection logic as the rescan — no duplication.

---

## In-Memory Folder Map

`Indexer` holds a `RwLock<Vec<(PathBuf, i64)>>` sorted by path length descending. Finding the owner folder of any file path is a linear scan in memory — O(n folders), effectively O(1) for realistic folder counts (< 20). Updated on `add_folder` and `remove_folder`. Eliminates all DB round trips from `find_folder_for_path`.

---

## `indexer.rs` Public Interface

```rust
pub struct Indexer { /* pool, data_dir, folder_map, app */ }

impl Indexer {
    // Startup
    pub async fn init(pool, data_dir, app) -> Arc<Self>
    pub async fn start_rescan(&self)           // non-blocking, background task

    // Folder management (called from commands.rs)
    pub async fn add_folder(&self, path: String) -> Result<FolderWithCount>
    pub async fn remove_folder(&self, id: i64) -> Result<()>

    // Event handling (called from watcher.rs)
    pub async fn handle_event_batch(&self, events: Vec<DebouncedEvent>)

    // Internal
    async fn process_file(&self, path: &Path, folder_id: i64)
    async fn sync_folder_map(&self)
}
```

`watcher.rs` shrinks to ~40 lines. `lib.rs` startup becomes three lines. `commands.rs` folder management delegates entirely to `indexer`.

---

## What Does Not Change

- Embedding queue table and pipeline (`semantic` / `subject`) — unchanged
- Thumbnail generation logic — unchanged  
- `search.rs`, `clustering.rs`, `vision_engine.rs` — unchanged
- Frontend Tauri events (`image_added`, `image_updated`, `image_removed`) — same names, same payloads; two new events added: `sync_progress { done: u32, total: u32 }` and `sync_complete`

---

## Files Changed

| File | Change |
|---|---|
| `src-tauri/src/db.rs` | Schema rename + new column + migration system rewrite |
| `src-tauri/src/models.rs` | Rename `date_file` → `mtime`, add `file_size` field; add `SyncProgressPayload` |
| `src-tauri/src/indexer.rs` | **New file** — all indexing logic |
| `src-tauri/src/watcher.rs` | Stripped to thin adapter + debounce loop |
| `src-tauri/src/lib.rs` | Startup wiring simplified |
| `src-tauri/src/commands.rs` | `add_folder` / `remove_folder` delegate to indexer |
| `src-tauri/src/Cargo.toml` | Add `rayon` dependency |
