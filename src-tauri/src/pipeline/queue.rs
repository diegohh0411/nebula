use anyhow::Result;
use sqlx::{Row, SqlitePool};

use crate::models::ProcessingStatus;

pub async fn enqueue_image(pool: &SqlitePool, image_id: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("DELETE FROM embedding_queue WHERE image_id = ?")
        .bind(image_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at) VALUES (?, 'semantic', 0, ?)")
        .bind(image_id)
        .bind(now)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at) VALUES (?, 'subject', 0, ?)")
        .bind(image_id)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_queue_batch(
    pool: &SqlitePool,
    pipeline: &str,
    limit: i64,
) -> Result<Vec<(i64, i64, i32)>> {
    let now = chrono::Utc::now().timestamp();
    let rows = sqlx::query(
        "SELECT id, image_id, attempts FROM embedding_queue
         WHERE pipeline = ? AND scheduled_at <= ? ORDER BY scheduled_at ASC LIMIT ?",
    )
    .bind(pipeline)
    .bind(now)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<i64, _>("id"),
                r.get::<i64, _>("image_id"),
                r.get::<i32, _>("attempts"),
            )
        })
        .collect())
}

pub async fn mark_semantic_analysis_done(
    pool: &SqlitePool,
    queue_id: i64,
    image_id: i64,
    embedding: &[u8],
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE images SET embedding = ?, semantic_analysis_done = 1, updated_at = ? WHERE id = ?",
    )
    .bind(embedding)
    .bind(now)
    .bind(image_id)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM embedding_queue WHERE id = ?")
        .bind(queue_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_subject_analysis_done(
    pool: &SqlitePool,
    queue_id: i64,
    image_id: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE images SET subject_analysis_done = 1, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(image_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM embedding_queue WHERE id = ?")
        .bind(queue_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Retry cap for the inference queue. Once an entry has failed this many times
/// it is dead-lettered (removed) instead of rescheduled, so a file that can
/// never be decoded — e.g. it was moved or deleted on disk after indexing —
/// stops being retried forever and no longer dominates the head of the queue.
pub const MAX_QUEUE_ATTEMPTS: i32 = 6;

/// What `mark_failed` did with the entry, so the caller can log accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOutcome {
    /// Rescheduled for a later retry with exponential backoff.
    Retrying,
    /// Attempts exhausted; the entry was removed from the queue.
    DeadLettered,
}

pub async fn mark_failed(
    pool: &SqlitePool,
    queue_id: i64,
    attempts: i32,
    error: &str,
) -> Result<FailureOutcome> {
    let new_attempts = attempts + 1;
    if new_attempts >= MAX_QUEUE_ATTEMPTS {
        dead_letter(pool, queue_id).await?;
        return Ok(FailureOutcome::DeadLettered);
    }
    let backoff_exponent = std::cmp::min(new_attempts.max(0) as u32, 10);
    let backoff = std::cmp::min(2_i64.pow(backoff_exponent) * 30, 28800);
    let scheduled_at = chrono::Utc::now().timestamp() + backoff;
    sqlx::query(
        "UPDATE embedding_queue SET attempts = ?, last_error = ?, scheduled_at = ? WHERE id = ?",
    )
    .bind(new_attempts)
    .bind(error)
    .bind(scheduled_at)
    .bind(queue_id)
    .execute(pool)
    .await?;
    Ok(FailureOutcome::Retrying)
}

/// Permanently remove a single queue entry. Used when attempts are exhausted
/// and when the source file is known to be gone from disk, so the entry stops
/// being retried and no longer blocks images behind it.
pub async fn dead_letter(pool: &SqlitePool, queue_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM embedding_queue WHERE id = ?")
        .bind(queue_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_processing_counts(pool: &SqlitePool) -> Result<ProcessingStatus> {
    let row = sqlx::query(
        "SELECT
           (SELECT COUNT(DISTINCT image_id) FROM embedding_queue) as total_pending,
           (SELECT COUNT(*) FROM images WHERE semantic_analysis_done = 1 AND subject_analysis_done = 1 AND deleted_at IS NULL) as done",
    )
    .fetch_one(pool)
    .await?;
    Ok(ProcessingStatus {
        total_pending: row.get("total_pending"),
        done: row.get("done"),
    })
}

/// Number of distinct images still awaiting inference. Used by the hash worker
/// as a backpressure signal: while this is deep, hashing yields to the pipeline.
pub async fn count_pending_inference(pool: &SqlitePool) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(DISTINCT image_id) AS n FROM embedding_queue")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal in-memory queue table (no images FK) to isolate queue policy.
    async fn queue_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE embedding_queue (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 image_id     INTEGER NOT NULL,
                 pipeline     TEXT NOT NULL DEFAULT 'semantic',
                 attempts     INTEGER NOT NULL DEFAULT 0,
                 last_error   TEXT,
                 scheduled_at INTEGER NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert_entry(pool: &SqlitePool, attempts: i32) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO embedding_queue (image_id, attempts, scheduled_at)
             VALUES (1, ?, 0) RETURNING id",
        )
        .bind(attempts)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn row_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM embedding_queue")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn mark_failed_reschedules_below_the_cap() {
        let pool = queue_pool().await;
        let id = insert_entry(&pool, 0).await;

        let outcome = mark_failed(&pool, id, 0, "boom").await.unwrap();
        assert_eq!(outcome, FailureOutcome::Retrying);

        // Row still present, attempts incremented, error recorded, deferred.
        assert_eq!(row_count(&pool).await, 1);
        let (attempts, err, scheduled): (i32, String, i64) = sqlx::query_as(
            "SELECT attempts, last_error, scheduled_at FROM embedding_queue WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(err, "boom");
        assert!(scheduled > chrono::Utc::now().timestamp());
    }

    #[tokio::test]
    async fn mark_failed_dead_letters_at_the_cap() {
        let pool = queue_pool().await;
        // Already one attempt short of the cap; the next failure exhausts it.
        let id = insert_entry(&pool, MAX_QUEUE_ATTEMPTS - 1).await;

        let outcome = mark_failed(&pool, id, MAX_QUEUE_ATTEMPTS - 1, "gone")
            .await
            .unwrap();
        assert_eq!(outcome, FailureOutcome::DeadLettered);
        // Entry removed so it stops being retried and blocking the queue head.
        assert_eq!(row_count(&pool).await, 0);
    }

    #[tokio::test]
    async fn dead_letter_removes_the_entry_immediately() {
        let pool = queue_pool().await;
        let id = insert_entry(&pool, 0).await;
        dead_letter(&pool, id).await.unwrap();
        assert_eq!(row_count(&pool).await, 0);
    }
}
