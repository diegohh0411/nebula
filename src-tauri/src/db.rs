use anyhow::Result;
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Row, SqlitePool};
use std::path::Path;

use crate::models::{ProcessingStatus, Folder, FolderWithCount, Image, Face, Subject};

const BASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
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
    preview_path           TEXT,
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
    score REAL NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_merge_pair ON merge_suggestions(
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END
);
"#;

const VERSIONED_MIGRATIONS: &[(u32, &str)] = &[
    (1, "
        DROP TABLE IF EXISTS merge_suggestions;
        CREATE TABLE merge_suggestions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            score REAL NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_merge_pair ON merge_suggestions(
            CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
            CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END
        );
    "),
    (2, "
        CREATE TABLE IF NOT EXISTS dismissed_pairs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            dismissed_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_dismissed_pair ON dismissed_pairs(subject_id_a, subject_id_b);
    "),
];

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

    let current: u32 = sqlx::query_scalar("SELECT version FROM schema_version WHERE rowid = 1")
        .fetch_one(&pool)
        .await?;

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
        "UPDATE images SET file_hash = ?, file_size = ?, mtime = ?,
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

#[derive(Clone)]
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

pub async fn update_preview_path(pool: &SqlitePool, image_id: i64, preview_path: &str) -> Result<()> {
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
            "SELECT id, folder_id, path, file_hash, file_size, date_taken, mtime, thumbnail_path, preview_path,
                    semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
             FROM images WHERE folder_id = ? AND deleted_at IS NULL
             ORDER BY COALESCE(date_taken, mtime) DESC",
        )
        .bind(fid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, folder_id, path, file_hash, file_size, date_taken, mtime, thumbnail_path, preview_path,
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
        "SELECT id, folder_id, path, file_hash, file_size, date_taken, mtime, thumbnail_path, preview_path,
                semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
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

pub async fn get_queue_batch(pool: &SqlitePool, pipeline: &str, limit: i64) -> Result<Vec<(i64, i64, i32)>> {
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
        .map(|r| (r.get::<i64, _>("id"), r.get::<i64, _>("image_id"), r.get::<i32, _>("attempts")))
        .collect())
}

pub async fn mark_semantic_analysis_done(pool: &SqlitePool, queue_id: i64, image_id: i64, embedding: &[u8]) -> Result<()> {
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

pub async fn mark_subject_analysis_done(pool: &SqlitePool, queue_id: i64, image_id: i64) -> Result<()> {
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

pub async fn insert_subject(pool: &SqlitePool, name: Option<&str>, subject_type: &str) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query("INSERT INTO subjects (name, type, added_at) VALUES (?, ?, ?)")
        .bind(name)
        .bind(subject_type)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(result.last_insert_rowid())
}

pub async fn insert_face(
    pool: &SqlitePool,
    image_id: i64,
    subject_id: Option<i64>,
    bbox: (f64, f64, f64, f64),
    embedding: Option<&[u8]>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(image_id)
    .bind(subject_id)
    .bind(bbox.0)
    .bind(bbox.1)
    .bind(bbox.2)
    .bind(bbox.3)
    .bind(embedding)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn list_all_subjects(pool: &SqlitePool) -> Result<Vec<Subject>> {
    let rows = sqlx::query("SELECT id, name, thumbnail_face_id, type, added_at FROM subjects ORDER BY CASE WHEN name IS NOT NULL THEN 0 ELSE 1 END, added_at DESC")
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| Subject {
            id: r.get("id"),
            name: r.get("name"),
            thumbnail_face_id: r.get("thumbnail_face_id"),
            subject_type: r.get("type"),
            added_at: r.get("added_at"),
        })
        .collect())
}

pub async fn list_faces_for_subject(pool: &SqlitePool, subject_id: i64) -> Result<Vec<Face>> {
    let rows = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual
         FROM faces WHERE subject_id = ? ORDER BY added_at DESC",
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Face {
            id: r.get("id"),
            image_id: r.get("image_id"),
            subject_id: r.get("subject_id"),
            bbox_x: r.get("bbox_x"),
            bbox_y: r.get("bbox_y"),
            bbox_w: r.get("bbox_w"),
            bbox_h: r.get("bbox_h"),
            embedding: r.get("embedding"),
            added_at: r.get("added_at"),
            is_manual: r.get::<i32, _>("is_manual") != 0,
        })
        .collect())
}

pub async fn get_subject_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT subject_id, embedding FROM faces
         WHERE subject_id IS NOT NULL AND embedding IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get("subject_id"), r.get("embedding")))
        .collect())
}

