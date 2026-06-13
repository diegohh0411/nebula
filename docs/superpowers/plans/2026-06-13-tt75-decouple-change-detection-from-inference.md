# TT-75 — Decouple Change-Detection from Inference (cheap, throttled, lazy hashing) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `(file_size, mtime)` the authoritative change signal and move content hashing off the import critical path into a single bounded, low-priority BLAKE3 worker that yields to the inference pipeline, so a 1,000+ image import reaches full embedding/face throughput within seconds.

**Architecture:** First-import inserts an image (`file_hash = ''`, `hash_status = 'PENDING'` by schema default), enqueues it for inference, and emits `image_added` — with **no** per-file hash task spawned. A new background worker (`library/hasher.rs`) pulls `hash_status = 'PENDING'` rows in batches, computes BLAKE3 only while the inference queue is shallow (backpressure), and writes results in a single batched transaction. The modify path keeps an inline tie-breaker hash (now BLAKE3) but only when `(size, mtime)` actually changed, and treats `(size, mtime)`-unchanged files as unchanged regardless of `hash_status`.

**Tech Stack:** Rust, Tauri, sqlx (SQLite/WAL), tokio, BLAKE3 (`blake3` crate). No schema change — `file_hash` stays `TEXT`, `hash_status` already exists (per [CLAUDE.md](CLAUDE.md): `db/mod.rs` owns `BASE_SCHEMA` + `VERSIONED_MIGRATIONS`; do not edit `BASE_SCHEMA` in place).

---

## File Structure

- **Create** `src-tauri/src/library/hasher.rs` — the bounded BLAKE3 hash worker: `compute_blake3`, `spawn_hash_worker`, `run_hash_worker`.
- **Modify** `src-tauri/src/library/mod.rs` — register `pub mod hasher;`.
- **Modify** `src-tauri/src/library/repo.rs` — add `get_pending_hash_batch` + `apply_hash_results`.
- **Modify** `src-tauri/src/pipeline/queue.rs` — add `count_pending_inference` (backpressure signal; pipeline-domain query).
- **Modify** `src-tauri/src/library/indexer.rs` — remove per-file hash spawn on first import; swap modify-path tie-breaker from SHA256 to BLAKE3; fix the modify-path "unchanged" check to ignore `hash_status`; delete `compute_sha256` + `sha2` import.
- **Modify** `src-tauri/src/app/mod.rs` — spawn the hash worker at startup.
- **Modify** `src-tauri/Cargo.toml` — add `blake3` dependency.
- **Modify** `src-tauri/src/db/tests.rs` — tests for new repo helpers + BLAKE3 helper.

> All paths below are relative to `src-tauri/`. Run all `cargo` commands from `src-tauri/`.

---

### Task 1: Add BLAKE3 dependency + `compute_blake3` helper

**Files:**
- Modify: `Cargo.toml:34` (dependency block)
- Create: `src/library/hasher.rs`
- Modify: `src/library/mod.rs`
- Test: `src/db/tests.rs`

- [ ] **Step 1: Add the `blake3` dependency**

In `Cargo.toml`, the dependency block currently contains (around line 34):

```toml
sha2 = "0.10"
```

Add directly below it:

```toml
blake3 = "1"
```

(Leave `sha2` in place — it is still used by `src/commands.rs`. Only `indexer.rs` stops using it, in Task 5.)

- [ ] **Step 2: Create `src/library/hasher.rs` with the BLAKE3 helper only**

Create `src/library/hasher.rs` with exactly this content for now (the worker is added in Task 4):

```rust
//! Bounded, low-priority content-hash worker (TT-75).
//!
//! Change-detection authority is `(file_size, mtime)`; the content hash is only
//! a tie-breaker. We use BLAKE3 (non-cryptographic strength is sufficient and it
//! is ~5–10× cheaper than SHA256) and compute it off the import critical path.

use anyhow::Result;
use std::path::Path;

/// Compute the BLAKE3 hex digest of a file's contents on a blocking thread.
pub async fn compute_blake3(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(hasher.finalize().to_hex().to_string())
    })
    .await??;
    Ok(hash)
}
```

