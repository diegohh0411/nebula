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

pub async fn delete_folder(pool: &SqlitePool, data_dir: &std::path::Path, id: i64) -> Result<Vec<i64>> {
    let faces = sqlx::query("SELECT id FROM faces WHERE image_id IN (SELECT id FROM images WHERE folder_id = ?)")
        .bind(id)
        .fetch_all(pool)
        .await?;
    let face_ids: Vec<i64> = faces.iter().map(|r| r.get::<i64, _>("id")).collect();

    let images = sqlx::query("SELECT id, thumbnail_path, preview_path FROM images WHERE folder_id = ?")
        .bind(id)
        .fetch_all(pool)
        .await?;

    let image_ids: Vec<i64> = images.iter().map(|r| r.get::<i64, _>("id")).collect();

    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE subjects SET thumbnail_face_id = NULL WHERE thumbnail_face_id IN (SELECT id FROM faces WHERE image_id IN (SELECT id FROM images WHERE folder_id = ?))"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM face_vectors WHERE rowid IN (SELECT id FROM faces WHERE image_id IN (SELECT id FROM images WHERE folder_id = ?))"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM faces WHERE image_id IN (SELECT id FROM images WHERE folder_id = ?)"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM subjects WHERE id NOT IN (SELECT DISTINCT subject_id FROM faces WHERE subject_id IS NOT NULL)"
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM images WHERE folder_id = ?"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "DELETE FROM folders WHERE id = ?"
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // Cleanup face crop cache files
    let face_crop_dir = crate::platform::paths::face_crop_cache_dir(data_dir);
    for face_id in face_ids {
        let path = face_crop_dir.join(format!("{}.webp", face_id));
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }

    // Cleanup images thumbnail/preview cache files
    for row in images {
        if let Some(thumb) = row.get::<Option<String>, _>("thumbnail_path") {
            let path = std::path::PathBuf::from(thumb);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
        if let Some(prev) = row.get::<Option<String>, _>("preview_path") {
            let path = std::path::PathBuf::from(prev);
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    Ok(image_ids)
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

/// A batch of images still awaiting a content hash. Returns `(id, path, mtime)`.
/// Excludes soft-deleted rows; ordered by id so progress is FIFO and stable.
pub async fn get_pending_hash_batch(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<(i64, String, i64)>> {
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
        .map(|r| {
            (
                r.get::<i64, _>("id"),
                r.get::<String, _>("path"),
                r.get::<i64, _>("mtime"),
            )
        })
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