pub async fn get_manual_face_embeddings_by_subject(
    pool: &SqlitePool,
) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT subject_id, embedding FROM faces \
         WHERE subject_id IS NOT NULL AND embedding IS NOT NULL AND is_manual = 1",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_face_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Face>> {
    let row = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual
         FROM faces WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    
    Ok(row.as_ref().map(|r| Face {
        id: r.get("id"),
        image_id: r.get("image_id"),
        subject_id: r.get("subject_id"),
        bbox_x: r.get("bbox_x"),
        bbox_y: r.get("bbox_y"),
        bbox_w: r.get("bbox_w"),
        bbox_h: r.get("bbox_h"),
        embedding: r.get("embedding"),
        added_at: r.get("added_at"),
        is_manual: r.get::<i32, _>("is_manual") != 0,
    }))
}

pub async fn update_subject_name(pool: &SqlitePool, id: i64, name: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE subjects SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_subject_thumbnail_face(pool: &SqlitePool, subject_id: i64, face_id: i64) -> Result<()> {
    // Validate face belongs to subject
    let face = sqlx::query("SELECT id FROM faces WHERE id = ? AND subject_id = ?")
        .bind(face_id)
        .bind(subject_id)
        .fetch_optional(pool)
        .await?;

    if face.is_none() {
        return Err(anyhow::anyhow!("Face does not belong to subject"));
    }

    sqlx::query("UPDATE subjects SET thumbnail_face_id = ? WHERE id = ?")
        .bind(face_id)
        .bind(subject_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_subject_detail_with_counts(pool: &SqlitePool, id: i64) -> Result<Option<crate::models::SubjectDetail>> {
    let row = sqlx::query(
        r#"SELECT s.id, s.name, s.thumbnail_face_id, s.type, s.added_at,
                  (SELECT COUNT(DISTINCT image_id) FROM faces WHERE subject_id = s.id) as photo_count,
                  (SELECT COUNT(*) FROM faces WHERE subject_id = s.id) as face_count
           FROM subjects s
           WHERE s.id = ?"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| crate::models::SubjectDetail {
        subject: Subject {
            id: r.get("id"),
            name: r.get("name"),
            thumbnail_face_id: r.get("thumbnail_face_id"),
            subject_type: r.get("type"),
            added_at: r.get("added_at"),
        },
        photo_count: r.get("photo_count"),
        face_count: r.get("face_count"),
    }))
}

pub async fn list_images_for_subject(pool: &SqlitePool, subject_id: i64) -> Result<Vec<Image>> {
    let rows = sqlx::query(
        r#"SELECT DISTINCT i.id, i.folder_id, i.path, i.file_hash, i.file_size, i.date_taken, i.mtime, i.thumbnail_path, i.preview_path,
                           i.semantic_analysis_done, i.subject_analysis_done, i.added_at, i.updated_at, i.deleted_at
           FROM images i
           JOIN faces f ON f.image_id = i.id
           WHERE f.subject_id = ? AND i.deleted_at IS NULL
           ORDER BY COALESCE(i.date_taken, i.mtime) DESC"#,
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(row_to_image).collect())
}

pub async fn get_largest_face_for_subject(pool: &SqlitePool, subject_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT id FROM faces WHERE subject_id = ? ORDER BY (bbox_w * bbox_h) DESC LIMIT 1"
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;
    
    Ok(row.map(|r| r.get("id")))
}

pub async fn list_faces_for_image(pool: &SqlitePool, image_id: i64) -> Result<Vec<Face>> {
    let rows = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual
         FROM faces WHERE image_id = ? ORDER BY added_at DESC",
    )
    .bind(image_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Face {
            id: r.get("id"),
            image_id: r.get("image_id"),
            subject_id: r.get("subject_id"),
            bbox_x: r.get("bbox_x"),
            bbox_y: r.get("bbox_y"),
            bbox_w: r.get("bbox_w"),
            bbox_h: r.get("bbox_h"),
            embedding: r.get("embedding"),
            added_at: r.get("added_at"),
            is_manual: r.get::<i32, _>("is_manual") != 0,
        })
        .collect())
}

pub async fn search_subjects_by_name(pool: &SqlitePool, query: &str) -> Result<Vec<Subject>> {
    let like_query = format!("%{}%", query);
    let rows = sqlx::query(
        "SELECT id, name, thumbnail_face_id, type, added_at 
         FROM subjects 
         WHERE name LIKE ? COLLATE NOCASE 
         ORDER BY added_at DESC"
    )
    .bind(like_query)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Subject {
            id: r.get("id"),
            name: r.get("name"),
            thumbnail_face_id: r.get("thumbnail_face_id"),
            subject_type: r.get("type"),
            added_at: r.get("added_at"),
        })
        .collect())
}