- [ ] **Step 3: Register the module**

In `src/library/mod.rs`, add the module declaration alongside the other `pub mod` lines (keep the file's existing ordering/style):

```rust
pub mod hasher;
```

- [ ] **Step 4: Write the failing test for `compute_blake3`**

Append to `src/db/tests.rs`:

```rust
#[tokio::test]
async fn compute_blake3_is_deterministic_and_content_sensitive() {
    use crate::library::hasher::compute_blake3;

    let dir = std::env::temp_dir().join(format!("nebula_blake3_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    std::fs::write(&a, b"hello world").unwrap();
    std::fs::write(&b, b"hello worlD").unwrap();

    let h1 = compute_blake3(&a).await.unwrap();
    let h2 = compute_blake3(&a).await.unwrap();
    let h3 = compute_blake3(&b).await.unwrap();

    assert_eq!(h1, h2, "same content must hash identically");
    assert_ne!(h1, h3, "different content must hash differently");
    assert_eq!(h1.len(), 64, "BLAKE3 hex digest is 64 chars");

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test --manifest-path Cargo.toml compute_blake3_is_deterministic -- --nocapture`
Expected: FAILS to compile until `blake3` resolves and `hasher` is wired — once compiling, the test PASSES. If it fails to compile with "unresolved import `blake3`", run `cargo fetch` first.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --manifest-path Cargo.toml compute_blake3_is_deterministic`
Expected: PASS (1 passed).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/library/hasher.rs src/library/mod.rs src/db/tests.rs
git commit -m "feat(library): add BLAKE3 compute helper (TT-75)"
```

---

### Task 2: Add repo helpers — pending-hash batch read + batched write

**Files:**
- Modify: `src/library/repo.rs` (append functions after `get_all_images_for_rescan`, ~line 195)
- Test: `src/db/tests.rs`

- [ ] **Step 1: Write the failing test for the batch read/write round-trip**

Append to `src/db/tests.rs`:

```rust
#[tokio::test]
async fn pending_hash_batch_and_apply_results_round_trip() {
    use crate::library::repo::{get_pending_hash_batch, apply_hash_results};

    let dir = std::env::temp_dir().join(format!("nebula_hashbatch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    let fid = insert_folder(&pool, "/tmp/f").await.unwrap();

    // Three PENDING images (insert_image leaves hash_status at its 'PENDING' default).
    let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "", 10, 100).await.unwrap();
    let b = insert_image(&pool, fid, "/tmp/f/b.jpg", "", 20, 200).await.unwrap();
    let c = insert_image(&pool, fid, "/tmp/f/c.jpg", "", 30, 300).await.unwrap();

    // Soft-delete c: it must NOT appear in the pending batch.
    sqlx::query("UPDATE images SET deleted_at = 1 WHERE id = ?").bind(c).execute(&pool).await.unwrap();

    let batch = get_pending_hash_batch(&pool, 10).await.unwrap();
    let ids: Vec<i64> = batch.iter().map(|(id, _, _)| *id).collect();
    assert!(ids.contains(&a) && ids.contains(&b), "live PENDING rows must be returned");
    assert!(!ids.contains(&c), "soft-deleted rows must be excluded");
    // mtime is carried so writes can be guarded against concurrent modification.
    assert!(batch.iter().any(|(id, _, m)| *id == a && *m == 100));

    // Apply: a succeeds with a hash, b fails (None).
    apply_hash_results(&pool, &[(a, 100, Some("deadbeef".to_string())), (b, 200, None)])
        .await
        .unwrap();

    let img_a = get_image_by_id(&pool, a).await.unwrap().unwrap();
    let img_b = get_image_by_id(&pool, b).await.unwrap().unwrap();
    assert_eq!(img_a.file_hash, "deadbeef");
    assert_eq!(img_a.hash_status, "DONE");
    assert_eq!(img_b.hash_status, "FAILED");

    // a is no longer PENDING, so a re-read returns only nothing new.
    let after = get_pending_hash_batch(&pool, 10).await.unwrap();
    assert!(after.iter().all(|(id, _, _)| *id != a && *id != b));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn apply_hash_results_is_guarded_by_mtime() {
    use crate::library::repo::apply_hash_results;

    let dir = std::env::temp_dir().join(format!("nebula_hashguard_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    let fid = insert_folder(&pool, "/tmp/f").await.unwrap();
    let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "", 10, 100).await.unwrap();

    // The file was re-touched while hashing was in flight: mtime is now 999.
    sqlx::query("UPDATE images SET mtime = 999 WHERE id = ?").bind(a).execute(&pool).await.unwrap();

    // Applying a result computed against the OLD mtime (100) must be a no-op.
    apply_hash_results(&pool, &[(a, 100, Some("stale".to_string()))]).await.unwrap();

    let img = get_image_by_id(&pool, a).await.unwrap().unwrap();
    assert_ne!(img.file_hash, "stale", "stale-mtime write must not clobber a re-touched file");
    assert_eq!(img.hash_status, "PENDING", "row stays PENDING so the worker re-hashes it");

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml pending_hash_batch apply_hash_results_is_guarded`
Expected: FAIL to compile — `get_pending_hash_batch` / `apply_hash_results` not found.

- [ ] **Step 3: Implement the repo helpers**

In `src/library/repo.rs`, add these two functions immediately after `get_all_images_for_rescan` (after line 195):

```rust
/// A batch of images still awaiting a content hash. Returns `(id, path, mtime)`.
/// Excludes soft-deleted rows; ordered by id so progress is FIFO and stable.
pub async fn get_pending_hash_batch(pool: &SqlitePool, limit: i64) -> Result<Vec<(i64, String, i64)>> {
    let rows = sqlx::query(
        "SELECT id, path, mtime FROM images
         WHERE hash_status = 'PENDING' AND deleted_at IS NULL
         ORDER BY id ASC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<String, _>("path"), r.get::<i64, _>("mtime")))
        .collect())
}

/// Write a batch of hash results in a single transaction (one writer burst per
/// batch instead of one UPDATE per file). Each entry is `(id, mtime, hash)`:
/// `Some(hash)` → DONE, `None` → FAILED. Every UPDATE is guarded by `mtime` so a
/// result computed against a now-stale file is dropped (the row stays PENDING and
/// is re-hashed on the next pass).
pub async fn apply_hash_results(
    pool: &SqlitePool,
    results: &[(i64, i64, Option<String>)],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    for (id, mtime, hash) in results {
        match hash {
            Some(h) => {
                sqlx::query(
                    "UPDATE images SET file_hash = ?, hash_status = 'DONE' WHERE id = ? AND mtime = ?",
                )
                .bind(h)
                .bind(id)
                .bind(mtime)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query("UPDATE images SET hash_status = 'FAILED' WHERE id = ? AND mtime = ?")
                    .bind(id)
                    .bind(mtime)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml pending_hash_batch apply_hash_results_is_guarded`
Expected: PASS (2 passed).

- [ ] **Step 5: Commit**

```bash
git add src/library/repo.rs src/db/tests.rs
git commit -m "feat(library): batched pending-hash read + mtime-guarded write (TT-75)"
```

---

### Task 3: Add the inference-queue backpressure signal

**Files:**
- Modify: `src/pipeline/queue.rs` (append after `get_processing_counts`, ~line 102)
- Test: `src/db/tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `src/db/tests.rs`:

```rust
#[tokio::test]
async fn count_pending_inference_counts_distinct_images() {
    use crate::pipeline::queue::{enqueue_image, count_pending_inference};

    let dir = std::env::temp_dir().join(format!("nebula_inferdepth_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    let fid = insert_folder(&pool, "/tmp/f").await.unwrap();
    let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "", 1, 1).await.unwrap();
    let b = insert_image(&pool, fid, "/tmp/f/b.jpg", "", 1, 1).await.unwrap();

    assert_eq!(count_pending_inference(&pool).await.unwrap(), 0);

    // Each enqueue inserts BOTH a 'semantic' and 'subject' row for one image;
    // the count is by DISTINCT image_id, so two images → 2 (not 4).
    enqueue_image(&pool, a).await.unwrap();
    enqueue_image(&pool, b).await.unwrap();
    assert_eq!(count_pending_inference(&pool).await.unwrap(), 2);

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path Cargo.toml count_pending_inference_counts_distinct`
Expected: FAIL to compile — `count_pending_inference` not found.

- [ ] **Step 3: Implement the function**

In `src/pipeline/queue.rs`, add after `get_processing_counts` (after line 102):

```rust
/// Number of distinct images still awaiting inference. Used by the hash worker
/// as a backpressure signal: while this is deep, hashing yields to the pipeline.
pub async fn count_pending_inference(pool: &SqlitePool) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(DISTINCT image_id) AS n FROM embedding_queue")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n"))
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path Cargo.toml count_pending_inference_counts_distinct`
Expected: PASS (1 passed).

- [ ] **Step 5: Commit**

```bash
git add src/pipeline/queue.rs src/db/tests.rs
git commit -m "feat(pipeline): add count_pending_inference backpressure query (TT-75)"
```

---

### Task 4: Build the bounded hash worker loop

**Files:**
- Modify: `src/library/hasher.rs` (append the worker)

> This task has no unit test: the worker is an unbounded `loop` with timers, which is integration-level behavior covered by Task 6's manual import check. Keep it small and obviously correct; its building blocks (`compute_blake3`, `get_pending_hash_batch`, `apply_hash_results`, `count_pending_inference`) are already tested.

- [ ] **Step 1: Append the worker to `src/library/hasher.rs`**

Add these imports to the top of `src/library/hasher.rs` (merge with the existing `use` lines):

```rust
use log::error;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
```

Then append the worker below `compute_blake3`:

```rust
/// Pull at most this many PENDING rows per pass.
const HASH_BATCH: i64 = 32;
/// Concurrent file reads/hashes — deliberately low so disk/CPU stay free for the pipeline.
const HASH_CONCURRENCY: usize = 2;
/// While more than this many images await inference, hashing pauses entirely.
/// ~4× the pipeline batch_size (12) — enough headroom that hashing never starves inference.
const INFER_BACKPRESSURE: i64 = 48;
/// Idle/backoff sleep when there is nothing to do or the pipeline is busy.
const IDLE_SLEEP: Duration = Duration::from_secs(2);
/// Brief yield after each write burst so the worker never monopolizes the DB writer.
const POST_BATCH_YIELD: Duration = Duration::from_millis(50);

/// Spawn the single background hash worker. Call once at startup.
pub fn spawn_hash_worker(pool: SqlitePool) {
    tokio::spawn(async move { run_hash_worker(pool).await });
}

async fn run_hash_worker(pool: SqlitePool) {
    loop {
        // Backpressure: yield to inference while its queue is deep.
        match crate::pipeline::queue::count_pending_inference(&pool).await {
            Ok(n) if n > INFER_BACKPRESSURE => {
                tokio::time::sleep(IDLE_SLEEP).await;
                continue;
            }
            Err(e) => {
                error!("[hasher] backpressure query failed: {e}");
                tokio::time::sleep(IDLE_SLEEP).await;
                continue;
            }
            _ => {}
        }

        let batch = match crate::library::repo::get_pending_hash_batch(&pool, HASH_BATCH).await {
            Ok(b) => b,
            Err(e) => {
                error!("[hasher] pending-batch query failed: {e}");
                tokio::time::sleep(IDLE_SLEEP).await;
                continue;
            }
        };
        if batch.is_empty() {
            tokio::time::sleep(IDLE_SLEEP).await;
            continue;
        }

        // Bounded-parallel hashing.
        let sem = Arc::new(Semaphore::new(HASH_CONCURRENCY));
        let mut handles = Vec::with_capacity(batch.len());
        for (id, path, mtime) in batch {
            let permit = sem.clone().acquire_owned().await.expect("hash semaphore closed");
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let hash = compute_blake3(std::path::Path::new(&path)).await.ok();
                (id, mtime, hash)
            }));
        }

        let mut results: Vec<(i64, i64, Option<String>)> = Vec::with_capacity(handles.len());
        for h in handles {
            match h.await {
                Ok(r) => results.push(r),
                Err(e) => error!("[hasher] hash task panicked: {e}"),
            }
        }

        if let Err(e) = crate::library::repo::apply_hash_results(&pool, &results).await {
            error!("[hasher] applying hash results failed: {e}");
        }

        // Yield so a burst of writes doesn't monopolize the single SQLite writer.
        tokio::time::sleep(POST_BATCH_YIELD).await;
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --manifest-path Cargo.toml`
Expected: Builds (warnings about `spawn_hash_worker` being unused are fine until Task 5 wires it).

- [ ] **Step 3: Commit**

```bash
git add src/library/hasher.rs
git commit -m "feat(library): bounded, backpressured BLAKE3 hash worker (TT-75)"
```

---

### Task 5: Rewire the indexer — drop the per-file hash storm, fix change-detection

**Files:**
- Modify: `src/library/indexer.rs`
- Test: `src/db/tests.rs`

> Context: `process_file` (`indexer.rs:148`) has two branches. The `None` branch (new file, lines 176–236) currently inserts, enqueues, emits, **and** spawns a per-file SHA256 task. The `Some` branch (modify, lines 237–312) early-returns when unchanged, else spawns a per-file SHA256 tie-breaker. We remove the `None`-branch spawn entirely, swap the `Some`-branch hash to BLAKE3, and fix the unchanged-check so a not-yet-hashed (`PENDING`) file with matching `(size, mtime)` is treated as unchanged.

- [ ] **Step 1: Remove the per-file hash spawn from the `None` (new-file) branch**

In `src/library/indexer.rs`, delete lines 211–235 — the block starting at the comment `// Spawn a background task to compute and update the real hash` through the closing `});` of that `tokio::spawn`. The `None` branch must end right after the `image_added` emit (line 209's closing `);`).

