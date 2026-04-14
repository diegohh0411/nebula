use anyhow::Result;
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Row, SqlitePool};
use std::path::Path;

use crate::models::{EmbedStatus, Folder, FolderWithCount, Image};

const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS folders (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    path     TEXT UNIQUE NOT NULL,
    added_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS images (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_id      INTEGER NOT NULL REFERENCES folders(id),
    path           TEXT UNIQUE NOT NULL,
    file_hash      TEXT NOT NULL,
    date_taken     INTEGER,
    date_file      INTEGER NOT NULL,
    thumbnail_path TEXT,
    embed_status   TEXT NOT NULL DEFAULT 'pending',
    embedding      BLOB,
    added_at       INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    deleted_at     INTEGER
);

CREATE INDEX IF NOT EXISTS idx_images_folder ON images(folder_id);
CREATE INDEX IF NOT EXISTS idx_images_embed ON images(embed_status) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS embedding_queue (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id     INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    scheduled_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_queue_scheduled ON embedding_queue(scheduled_at);
"#;

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

    for stmt in MIGRATIONS.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            sqlx::query(s).execute(&pool).await?;
        }
    }
    Ok(pool)
}

pub async fn insert_folder(pool: &SqlitePool, path: &str) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query("INSERT OR IGNORE INTO folders (path, added_at) VALUES (?, ?)")
        .bind(path)
        .bind(now)
        .execute(pool)
        .await?;

    if result.rows_affected() > 0 {
        Ok(result.last_insert_rowid())
    } else {
        let row = sqlx::query("SELECT id FROM folders WHERE path = ?")
            .bind(path)
            .fetch_one(pool)
            .await?;
        Ok(row.get::<i64, _>("id"))
    }
}

pub async fn delete_folder(pool: &SqlitePool, id: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE images SET deleted_at = ? WHERE folder_id = ? AND deleted_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM folders WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_folders_with_counts(pool: &SqlitePool) -> Result<Vec<FolderWithCount>> {
    let rows = sqlx::query(
        r#"SELECT f.id, f.path, f.added_at,
                  COUNT(i.id) as photo_count
           FROM folders f
           LEFT JOIN images i ON i.folder_id = f.id AND i.deleted_at IS NULL
           GROUP BY f.id
           ORDER BY f.added_at DESC"#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| FolderWithCount {
            id: r.get("id"),
            path: r.get("path"),
            added_at: r.get("added_at"),
            photo_count: r.get("photo_count"),
        })
        .collect())
}

pub async fn list_all_folders(pool: &SqlitePool) -> Result<Vec<Folder>> {
    let rows = sqlx::query("SELECT id, path, added_at FROM folders ORDER BY added_at ASC")
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| Folder {
            id: r.get("id"),
            path: r.get("path"),
            added_at: r.get("added_at"),
        })
        .collect())
}

fn row_to_image(r: &sqlx::sqlite::SqliteRow) -> Image {
    Image {
        id: r.get("id"),
        folder_id: r.get("folder_id"),
        path: r.get("path"),
        file_hash: r.get("file_hash"),
        date_taken: r.get("date_taken"),
        date_file: r.get("date_file"),
        thumbnail_path: r.get("thumbnail_path"),
        embed_status: r.get("embed_status"),
        added_at: r.get("added_at"),
        updated_at: r.get("updated_at"),
        deleted_at: r.get("deleted_at"),
    }
}