pub async fn get_image_ids_for_subjects(pool: &SqlitePool, subject_ids: &[i64]) -> Result<Vec<i64>> {
    if subject_ids.is_empty() {
        return Ok(vec![]);
    }
    let params = format!("?{}", ", ?".repeat(subject_ids.len() - 1));
    let query_str = format!(
        "SELECT DISTINCT image_id FROM faces WHERE subject_id IN ({})",
        params
    );
    let mut query = sqlx::query(&query_str);
    for id in subject_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| r.get("image_id")).collect())
}

pub async fn get_unassigned_faces_with_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT id, embedding FROM faces WHERE subject_id IS NULL AND embedding IS NOT NULL",
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

pub async fn update_face_subject(pool: &SqlitePool, face_id: i64, subject_id: Option<i64>) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = ? WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_subjects_with_no_faces(pool: &SqlitePool) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM subjects WHERE id NOT IN (SELECT DISTINCT subject_id FROM faces WHERE subject_id IS NOT NULL)",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn auto_assign_missing_thumbnails(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT s.id FROM subjects s WHERE s.thumbnail_face_id IS NULL",
    )
    .fetch_all(pool)
    .await?;

    for row in &rows {
        let subject_id: i64 = row.get("id");
        if let Ok(Some(face_id)) = get_largest_face_for_subject(pool, subject_id).await {
            let _ = update_subject_thumbnail_face(pool, subject_id, face_id).await;
        }
    }
    Ok(())
}