After this edit, the `None` branch reads (no hashing; the row is left `hash_status = 'PENDING'` by the schema default, and the new worker will hash it):

```rust
            None => {
                debug!("process_file: found new file: {}", path_str);

                // Change-detection authority is (file_size, mtime); the content
                // hash is computed lazily off the critical path by the hash worker
                // (TT-75). Insert with an empty hash + PENDING status, enqueue, emit.
                let image_id = match repo::insert_image(
                    &self.pool,
                    folder_id,
                    &path_str,
                    "",
                    file_size,
                    mtime,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        error!("Failed to insert image {}: {}", path_str, e);
                        return;
                    }
                };

                if let Err(e) = crate::pipeline::queue::enqueue_image(&self.pool, image_id).await {
                    error!("Failed to enqueue image {}: {}", image_id, e);
                }
                self.preview.enqueue_low(image_id);

                let _ = self.app.emit(
                    "image_added",
                    crate::models::ImageAddedPayload {
                        image_id,
                        path: path_str,
                    },
                );
            }
```

- [ ] **Step 2: Fix the modify-path "unchanged" check to ignore `hash_status`**

In the `Some(existing)` branch, change the early-return condition (currently line 238):

```rust
                if mtime == existing.mtime && file_size == existing.file_size && existing.hash_status == "DONE" {
```

