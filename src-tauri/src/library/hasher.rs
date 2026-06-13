//! Bounded, low-priority content-hash worker (TT-75).
//!
//! Change-detection authority is `(file_size, mtime)`; the content hash is only
//! a tie-breaker. We use BLAKE3 (non-cryptographic strength is sufficient and it
//! is ~5–10× cheaper than SHA256) and compute it off the import critical path.

use anyhow::Result;
use log::error;
use sqlx::SqlitePool;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

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