pub async fn get_cached_embedding(pool: &SqlitePool, cache_key: &str, query_type: &str) -> Result<Option<Vec<u8>>> {
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

pub async fn insert_cached_embedding(pool: &SqlitePool, cache_key: &str, query_type: &str, embedding: &[u8]) -> Result<()> {
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

pub async fn clear_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_merge_suggestion(
    pool: &SqlitePool,
    subject_id_a: i64,
    subject_id_b: i64,
    score: f64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let (lo, hi) = if subject_id_a < subject_id_b {
        (subject_id_a, subject_id_b)
    } else {
        (subject_id_b, subject_id_a)
    };
    sqlx::query(
        "INSERT OR IGNORE INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(lo)
    .bind(hi)
    .bind(score)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_merge_suggestions(pool: &SqlitePool, limit: Option<i64>) -> Result<Vec<crate::models::MergeSuggestion>> {
    let rows = match limit {
        Some(n) if n > 0 => {
            sqlx::query(
                r#"SELECT ms.id, ms.score,
                          sa.id as sa_id, sa.name as sa_name, sa.thumbnail_face_id as sa_thumbnail_face_id, sa.type as sa_type, sa.added_at as sa_added_at,
                          sb.id as sb_id, sb.name as sb_name, sb.thumbnail_face_id as sb_thumbnail_face_id, sb.type as sb_type, sb.added_at as sb_added_at
                   FROM merge_suggestions ms
                   JOIN subjects sa ON ms.subject_id_a = sa.id
                   JOIN subjects sb ON ms.subject_id_b = sb.id
                   ORDER BY ms.score DESC, ms.id ASC
                   LIMIT ?"#
            )
            .bind(n)
            .fetch_all(pool)
            .await?
        }
        Some(_) => return Ok(vec![]),
        None => {
            sqlx::query(
                r#"SELECT ms.id, ms.score,
                          sa.id as sa_id, sa.name as sa_name, sa.thumbnail_face_id as sa_thumbnail_face_id, sa.type as sa_type, sa.added_at as sa_added_at,
                          sb.id as sb_id, sb.name as sb_name, sb.thumbnail_face_id as sb_thumbnail_face_id, sb.type as sb_type, sb.added_at as sb_added_at
                   FROM merge_suggestions ms
                   JOIN subjects sa ON ms.subject_id_a = sa.id
                   JOIN subjects sb ON ms.subject_id_b = sb.id
                   ORDER BY ms.score DESC, ms.id ASC"#
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|r| crate::models::MergeSuggestion {
            id: r.get("id"),
            subject_a: crate::models::Subject {
                id: r.get("sa_id"),
                name: r.get("sa_name"),
                thumbnail_face_id: r.get("sa_thumbnail_face_id"),
                subject_type: r.get("sa_type"),
                added_at: r.get("sa_added_at"),
            },
            subject_b: crate::models::Subject {
                id: r.get("sb_id"),
                name: r.get("sb_name"),
                thumbnail_face_id: r.get("sb_thumbnail_face_id"),
                subject_type: r.get("sb_type"),
                added_at: r.get("sb_added_at"),
            },
            score: r.get("score"),
        })
        .collect())
}

pub async fn get_subject_named_flags(pool: &SqlitePool) -> Result<std::collections::HashMap<i64, bool>> {
    let rows = sqlx::query("SELECT id, (name IS NOT NULL) as has_name FROM subjects")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<bool, _>("has_name")))
        .collect())
}

pub async fn merge_subjects(pool: &SqlitePool, target_id: i64, source_id: i64) -> Result<()> {
    if target_id == source_id {
        return Ok(());
    }

    // Determine which subject has a name; if only one is named, ensure its name survives.
    let rows = sqlx::query("SELECT id, name FROM subjects WHERE id = ? OR id = ?")
        .bind(target_id)
        .bind(source_id)
        .fetch_all(pool)
        .await?;

    let mut target_name: Option<String> = None;
    let mut source_name: Option<String> = None;
    for row in rows {
        let id: i64 = row.get("id");
        let name: Option<String> = row.get("name");
        if id == target_id {
            target_name = name;
        } else if id == source_id {
            source_name = name;
        }
    }

    // Rule: named subject's name always survives.
    // If target is unnamed and source is named, copy the source name to target.
    if target_name.is_none() && source_name.is_some() {
        sqlx::query("UPDATE subjects SET name = ? WHERE id = ? AND name IS NULL")
            .bind(&source_name)
            .bind(target_id)
            .execute(pool)
            .await?;
    }

    sqlx::query("UPDATE faces SET subject_id = ?, is_manual = 1 WHERE subject_id = ?")
        .bind(target_id)
        .bind(source_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM merge_suggestions WHERE subject_id_a = ? OR subject_id_b = ? OR subject_id_a = ? OR subject_id_b = ?")
        .bind(target_id)
        .bind(target_id)
        .bind(source_id)
        .bind(source_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM subjects WHERE id = ?")
        .bind(source_id)
        .execute(pool)
        .await?;

    let _ = auto_assign_missing_thumbnails(pool).await;
    Ok(())
}

pub async fn get_dismissed_pair_set(pool: &SqlitePool) -> Result<std::collections::HashSet<(i64, i64)>> {
    let rows = sqlx::query("SELECT subject_id_a, subject_id_b FROM dismissed_pairs")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let a = r.get::<i64, _>("subject_id_a");
            let b = r.get::<i64, _>("subject_id_b");
            if a < b { (a, b) } else { (b, a) }
        })
        .collect())
}

pub async fn dismiss_merge_suggestion(pool: &SqlitePool, id: i64) -> Result<()> {
    let row = sqlx::query("SELECT subject_id_a, subject_id_b FROM merge_suggestions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    if let Some(r) = row {
        let sid_a: i64 = r.get("subject_id_a");
        let sid_b: i64 = r.get("subject_id_b");
        let (lo, hi) = if sid_a < sid_b { (sid_a, sid_b) } else { (sid_b, sid_a) };
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR IGNORE INTO dismissed_pairs (subject_id_a, subject_id_b, dismissed_at) VALUES (?, ?, ?)"
        )
        .bind(lo)
        .bind(hi)
        .bind(now)
        .execute(pool)
        .await?;
    }

    sqlx::query("DELETE FROM merge_suggestions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_subject_by_name(pool: &SqlitePool, name: &str, exclude_id: i64) -> Result<Option<Subject>> {
    let row = sqlx::query(
        "SELECT id, name, thumbnail_face_id, type, added_at FROM subjects WHERE name = ? COLLATE NOCASE AND id != ? LIMIT 1",
    )
    .bind(name)
    .bind(exclude_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Subject {
        id: r.get("id"),
        name: r.get("name"),
        thumbnail_face_id: r.get("thumbnail_face_id"),
        subject_type: r.get("type"),
        added_at: r.get("added_at"),
    }))
}

pub async fn assign_face_to_subject(pool: &SqlitePool, face_id: i64, subject_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = ?, is_manual = 1 WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_subject_for_face(pool: &SqlitePool, face_id: i64, name: Option<&str>) -> Result<Subject> {
    let subject_id = insert_subject(pool, name, "person").await?;
    sqlx::query("UPDATE faces SET subject_id = ?, is_manual = 1 WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    let row = sqlx::query(
        "SELECT id, name, thumbnail_face_id, type, added_at FROM subjects WHERE id = ?"
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await?;
    Ok(Subject {
        id: row.get("id"),
        name: row.get("name"),
        thumbnail_face_id: row.get("thumbnail_face_id"),
        subject_type: row.get("type"),
        added_at: row.get("added_at"),
    })
}

pub async fn get_face_subject_id(pool: &SqlitePool, face_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT subject_id FROM faces WHERE id = ?")
        .bind(face_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.get::<Option<i64>, _>("subject_id")))
}

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("value")))
}