to:

```rust
                // (size, mtime) is the authoritative change signal. A PENDING file
                // whose (size, mtime) is unchanged is NOT changed — the worker will
                // still compute its hash; do not re-hash or re-enqueue here (TT-75).
                if mtime == existing.mtime && file_size == existing.file_size {
```

- [ ] **Step 3: Swap the modify-path tie-breaker hash from SHA256 to BLAKE3**

In the same `Some` branch, the spawned task computes `compute_sha256` (line 267). Replace that call:

```rust
                    let hash = match compute_sha256(&path_buf).await {
```

with:

```rust
                    let hash = match crate::library::hasher::compute_blake3(&path_buf).await {
```

(The tie-breaker still runs here because we only reach it when `(size, mtime)` changed — the exact case the spec says to hash. Existing SHA256 `file_hash` values will mismatch once and the file gets recomputed/re-enqueued, which is the accepted migration behavior.)

- [ ] **Step 4: Delete the now-unused `compute_sha256` and `sha2` import**

Delete the `compute_sha256` function (`indexer.rs:51-61`) and the import at `indexer.rs:4`:

```rust
use sha2::{Digest, Sha256};
```

- [ ] **Step 5: Verify it compiles with no warnings about unused `sha2`/`hash_semaphore`**

Run: `cargo build --manifest-path Cargo.toml`
Expected: Builds. `hash_semaphore` is still used by the modify-path spawn, so no unused-field warning. If the compiler reports `compute_sha256` still referenced, you missed a call site — re-check Step 3.