pub async fn upsert_image(
    pool: &SqlitePool,
    folder_id: i64,
    path: &str,
    file_hash: &str,
    date_file: i64,
) -> Result<(i64, bool)> {
    let now = chrono::Utc::now().timestamp();

    let existing = sqlx::query("SELECT id, file_hash, deleted_at FROM images WHERE path = ?")
        .bind(path)
        .fetch_optional(pool)
        .await?;

    if let Some(row) = existing {
        let image_id: i64 = row.get("id");
        let old_hash: String = row.get("file_hash");
        let deleted_at: Option<i64> = row.get("deleted_at");
        let was_deleted = deleted_at.is_some();
        let hash_changed = old_hash != file_hash;

        if hash_changed || was_deleted {
            sqlx::query(
                "UPDATE images SET file_hash = ?, date_file = ?, embed_status = 'pending',
                 embedding = NULL, updated_at = ?, deleted_at = NULL WHERE id = ?",
            )
            .bind(file_hash)
            .bind(date_file)
            .bind(now)
            .bind(image_id)
            .execute(pool)
            .await?;
            return Ok((image_id, true));
        }
        Ok((image_id, false))
    } else {
        let result = sqlx::query(
            "INSERT INTO images (folder_id, path, file_hash, date_file, embed_status, added_at, updated_at)
             VALUES (?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(folder_id)
        .bind(path)
        .bind(file_hash)
        .bind(date_file)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        Ok((result.last_insert_rowid(), true))
    }
}

pub async fn soft_delete_image(pool: &SqlitePool, path: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE images SET deleted_at = ? WHERE path = ? AND deleted_at IS NULL")
        .bind(now)
        .bind(path)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_thumbnail_path(pool: &SqlitePool, image_id: i64, thumb_path: &str) -> Result<()> {
    sqlx::query("UPDATE images SET thumbnail_path = ? WHERE id = ?")
        .bind(thumb_path)
        .bind(image_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_images(pool: &SqlitePool, folder_id: Option<i64>) -> Result<Vec<Image>> {
    let rows = if let Some(fid) = folder_id {
        sqlx::query(
            "SELECT id, folder_id, path, file_hash, date_taken, date_file, thumbnail_path,
                    embed_status, added_at, updated_at, deleted_at
             FROM images WHERE folder_id = ? AND deleted_at IS NULL
             ORDER BY COALESCE(date_taken, date_file) DESC",
        )
        .bind(fid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, folder_id, path, file_hash, date_taken, date_file, thumbnail_path,
                    embed_status, added_at, updated_at, deleted_at
             FROM images WHERE deleted_at IS NULL
             ORDER BY COALESCE(date_taken, date_file) DESC",
        )
        .fetch_all(pool)
        .await?
    };

    Ok(rows.iter().map(row_to_image).collect())
}

pub async fn get_image_by_path(pool: &SqlitePool, path: &str) -> Result<Option<Image>> {
    let row = sqlx::query(
        "SELECT id, folder_id, path, file_hash, date_taken, date_file, thumbnail_path,
                embed_status, added_at, updated_at, deleted_at
         FROM images WHERE path = ?",
    )
    .bind(path)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_image))
}

pub async fn get_image_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Image>> {
    let row = sqlx::query(
        "SELECT id, folder_id, path, file_hash, date_taken, date_file, thumbnail_path,
                embed_status, added_at, updated_at, deleted_at
         FROM images WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_image))
}

pub async fn enqueue_image(pool: &SqlitePool, image_id: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("DELETE FROM embedding_queue WHERE image_id = ?")
        .bind(image_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO embedding_queue (image_id, attempts, scheduled_at) VALUES (?, 0, ?)")
        .bind(image_id)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_queue_batch(pool: &SqlitePool, limit: i64) -> Result<Vec<(i64, i64, i32)>> {
    let now = chrono::Utc::now().timestamp();
    let rows = sqlx::query(
        "SELECT id, image_id, attempts FROM embedding_queue
         WHERE scheduled_at <= ? ORDER BY scheduled_at ASC LIMIT ?",
    )
    .bind(now)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<i64, _>("image_id"), r.get::<i32, _>("attempts")))
        .collect())
}

pub async fn mark_embedded(pool: &SqlitePool, image_id: i64, embedding: &[u8]) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE images SET embedding = ?, embed_status = 'done', updated_at = ? WHERE id = ?",
    )
    .bind(embedding)
    .bind(now)
    .bind(image_id)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM embedding_queue WHERE image_id = ?")
        .bind(image_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_failed(pool: &SqlitePool, queue_id: i64, attempts: i32, error: &str) -> Result<()> {
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

pub async fn get_all_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT id, embedding FROM images
         WHERE embed_status = 'done' AND deleted_at IS NULL AND embedding IS NOT NULL",
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

pub async fn get_embed_counts(pool: &SqlitePool) -> Result<EmbedStatus> {
    let row = sqlx::query(
        "SELECT
           (SELECT COUNT(*) FROM embedding_queue) as pending,
           (SELECT COUNT(*) FROM images WHERE embed_status = 'done' AND deleted_at IS NULL) as done",
    )
    .fetch_one(pool)
    .await?;
    Ok(EmbedStatus {
        pending: row.get("pending"),
        done: row.get("done"),
    })
}