pub async fn record_face_correction(pool: &SqlitePool, face_id: i64, old_subject_id: Option<i64>, new_subject_id: Option<i64>) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO face_corrections (face_id, old_subject_id, new_subject_id, created_at) VALUES (?, ?, ?, ?)"
    )
    .bind(face_id)
    .bind(old_subject_id)
    .bind(new_subject_id)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_face_cannot_link_subjects(
    pool: &SqlitePool,
) -> Result<std::collections::HashMap<i64, std::collections::HashSet<i64>>> {
    let rows = sqlx::query(
        "SELECT face_id, old_subject_id FROM face_corrections \
         WHERE new_subject_id IS NULL AND old_subject_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    let mut map: std::collections::HashMap<i64, std::collections::HashSet<i64>> =
        std::collections::HashMap::new();
    for row in rows {
        let face_id: i64 = row.get("face_id");
        let forbidden: i64 = row.get("old_subject_id");
        map.entry(face_id).or_default().insert(forbidden);
    }
    Ok(map)
}

pub async fn unassign_face(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = NULL, is_manual = 1 WHERE id = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn reset_all_embeddings(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    // Clear image embeddings and reset status
    sqlx::query("UPDATE images SET embedding = NULL, semantic_analysis_done = 0, subject_analysis_done = 0 WHERE deleted_at IS NULL")
        .execute(&mut *tx)
        .await?;

    // Clear face embeddings (face detections remain, but need re-embedding)
    sqlx::query("UPDATE faces SET embedding = NULL")
        .execute(&mut *tx)
        .await?;

    // Clear model-dependent caches and suggestions
    sqlx::query("DELETE FROM embedding_cache").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM merge_suggestions").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM embedding_queue").execute(&mut *tx).await?;

    let now = chrono::Utc::now().timestamp();
    // Re-populate queue for both pipelines
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

pub async fn reset_all_subject_data(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM face_corrections")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM faces")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM subjects")
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE images SET subject_analysis_done = 0 WHERE deleted_at IS NULL")
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM embedding_queue WHERE pipeline = 'subject'")
        .execute(&mut *tx)
        .await?;

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at)
         SELECT id, 'subject', 0, ? FROM images WHERE deleted_at IS NULL",
    )
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn make_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE subjects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT,
                thumbnail_face_id INTEGER,
                type TEXT NOT NULL DEFAULT 'person',
                added_at INTEGER NOT NULL
            )"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE faces (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                image_id INTEGER NOT NULL,
                subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
                bbox_x REAL NOT NULL, bbox_y REAL NOT NULL,
                bbox_w REAL NOT NULL, bbox_h REAL NOT NULL,
                embedding BLOB,
                added_at INTEGER NOT NULL,
                is_manual INTEGER NOT NULL DEFAULT 0
            )"
        ).execute(&pool).await.unwrap();
        pool
    }

    async fn make_merge_pool() -> SqlitePool {
        let pool = make_pool().await;
        sqlx::query(
            "CREATE TABLE merge_suggestions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                score REAL NOT NULL,
                created_at INTEGER NOT NULL
            )"
        ).execute(&pool).await.unwrap();
        pool
    }

    async fn insert_subject(pool: &SqlitePool, name: Option<&str>) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES (?, 'person', 0) RETURNING id"
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn get_merge_suggestions_with_limit_returns_top_n_by_score() {
        let pool = make_merge_pool().await;

        let a: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let b: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let c: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Carol', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let d: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Dave', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let e: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Eve', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        for (sa, sb, score) in [
            (a, b, 0.95f64),
            (b, c, 0.90),
            (c, d, 0.80),
            (d, e, 0.70),
            (a, e, 0.60),
        ] {
            sqlx::query(
                "INSERT INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at) VALUES (?, ?, ?, 0)"
            ).bind(sa).bind(sb).bind(score).execute(&pool).await.unwrap();
        }

        let top3 = get_merge_suggestions(&pool, Some(3)).await.unwrap();
        assert_eq!(top3.len(), 3);
        assert!((top3[0].score - 0.95).abs() < 1e-9, "first should be highest score");
        assert!((top3[1].score - 0.90).abs() < 1e-9);
        assert!((top3[2].score - 0.80).abs() < 1e-9);

        let all = get_merge_suggestions(&pool, None).await.unwrap();
        assert_eq!(all.len(), 5, "no limit should return all 5");
        assert!((all[0].score - 0.95).abs() < 1e-9, "first should still be highest score");
    }

    /// A minimal in-memory pool containing only the folders + images tables,
    /// sufficient for testing image-level DB helpers without requiring a temp file.
    async fn make_images_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE folders (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                path     TEXT UNIQUE NOT NULL,
                added_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE images (
                id                     INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_id              INTEGER NOT NULL REFERENCES folders(id),
                path                   TEXT UNIQUE NOT NULL,
                file_hash              TEXT NOT NULL,
                file_size              INTEGER NOT NULL DEFAULT 0,
                date_taken             INTEGER,
                mtime                  INTEGER NOT NULL,
                thumbnail_path         TEXT,
                preview_path           TEXT,
                semantic_analysis_done INTEGER NOT NULL DEFAULT 0,
                subject_analysis_done  INTEGER NOT NULL DEFAULT 0,
                embedding              BLOB,
                added_at               INTEGER NOT NULL,
                updated_at             INTEGER NOT NULL,
                deleted_at             INTEGER
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// update_thumbnail_path must persist the path and make it visible via
    /// get_image_by_id — this is the DB half of the early-preview contract.
    #[tokio::test]
    async fn update_thumbnail_path_persists_and_is_readable() {
        let pool = make_images_pool().await;

        // Insert a folder so the FK constraint is satisfied
        let folder_id: i64 = sqlx::query_scalar(
            "INSERT INTO folders (path, added_at) VALUES ('/test/photos', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // Insert an image row; thumbnail_path starts NULL
        let image_id = insert_image(&pool, folder_id, "/test/photos/img.jpg", "abc123", 1024, 0)
            .await
            .unwrap();

        let before = get_image_by_id(&pool, image_id).await.unwrap().unwrap();
        assert!(
            before.thumbnail_path.is_none(),
            "thumbnail_path should be NULL before update"
        );

        // Record the thumbnail path (simulates the pipeline's Stage-1 early-preview step)
        let expected_path = format!("/data/thumbnails/{}.webp", image_id);
        update_thumbnail_path(&pool, image_id, &expected_path)
            .await
            .unwrap();

        let after = get_image_by_id(&pool, image_id).await.unwrap().unwrap();
        assert_eq!(
            after.thumbnail_path.as_deref(),
            Some(expected_path.as_str()),
            "thumbnail_path should equal the value written by update_thumbnail_path"
        );
    }

    #[tokio::test]
    async fn update_preview_path_persists_and_is_readable() {
        let dir = std::env::temp_dir().join(format!("nebula_prevdb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = init_db(&dir).await.unwrap();
        let folder_id = insert_folder(&pool, "/tmp/f").await.unwrap();
        let image_id = insert_image(&pool, folder_id, "/tmp/f/a.jpg", "h", 1, 1).await.unwrap();

        update_preview_path(&pool, image_id, "/tmp/p_7.webp").await.unwrap();

        let img = get_image_by_id(&pool, image_id).await.unwrap().unwrap();
        assert_eq!(img.preview_path.as_deref(), Some("/tmp/p_7.webp"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn images_needing_preview_excludes_thumbnailed_and_deleted() {
        let dir = std::env::temp_dir().join(format!("nebula_needprev_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = init_db(&dir).await.unwrap();
        let fid = insert_folder(&pool, "/tmp/f").await.unwrap();
        let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "h1", 1, 1).await.unwrap();
        let b = insert_image(&pool, fid, "/tmp/f/b.jpg", "h2", 1, 1).await.unwrap();
        // a already has a thumbnail -> excluded
        update_thumbnail_path(&pool, a, "/tmp/a.webp").await.unwrap();

        let need = images_needing_preview(&pool).await.unwrap();
        assert!(need.contains(&b));
        assert!(!need.contains(&a));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn get_manual_face_embeddings_returns_only_manual() {
        let pool = make_pool().await;
        let subject_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let manual_emb: Vec<u8> = vec![1u8; 8];
        let auto_emb: Vec<u8> = vec![2u8; 8];

        sqlx::query(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual) VALUES (1, ?, 0,0,1,1, ?, 0, 1)"
        ).bind(subject_id).bind(&manual_emb).execute(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual) VALUES (1, ?, 0,0,1,1, ?, 0, 0)"
        ).bind(subject_id).bind(&auto_emb).execute(&pool).await.unwrap();

        let results = get_manual_face_embeddings_by_subject(&pool).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, subject_id);
        assert_eq!(results[0].1, manual_emb);
    }

    async fn make_dismissal_pool() -> SqlitePool {
        let pool = make_merge_pool().await;
        sqlx::query(
            "CREATE TABLE dismissed_pairs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
                dismissed_at INTEGER NOT NULL
            )"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE UNIQUE INDEX idx_dismissed_pair ON dismissed_pairs(subject_id_a, subject_id_b)"
        ).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn dismiss_persists_pair_in_dismissed_pairs() {
        let pool = make_dismissal_pool().await;

        let a: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let b: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let suggestion_id: i64 = sqlx::query_scalar(
            "INSERT INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at) VALUES (?, ?, 0.9, 0) RETURNING id"
        ).bind(a).bind(b).fetch_one(&pool).await.unwrap();

        dismiss_merge_suggestion(&pool, suggestion_id).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0, "suggestion should be deleted");

        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let dismissed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dismissed_pairs WHERE subject_id_a = ? AND subject_id_b = ?"
        ).bind(lo).bind(hi).fetch_one(&pool).await.unwrap();
        assert_eq!(dismissed, 1, "dismissed pair should be persisted");
    }

    #[tokio::test]
    async fn get_dismissed_pair_set_returns_stored_pairs() {
        // Pool needs: subjects, faces, dismissed_pairs, merge_suggestions tables
        let pool = make_dismissal_pool().await;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS faces (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                image_id INTEGER NOT NULL DEFAULT 0,
                subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
                bbox_x REAL NOT NULL DEFAULT 0,
                bbox_y REAL NOT NULL DEFAULT 0,
                bbox_w REAL NOT NULL DEFAULT 0.5,
                bbox_h REAL NOT NULL DEFAULT 0.5,
                embedding BLOB,
                added_at INTEGER NOT NULL DEFAULT 0,
                is_manual INTEGER NOT NULL DEFAULT 0
            )"
        ).execute(&pool).await.unwrap();

        let a: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let b: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        // Insert a dismissed pair for (a, b)
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        sqlx::query(
            "INSERT INTO dismissed_pairs (subject_id_a, subject_id_b, dismissed_at) VALUES (?, ?, 0)"
        ).bind(lo).bind(hi).execute(&pool).await.unwrap();

        // Insert a suggestion for the same pair (simulating what clustering would insert)
        sqlx::query(
            "INSERT INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at) VALUES (?, ?, 0.95, 0)"
        ).bind(lo).bind(hi).execute(&pool).await.unwrap();

        // Verify the helper returns the right set
        let dismissed = get_dismissed_pair_set(&pool).await.unwrap();
        assert!(dismissed.contains(&(lo, hi)), "dismissed set should include the pair");

        // Verify insert_merge_suggestion is a no-op for dismissed pairs
        // (clustering will call this after checking the set, so test the set check logic)
        let is_dismissed = dismissed.contains(&(lo, hi));
        assert!(is_dismissed, "pair should be flagged as dismissed so clustering skips it");
    }

    #[tokio::test]
    async fn get_subject_named_flags_returns_true_for_named_and_false_for_unnamed() {
        let pool = make_pool().await;
        let named_id: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let unnamed_id: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let flags = get_subject_named_flags(&pool).await.unwrap();

        assert_eq!(flags.get(&named_id), Some(&true));
        assert_eq!(flags.get(&unnamed_id), Some(&false));
    }

    #[tokio::test]
    async fn merge_unnamed_into_named_preserves_name() {
        let pool = make_merge_pool().await;

        let named_id = insert_subject(&pool, Some("Casandra")).await;
        let unnamed_id = insert_subject(&pool, None).await;

        merge_subjects(&pool, named_id, unnamed_id).await.unwrap();

        let surviving_name: Option<String> = sqlx::query_scalar(
            "SELECT name FROM subjects WHERE id = ?"
        )
        .bind(named_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(surviving_name, Some("Casandra".to_string()));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subjects")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn merge_named_into_unnamed_preserves_name() {
        let pool = make_merge_pool().await;

        let unnamed_id = insert_subject(&pool, None).await;
        let named_id = insert_subject(&pool, Some("Casandra")).await;

        merge_subjects(&pool, unnamed_id, named_id).await.unwrap();

        let surviving_name: Option<String> = sqlx::query_scalar(
            "SELECT name FROM subjects WHERE id = ?"
        )
        .bind(unnamed_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(surviving_name, Some("Casandra".to_string()));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subjects")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn merge_named_into_named_preserves_target_name() {
        let pool = make_merge_pool().await;

        let target_id = insert_subject(&pool, Some("Cas")).await;
        let source_id = insert_subject(&pool, Some("Ana")).await;

        merge_subjects(&pool, target_id, source_id).await.unwrap();

        let surviving_name: Option<String> = sqlx::query_scalar(
            "SELECT name FROM subjects WHERE id = ?"
        )
        .bind(target_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(surviving_name, Some("Cas".to_string()));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subjects")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn merge_unnamed_into_unnamed_stays_unnamed() {
        let pool = make_merge_pool().await;

        let target_id = insert_subject(&pool, None).await;
        let source_id = insert_subject(&pool, None).await;

        merge_subjects(&pool, target_id, source_id).await.unwrap();

        let surviving_name: Option<String> = sqlx::query_scalar(
            "SELECT name FROM subjects WHERE id = ?"
        )
        .bind(target_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(surviving_name, None);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subjects")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn get_face_cannot_link_subjects_returns_removal_rows_only() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE subjects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT,
                thumbnail_face_id INTEGER,
                type TEXT NOT NULL DEFAULT 'person',
                added_at INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE faces (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                image_id INTEGER NOT NULL DEFAULT 0,
                subject_id INTEGER,
                bbox_x REAL NOT NULL DEFAULT 0,
                bbox_y REAL NOT NULL DEFAULT 0,
                bbox_w REAL NOT NULL DEFAULT 0.5,
                bbox_h REAL NOT NULL DEFAULT 0.5,
                embedding BLOB,
                added_at INTEGER NOT NULL DEFAULT 0,
                is_manual INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE face_corrections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                face_id INTEGER NOT NULL,
                old_subject_id INTEGER,
                new_subject_id INTEGER,
                created_at INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let s1: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (type, added_at) VALUES ('person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let s2: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (type, added_at) VALUES ('person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let f1: i64 = sqlx::query_scalar("INSERT INTO faces (added_at) VALUES (0) RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
        let f2: i64 = sqlx::query_scalar("INSERT INTO faces (added_at) VALUES (0) RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
        let f3: i64 = sqlx::query_scalar("INSERT INTO faces (added_at) VALUES (0) RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();

        // f1 removed from s1 (new_subject_id IS NULL) → forbidden {f1: {s1}}
        sqlx::query(
            "INSERT INTO face_corrections (face_id, old_subject_id, new_subject_id, created_at) VALUES (?, ?, NULL, 0)",
        )
        .bind(f1)
        .bind(s1)
        .execute(&pool)
        .await
        .unwrap();

        // f1 also removed from s2 → forbidden {f1: {s1, s2}}
        sqlx::query(
            "INSERT INTO face_corrections (face_id, old_subject_id, new_subject_id, created_at) VALUES (?, ?, NULL, 0)",
        )
        .bind(f1)
        .bind(s2)
        .execute(&pool)
        .await
        .unwrap();

        // f2 moved from s1 to s2 (new_subject_id NOT NULL) → must NOT appear
        sqlx::query(
            "INSERT INTO face_corrections (face_id, old_subject_id, new_subject_id, created_at) VALUES (?, ?, ?, 0)",
        )
        .bind(f2)
        .bind(s1)
        .bind(s2)
        .execute(&pool)
        .await
        .unwrap();

        // f3 has no corrections at all → must NOT appear
        let _ = f3;

        let result = get_face_cannot_link_subjects(&pool).await.unwrap();

        let f1_forbidden = result.get(&f1).expect("f1 must have forbidden set");
        assert!(f1_forbidden.contains(&s1), "f1 must be forbidden from s1");
        assert!(f1_forbidden.contains(&s2), "f1 must be forbidden from s2");
        assert_eq!(f1_forbidden.len(), 2);

        assert!(
            !result.contains_key(&f2),
            "f2 was moved (not removed) — must not appear"
        );
        assert!(
            !result.contains_key(&f3),
            "f3 has no corrections — must not appear"
        );
    }

    #[tokio::test]
    async fn merge_marks_source_faces_as_manual() {
        let pool = make_merge_pool().await;

        let target = insert_subject(&pool, Some("Alice")).await;
        let source = insert_subject(&pool, Some("Bob")).await;

        // Two faces for target (not manual yet)
        sqlx::query(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) \
             VALUES (1, ?, 0, 0, 0.5, 0.5, 0), (2, ?, 0, 0, 0.5, 0.5, 0)",
        )
        .bind(target)
        .bind(target)
        .execute(&pool)
        .await
        .unwrap();

        // Two faces for source (not manual yet)
        let src_face1: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) \
             VALUES (3, ?, 0, 0, 0.5, 0.5, 0) RETURNING id",
        )
        .bind(source)
        .fetch_one(&pool)
        .await
        .unwrap();

        let src_face2: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) \
             VALUES (4, ?, 0, 0, 0.5, 0.5, 0) RETURNING id",
        )
        .bind(source)
        .fetch_one(&pool)
        .await
        .unwrap();

        merge_subjects(&pool, target, source).await.unwrap();

        // Source faces must now belong to target
        let f1_subject: Option<i64> =
            sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
                .bind(src_face1)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(f1_subject, Some(target), "src_face1 must move to target");

        let f2_subject: Option<i64> =
            sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
                .bind(src_face2)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(f2_subject, Some(target), "src_face2 must move to target");

        // Source faces must be marked is_manual = 1
        let f1_manual: i32 = sqlx::query_scalar("SELECT is_manual FROM faces WHERE id = ?")
            .bind(src_face1)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(f1_manual, 1, "src_face1 must be marked is_manual after merge");

        let f2_manual: i32 = sqlx::query_scalar("SELECT is_manual FROM faces WHERE id = ?")
            .bind(src_face2)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(f2_manual, 1, "src_face2 must be marked is_manual after merge");
    }
}
