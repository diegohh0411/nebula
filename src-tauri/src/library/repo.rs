//! Library persistence: folders + images.
use crate::library::models::{DbImage, Folder, FolderWithCount, Image};
use anyhow::Result;
use sqlx::{Row, SqlitePool};

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

pub(crate) fn row_to_image(r: &sqlx::sqlite::SqliteRow) -> Image {
    Image {
        id: r.get("id"),
        folder_id: r.get("folder_id"),
        path: r.get("path"),
        file_hash: r.get("file_hash"),
        hash_status: r.get("hash_status"),
        file_size: r.get::<i64, _>("file_size"),
        date_taken: r.get("date_taken"),
        mtime: r.get("mtime"),
        thumbnail_path: r.get("thumbnail_path"),
        preview_path: r.get("preview_path"),
        semantic_analysis_done: r.get::<i32, _>("semantic_analysis_done") != 0,
        subject_analysis_done: r.get::<i32, _>("subject_analysis_done") != 0,
        added_at: r.get("added_at"),
        updated_at: r.get("updated_at"),
        deleted_at: r.get("deleted_at"),
    }
}

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
        "UPDATE images SET file_hash = ?, hash_status = 'DONE', file_size = ?, mtime = ?,
         semantic_analysis_done = 0, subject_analysis_done = 0, embedding = NULL,
         thumbnail_path = NULL, preview_path = NULL,
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
        "UPDATE images SET hash_status = 'DONE', file_size = ?, mtime = ?, updated_at = ?, deleted_at = NULL WHERE id = ?",
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

pub async fn get_all_images_for_rescan(pool: &SqlitePool) -> Result<Vec<DbImage>> {
    let rows = sqlx::query(
        "SELECT id, path, mtime, file_size, file_hash, hash_status, deleted_at FROM images",
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
            hash_status: r.get("hash_status"),
            deleted_at: r.get("deleted_at"),
        })
        .collect())
}

pub async fn get_image_metadata_by_path(pool: &SqlitePool, path: &str) -> Result<Option<DbImage>> {
    let row = sqlx::query(
        "SELECT id, path, mtime, file_size, file_hash, hash_status, deleted_at FROM images WHERE path = ?",
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
        hash_status: r.get("hash_status"),
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

pub async fn soft_delete_image(pool: &SqlitePool, path: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE images SET deleted_at = ? WHERE path = ? AND deleted_at IS NULL")
        .bind(now)
        .bind(path)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_thumbnail_path(
    pool: &SqlitePool,
    image_id: i64,
    thumb_path: &str,
) -> Result<()> {
    sqlx::query("UPDATE images SET thumbnail_path = ? WHERE id = ?")
        .bind(thumb_path)
        .bind(image_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_preview_path(
    pool: &SqlitePool,
    image_id: i64,
    preview_path: &str,
) -> Result<()> {
    sqlx::query("UPDATE images SET preview_path = ? WHERE id = ?")
        .bind(preview_path)
        .bind(image_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Ids of non-deleted images that still lack an 800px thumbnail.
pub async fn images_needing_preview(pool: &SqlitePool) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT id FROM images
         WHERE thumbnail_path IS NULL AND deleted_at IS NULL
         ORDER BY COALESCE(date_taken, mtime) DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>("id")).collect())
}

pub async fn list_images(pool: &SqlitePool, folder_id: Option<i64>) -> Result<Vec<Image>> {
    let rows = if let Some(fid) = folder_id {
        sqlx::query(
            "SELECT id, folder_id, path, file_hash, hash_status, file_size, date_taken, mtime, thumbnail_path, preview_path,
                    semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
             FROM images WHERE folder_id = ? AND deleted_at IS NULL
             ORDER BY COALESCE(date_taken, mtime) DESC",
        )
        .bind(fid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, folder_id, path, file_hash, hash_status, file_size, date_taken, mtime, thumbnail_path, preview_path,
                    semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
             FROM images WHERE deleted_at IS NULL
             ORDER BY COALESCE(date_taken, mtime) DESC",
        )
        .fetch_all(pool)
        .await?
    };

    Ok(rows.iter().map(row_to_image).collect())
}

pub async fn get_image_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Image>> {
    let row = sqlx::query(
        "SELECT id, folder_id, path, file_hash, hash_status, file_size, date_taken, mtime, thumbnail_path, preview_path,
                semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
         FROM images WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_image))
}
