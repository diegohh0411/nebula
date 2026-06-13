use anyhow::Result;
use sqlx::{Row, SqlitePool};

pub async fn get_image_embedding(pool: &SqlitePool, id: i64) -> Result<Option<Vec<u8>>> {
    let row = sqlx::query("SELECT embedding FROM images WHERE id = ? AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.get::<Option<Vec<u8>>, _>("embedding")))
}

pub async fn get_all_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT id, embedding FROM images
         WHERE semantic_analysis_done = 1 AND deleted_at IS NULL AND embedding IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let id: i64 = r.get("id");
            let emb: Option<Vec<u8>> = r.get("embedding");
            emb.map(|e| (id, e))
        })
        .collect())
}

pub async fn get_cached_embedding(
    pool: &SqlitePool,
    cache_key: &str,
    query_type: &str,
) -> Result<Option<Vec<u8>>> {
    let cutoff = chrono::Utc::now().timestamp() - 1800;
    let row = sqlx::query(
        "SELECT embedding FROM embedding_cache WHERE cache_key = ? AND query_type = ? AND created_at > ?"
    )
    .bind(cache_key)
    .bind(query_type)
    .bind(cutoff)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get("embedding")))
}

pub async fn insert_cached_embedding(
    pool: &SqlitePool,
    cache_key: &str,
    query_type: &str,
    embedding: &[u8],
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR REPLACE INTO embedding_cache (cache_key, query_type, embedding, created_at) VALUES (?, ?, ?, ?)"
    )
    .bind(cache_key)
    .bind(query_type)
    .bind(embedding)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_stale_cache_entries(pool: &SqlitePool) -> Result<u64> {
    let cutoff = chrono::Utc::now().timestamp() - 1800;
    let result = sqlx::query("DELETE FROM embedding_cache WHERE created_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

pub async fn reset_all_embeddings(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE images SET embedding = NULL, semantic_analysis_done = 0, subject_analysis_done = 0 WHERE deleted_at IS NULL")
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM face_vectors")
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM embedding_cache")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM embedding_queue")
        .execute(&mut *tx)
        .await?;

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at)
         SELECT id, 'semantic', 0, ? FROM images WHERE deleted_at IS NULL
         UNION ALL
         SELECT id, 'subject', 0, ? FROM images WHERE deleted_at IS NULL",
    )
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