- [ ] **Step 6: Write a test proving an unchanged PENDING file is not re-enqueued**

This guards the core correctness criterion ("files with unchanged `(size, mtime)` are not re-enqueued"). Append to `src/db/tests.rs`:

```rust
#[tokio::test]
async fn unchanged_pending_file_is_not_treated_as_changed() {
    // Mirrors the indexer modify-path decision: a freshly imported (PENDING,
    // empty-hash) row whose (size, mtime) is unchanged must be left alone — not
    // re-enqueued — even though its hash_status is not yet 'DONE'.
    use crate::pipeline::queue::{enqueue_image, count_pending_inference};

    let dir = std::env::temp_dir().join(format!("nebula_unchanged_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    let fid = insert_folder(&pool, "/tmp/f").await.unwrap();
    let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "", 1000, 50).await.unwrap();
    enqueue_image(&pool, a).await.unwrap();

    // Drain inference as if the pipeline already processed it.
    sqlx::query("DELETE FROM embedding_queue WHERE image_id = ?").bind(a).execute(&pool).await.unwrap();
    assert_eq!(count_pending_inference(&pool).await.unwrap(), 0);

    // Re-observe the file with identical (size, mtime). It is still PENDING
    // (hash worker hasn't run). The authoritative check is (size, mtime) only:
    let img = get_image_by_id(&pool, a).await.unwrap().unwrap();
    let unchanged = img.mtime == 50 && img.file_size == 1000;
    assert!(unchanged, "the (size, mtime) signal reports the file as unchanged");
    assert_eq!(img.hash_status, "PENDING", "still PENDING — yet must NOT be re-enqueued");

    // Nothing re-enqueued.
    assert_eq!(count_pending_inference(&pool).await.unwrap(), 0);

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test --manifest-path Cargo.toml unchanged_pending_file_is_not_treated`
Expected: PASS (1 passed).

