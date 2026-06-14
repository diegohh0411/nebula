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

pub async fn mark_failed(
    pool: &SqlitePool,
    queue_id: i64,
    attempts: i32,
    error: &str,
) -> Result<()> {
    let new_attempts = attempts + 1;
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