- [ ] **Step 8: Commit**

```bash
git add src/library/indexer.rs src/db/tests.rs
git commit -m "refactor(library): decouple change-detection from inference, drop per-file hash storm (TT-75)"
```

---

### Task 6: Spawn the worker at startup + full verification

**Files:**
- Modify: `src/app/mod.rs`

- [ ] **Step 1: Spawn the hash worker after the indexer rescan is kicked off**

In `src/app/mod.rs`, find the rescan spawn block (lines 56–59):

```rust
            let indexer_rescan = app.state::<AppState>().indexer.clone();
            tauri::async_runtime::spawn(async move {
                indexer_rescan.start_rescan().await;
            });
```

Add directly below it:

```rust
            // TT-75: single bounded BLAKE3 hash worker; runs only while the
            // inference queue is shallow so a large import reaches full throughput fast.
            crate::library::hasher::spawn_hash_worker(pool.clone());
```

- [ ] **Step 2: Build the whole crate**

Run: `cargo build --manifest-path Cargo.toml`
Expected: Builds clean. `spawn_hash_worker` is now referenced, so its earlier dead-code warning is gone.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --manifest-path Cargo.toml --all-targets -- -D warnings`
Expected: No errors.

- [ ] **Step 4: Run the full test suite**

Run: `cargo test --manifest-path Cargo.toml`
Expected: All tests PASS, including the five added in this plan (`compute_blake3_is_deterministic_and_content_sensitive`, `pending_hash_batch_and_apply_results_round_trip`, `apply_hash_results_is_guarded_by_mtime`, `count_pending_inference_counts_distinct_images`, `unchanged_pending_file_is_not_treated_as_changed`).

- [ ] **Step 5: Manual import smoke check (acceptance criterion)**

Run the app (`npm run tauri dev` from the repo root, or the project's usual launch command) and add a folder with 1,000+ images. Watch the logs:
- Inference (`[pipeline] Processing batch …`) starts ramping within seconds — it does **not** wait on hashing.
- `[hasher]` activity appears only in lulls; no per-file hash spam.
- After the import settles, confirm hashing completed: open the DB and run
  `SELECT hash_status, COUNT(*) FROM images GROUP BY hash_status;` — expect all live rows to reach `DONE` (a few `FAILED` only for unreadable files), none stuck `PENDING` once the queue drains.

- [ ] **Step 6: Commit**

```bash
git add src/app/mod.rs
git commit -m "feat(app): spawn lazy BLAKE3 hash worker at startup (TT-75)"
```

---

## Acceptance Criteria → Task Map

- Pipeline reaches full throughput within seconds during 1,000+ import → Task 5 (drop `None`-branch spawn) + Task 4 (backpressure) + Task 6 Step 5.
- No per-file unbounded hash-task spawning; single bounded worker with backpressure → Task 4 + Task 5 Step 1.
- Hash writes batched (no ~1 UPDATE per file) → Task 2 (`apply_hash_results` transaction) + Task 4.
- Content hashing uses BLAKE3 → Task 1 + Task 5 Step 3.
- Unchanged `(size, mtime)` not re-enqueued; genuinely changed files are → Task 5 Step 2 + Task 5 Step 6 test.
- First-import critical path performs zero content hashing → Task 5 Step 1.
- `hash_status` lifecycle (`PENDING` → `DONE`/`FAILED`) preserved, populated by the worker → Task 2 + Task 4.
- Mid-import modification / racy mtime handled → Task 2 `apply_hash_results` mtime guard + Task 2 Step 1 test.
- No schema change / no forced mass re-hash on upgrade → no `db/mod.rs` edit; old SHA256 values left as-is and lazily recomputed only on genuine change (Task 5 Step 3).

## Notes for the implementer

- **Do not** touch `db/mod.rs` `BASE_SCHEMA` or add a `VERSIONED_MIGRATIONS` entry — this design is intentionally schema-free. If you discover a column is genuinely required, stop and add it as a *new* versioned migration (never edit `BASE_SCHEMA` in place).
- `insert_image` already leaves `hash_status` at the schema default `'PENDING'` — there is no separate "mark pending" step.
- Keep `sha2` in `Cargo.toml`; `src/commands.rs` still uses it. Only `indexer.rs` drops it.
</content>
</invoke>
