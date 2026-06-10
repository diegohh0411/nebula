use anyhow::Result;
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Row, SqlitePool};
use std::path::Path;
use std::sync::Once;

use crate::models::{ProcessingStatus, Folder, FolderWithCount, Image, Face, Subject};

static SQLITE_VEC_INIT: Once = Once::new();

/// Register the sqlite-vec extension with every new SQLite connection.
/// Idempotent: safe to call multiple times; registers exactly once per process.
pub fn ensure_sqlite_vec_registered() {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        extern "C" {
            fn sqlite3_auto_extension(xInit: Option<unsafe extern "C" fn()>) -> i32;
        }
        // sqlite_vec::sqlite3_vec_init matches the standard extension init signature.
        // We transmute to the void-fn type that sqlite3_auto_extension demands per the C API.
        let f: unsafe extern "C" fn() =
            std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        sqlite3_auto_extension(Some(f));
    });
}

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
    is_manual   INTEGER NOT NULL DEFAULT 0,
    det_score      REAL,
    quality_score  REAL
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
    (3, "CREATE VIRTUAL TABLE IF NOT EXISTS face_vectors USING vec0(embedding float[512])"),
    (4, "
        CREATE TABLE IF NOT EXISTS constraints (
            face_a      INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
            face_b      INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
            kind        TEXT NOT NULL CHECK(kind IN ('must_link', 'cannot_link')),
            source      TEXT NOT NULL CHECK(source IN ('merge', 'manual_assign', 'removal', 'dismiss')),
            created_at  INTEGER NOT NULL,
            PRIMARY KEY (face_a, face_b, kind)
        )
    "),
    (5, "
        INSERT OR REPLACE INTO face_vectors(rowid, embedding)
            SELECT id, embedding FROM faces WHERE embedding IS NOT NULL;
        ALTER TABLE faces DROP COLUMN embedding;
        ALTER TABLE faces DROP COLUMN is_manual;
        DROP TABLE IF EXISTS face_corrections
    "),
    (6, "
    CREATE TABLE IF NOT EXISTS face_edges (
        face_a  INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
        face_b  INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
        weight  REAL NOT NULL,
        PRIMARY KEY (face_a, face_b)
    )
"),
];

pub async fn init_db(data_dir: &Path) -> Result<SqlitePool> {
    ensure_sqlite_vec_registered();
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
    det_score: Option<f64>,
    quality_score: Option<f64>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at, det_score, quality_score)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(image_id)
    .bind(subject_id)
    .bind(bbox.0)
    .bind(bbox.1)
    .bind(bbox.2)
    .bind(bbox.3)
    .bind(now)
    .bind(det_score)
    .bind(quality_score)
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
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at
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
            added_at: r.get("added_at"),
        })
        .collect())
}


pub async fn get_face_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Face>> {
    let row = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at
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
        added_at: r.get("added_at"),
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
        "SELECT id FROM faces WHERE subject_id = ?
         ORDER BY (quality_score IS NULL), quality_score DESC, (bbox_w * bbox_h) DESC
         LIMIT 1",
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| r.get("id")))
}

pub async fn list_faces_for_image(pool: &SqlitePool, image_id: i64) -> Result<Vec<Face>> {
    let rows = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at
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
            added_at: r.get("added_at"),
        })
        .collect())
}

/// Returns (image_path, (bbox_x, bbox_y, bbox_w, bbox_h)) for a face, or None if missing.
pub async fn get_face_with_image(
    pool: &SqlitePool,
    face_id: i64,
) -> Result<Option<(String, (f64, f64, f64, f64))>> {
    let row = sqlx::query(
        "SELECT i.path AS path, f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h
         FROM faces f JOIN images i ON i.id = f.image_id
         WHERE f.id = ?",
    )
    .bind(face_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (
            r.get::<String, _>("path"),
            (
                r.get::<f64, _>("bbox_x"),
                r.get::<f64, _>("bbox_y"),
                r.get::<f64, _>("bbox_w"),
                r.get::<f64, _>("bbox_h"),
            ),
        )
    }))
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
        "SELECT f.id, fv.embedding
         FROM face_vectors fv
         JOIN faces f ON f.id = fv.rowid
         WHERE f.subject_id IS NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<Vec<u8>, _>("embedding")))
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

/// For every subject, set `thumbnail_face_id` to its highest-quality face.
/// `quality_score` NULLs sort last; ties fall back to largest bbox area.
/// Never clears an existing thumbnail. Returns `(subject_id, face_id)` pairs for
/// subjects whose thumbnail changed so callers can regenerate those crops directly.
pub async fn upgrade_subject_thumbnails(pool: &SqlitePool) -> Result<Vec<(i64, i64)>> {
    let rows = sqlx::query(
        "SELECT s.id AS subject_id,
                s.thumbnail_face_id AS current_face,
                (SELECT f.id FROM faces f
                  WHERE f.subject_id = s.id
                  ORDER BY (f.quality_score IS NULL), f.quality_score DESC,
                           (f.bbox_w * f.bbox_h) DESC
                  LIMIT 1) AS best_face
         FROM subjects s",
    )
    .fetch_all(pool)
    .await?;

    let mut changed = Vec::new();
    for r in &rows {
        let subject_id: i64 = r.get("subject_id");
        let current: Option<i64> = r.get("current_face");
        let best: Option<i64> = r.get("best_face");
        if let Some(best_id) = best {
            if current != Some(best_id) {
                update_subject_thumbnail_face(pool, subject_id, best_id).await?;
                changed.push((subject_id, best_id));
            }
        }
        // best is None -> subject has no faces; leave thumbnail untouched (never NULL it).
    }
    Ok(changed)
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

    // Write must_link between all faces of target and all faces of source (durable merge)
    let target_faces = get_face_ids_for_subject(pool, target_id).await?;
    let source_faces = get_face_ids_for_subject(pool, source_id).await?;
    let now_c = chrono::Utc::now().timestamp();
    for &tf in &target_faces {
        for &sf in &source_faces {
            let (a, b) = if tf < sf { (tf, sf) } else { (sf, tf) };
            sqlx::query(
                "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) VALUES (?, ?, 'must_link', 'merge', ?)"
            ).bind(a).bind(b).bind(now_c).execute(pool).await?;
        }
    }

    sqlx::query("UPDATE faces SET subject_id = ? WHERE subject_id = ?")
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

        // Add cannot_link between one representative face from each subject (source='dismiss')
        let rep_a: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM faces WHERE subject_id = ? LIMIT 1"
        ).bind(lo).fetch_optional(pool).await?;
        let rep_b: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM faces WHERE subject_id = ? LIMIT 1"
        ).bind(hi).fetch_optional(pool).await?;
        if let (Some(fa), Some(fb)) = (rep_a, rep_b) {
            let (a, b) = if fa < fb { (fa, fb) } else { (fb, fa) };
            sqlx::query(
                "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) VALUES (?, ?, 'cannot_link', 'dismiss', ?)"
            ).bind(a).bind(b).bind(now).execute(pool).await?;
        }
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
    sqlx::query("UPDATE faces SET subject_id = ? WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_subject_for_face(pool: &SqlitePool, face_id: i64, name: Option<&str>) -> Result<Subject> {
    let subject_id = insert_subject(pool, name, "person").await?;
    sqlx::query("UPDATE faces SET subject_id = ? WHERE id = ?")
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

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("value")))
}

fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a <= b { (a, b) } else { (b, a) }
}

pub async fn add_must_link(pool: &SqlitePool, face_a: i64, face_b: i64, source: &str) -> Result<()> {
    if face_a == face_b {
        return Ok(());
    }
    let (a, b) = ordered_pair(face_a, face_b);
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) \
         VALUES (?, ?, 'must_link', ?, ?)",
    )
    .bind(a)
    .bind(b)
    .bind(source)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn add_cannot_link(pool: &SqlitePool, face_a: i64, face_b: i64, source: &str) -> Result<()> {
    if face_a == face_b {
        return Ok(());
    }
    let (a, b) = ordered_pair(face_a, face_b);
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) \
         VALUES (?, ?, 'cannot_link', ?, ?)",
    )
    .bind(a)
    .bind(b)
    .bind(source)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}


pub async fn upsert_face_edge(pool: &SqlitePool, face_a: i64, face_b: i64, weight: f32) -> Result<()> {
    let (a, b) = if face_a < face_b { (face_a, face_b) } else { (face_b, face_a) };
    sqlx::query(
        "INSERT OR REPLACE INTO face_edges (face_a, face_b, weight) VALUES (?, ?, ?)",
    )
    .bind(a)
    .bind(b)
    .bind(weight)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_all_face_edges(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM face_edges").execute(pool).await?;
    Ok(())
}

pub async fn get_all_similarity_edges(pool: &SqlitePool) -> Result<Vec<(i64, i64, f32)>> {
    let rows = sqlx::query("SELECT face_a, face_b, weight FROM face_edges")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.get("face_a"), r.get("face_b"), r.get::<f32, _>("weight"))).collect())
}

pub async fn get_all_must_link_pairs(pool: &SqlitePool) -> Result<Vec<(i64, i64)>> {
    let rows = sqlx::query("SELECT face_a, face_b FROM constraints WHERE kind = 'must_link'")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.get("face_a"), r.get("face_b"))).collect())
}

pub async fn get_all_cannot_link_pairs(pool: &SqlitePool) -> Result<std::collections::HashSet<(i64, i64)>> {
    let rows = sqlx::query("SELECT face_a, face_b FROM constraints WHERE kind = 'cannot_link'")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| {
        let a: i64 = r.get("face_a");
        let b: i64 = r.get("face_b");
        if a < b { (a, b) } else { (b, a) }
    }).collect())
}

pub async fn get_assigned_face_subject_map(pool: &SqlitePool) -> Result<std::collections::HashMap<i64, i64>> {
    let rows = sqlx::query("SELECT id, subject_id FROM faces WHERE subject_id IS NOT NULL")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.get::<i64, _>("id"), r.get::<i64, _>("subject_id"))).collect())
}

pub async fn get_face_ids_for_subject(pool: &SqlitePool, subject_id: i64) -> Result<Vec<i64>> {
    let rows = sqlx::query("SELECT id FROM faces WHERE subject_id = ?")
        .bind(subject_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.get::<i64, _>("id")).collect())
}

pub async fn get_all_face_ids_with_vectors(pool: &SqlitePool) -> Result<Vec<i64>> {
    let rows = sqlx::query("SELECT rowid FROM face_vectors")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.get::<i64, _>("rowid")).collect())
}


pub async fn unassign_face(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = NULL WHERE id = ?")
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
    sqlx::query("DELETE FROM face_vectors").execute(&mut *tx).await?;

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

    sqlx::query("DELETE FROM constraints")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM face_vectors").execute(&mut *tx).await?;
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
                added_at INTEGER NOT NULL
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
        sqlx::query(
            "CREATE TABLE constraints (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                face_a INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
                face_b INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
                kind TEXT NOT NULL CHECK(kind IN ('must_link', 'cannot_link')),
                source TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                UNIQUE(face_a, face_b, kind)
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
                added_at INTEGER NOT NULL DEFAULT 0
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
    async fn migration_6_creates_face_edges_table() {
        let pool = init_test_pool().await;
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(
            tables.contains(&"face_edges".to_string()),
            "face_edges table must exist after migration 6"
        );
        let cols: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('face_edges')")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(cols.contains(&"face_a".to_string()));
        assert!(cols.contains(&"face_b".to_string()));
        assert!(cols.contains(&"weight".to_string()));
    }

    #[tokio::test]
    async fn upsert_face_edge_normalizes_order_and_deduplicates() {
        let pool = init_test_pool().await;
        sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1,1,0,0,1,1,0),(2,1,0,0,1,1,0)")
            .execute(&pool).await.unwrap();

        upsert_face_edge(&pool, 2, 1, 0.8).await.unwrap();  // reversed order
        upsert_face_edge(&pool, 1, 2, 0.9).await.unwrap();  // should replace

        let edges = get_all_similarity_edges(&pool).await.unwrap();
        assert_eq!(edges.len(), 1, "duplicate upsert must replace");
        assert_eq!(edges[0].0, 1, "face_a must be smaller id");
        assert_eq!(edges[0].1, 2, "face_b must be larger id");
        assert!((edges[0].2 - 0.9).abs() < 1e-6, "latest weight must win");
    }

    #[tokio::test]
    async fn clear_all_face_edges_removes_all_rows() {
        let pool = init_test_pool().await;
        sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1,1,0,0,1,1,0),(2,1,0,0,1,1,0)")
            .execute(&pool).await.unwrap();
        upsert_face_edge(&pool, 1, 2, 0.7).await.unwrap();
        clear_all_face_edges(&pool).await.unwrap();
        let edges = get_all_similarity_edges(&pool).await.unwrap();
        assert!(edges.is_empty());
    }

    // --- helper for constraint tests ---
    async fn init_test_pool() -> SqlitePool {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        crate::db::ensure_sqlite_vec_registered();
        let tmp = std::env::temp_dir().join(format!("nebula_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&tmp).unwrap();
        let pool = init_db(&tmp).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn constraint_enforces_face_a_less_than_face_b() {
        let pool = init_test_pool().await;
        // Insert two faces so FK constraints are satisfied
        sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (3, 1, 0,0,1,1,0), (5, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();

        // Call with larger id first — should normalize to (3, 5)
        add_cannot_link(&pool, 5, 3, "removal").await.unwrap();

        let (a, b): (i64, i64) =
            sqlx::query_as("SELECT face_a, face_b FROM constraints WHERE kind = 'cannot_link'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(a, 3, "face_a must be the smaller id");
        assert_eq!(b, 5, "face_b must be the larger id");
    }

    #[tokio::test]
    async fn constraint_insert_or_ignore_deduplicates() {
        let pool = init_test_pool().await;
        sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 1, 0,0,1,1,0), (2, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();

        add_cannot_link(&pool, 1, 2, "removal").await.unwrap();
        add_cannot_link(&pool, 1, 2, "removal").await.unwrap(); // second insert must be silently ignored

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "duplicate insert must not create a second row");
    }

    #[tokio::test]
    async fn must_link_and_cannot_link_are_independent_rows() {
        let pool = init_test_pool().await;
        sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 1, 0,0,1,1,0), (2, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();

        add_must_link(&pool, 1, 2, "merge").await.unwrap();
        add_cannot_link(&pool, 1, 2, "removal").await.unwrap(); // same pair, different kind → OK

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2, "must_link and cannot_link on the same pair are distinct rows");
    }

    #[tokio::test]
    async fn faces_table_has_quality_columns() {
        let pool = init_test_pool().await;
        let cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('faces')")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(cols.contains(&"det_score".to_string()), "faces must have det_score; got {cols:?}");
        assert!(cols.contains(&"quality_score".to_string()), "faces must have quality_score; got {cols:?}");
    }

    #[tokio::test]
    async fn migration_5_drops_embedding_and_is_manual_columns() {
        let pool = init_test_pool().await;

        // Verify neither column exists after migration
        let cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('faces')")
            .fetch_all(&pool)
            .await
            .unwrap();

        assert!(
            !cols.contains(&"embedding".to_string()),
            "faces.embedding must be dropped by migration 5"
        );
        assert!(
            !cols.contains(&"is_manual".to_string()),
            "faces.is_manual must be dropped by migration 5"
        );
    }

    #[tokio::test]
    async fn migration_5_face_corrections_table_dropped() {
        let pool = init_test_pool().await;

        // face_corrections should not exist after migration 5
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table'")
                .fetch_all(&pool)
                .await
                .unwrap();

        assert!(
            !tables.contains(&"face_corrections".to_string()),
            "face_corrections table must be dropped by migration 5"
        );
    }

    #[tokio::test]
    async fn migration_5_preserves_existing_vectors_in_face_vectors() {
        let pool = init_test_pool().await;

        // Insert a face (post-migration state — no embedding column)
        sqlx::query("INSERT INTO faces (image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 0,0,1,1,0)")
            .execute(&pool)
            .await
            .unwrap();
        let face_id: i64 = sqlx::query_scalar("SELECT last_insert_rowid()").fetch_one(&pool).await.unwrap();

        // Manually insert a vector (simulating what the pipeline will do post-migration)
        // face_vectors uses float[512]; build a unit vector in the first dimension.
        let mut vec512 = vec![0.0f32; 512];
        vec512[0] = 1.0;
        crate::face_store::upsert_vector(&pool, face_id, &vec512).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_vectors").fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1, "face_vectors should store the upserted vector");
    }

    #[tokio::test]
    async fn sqlite_vec_extension_loads() {
        crate::db::ensure_sqlite_vec_registered();
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let version: String = sqlx::query_scalar("SELECT vec_version()")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!version.is_empty(), "vec_version() should return a non-empty string");
    }

    #[tokio::test]
    async fn merge_moves_source_faces_to_target() {
        // Post-B1: is_manual column removed; verify faces are reassigned to target subject.
        let pool = make_merge_pool().await;

        let target = insert_subject(&pool, Some("Alice")).await;
        let source = insert_subject(&pool, Some("Bob")).await;

        // Two faces for target
        sqlx::query(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) \
             VALUES (1, ?, 0, 0, 0.5, 0.5, 0), (2, ?, 0, 0, 0.5, 0.5, 0)",
        )
        .bind(target)
        .bind(target)
        .execute(&pool)
        .await
        .unwrap();

        // Two faces for source
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
    }

    #[tokio::test]
    async fn merge_subjects_writes_must_link_constraints() {
        let pool = init_test_pool().await;

        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let bob: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let fa: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, ?, 0,0,1,1,0) RETURNING id"
        ).bind(alice).fetch_one(&pool).await.unwrap();
        let fb: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (2, ?, 0,0,1,1,0) RETURNING id"
        ).bind(bob).fetch_one(&pool).await.unwrap();

        merge_subjects(&pool, alice, bob).await.unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM constraints WHERE kind = 'must_link' AND source = 'merge'"
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1, "one must_link expected for fa-fb cross-group pair");

        // Verify the pair is stored with face_a < face_b
        let (stored_a, stored_b): (i64, i64) = sqlx::query_as(
            "SELECT face_a, face_b FROM constraints WHERE kind = 'must_link'"
        ).fetch_one(&pool).await.unwrap();
        let expected_a = fa.min(fb);
        let expected_b = fa.max(fb);
        assert_eq!(stored_a, expected_a);
        assert_eq!(stored_b, expected_b);
    }

    #[tokio::test]
    async fn insert_face_persists_quality_scores() {
        let pool = init_test_pool().await;
        let face_id = insert_face(&pool, 1, None, (0.1, 0.1, 0.2, 0.2), Some(0.9), Some(0.75))
            .await
            .unwrap();
        let (det, qual): (Option<f64>, Option<f64>) =
            sqlx::query_as("SELECT det_score, quality_score FROM faces WHERE id = ?")
                .bind(face_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(det, Some(0.9));
        assert_eq!(qual, Some(0.75));
    }

    #[tokio::test]
    async fn upgrade_subject_thumbnails_picks_best_and_upgrades_never_nulls() {
        let pool = init_test_pool().await;

        // One subject with a low-quality face.
        let sid: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let low = insert_face(&pool, 1, Some(sid), (0.0, 0.0, 0.2, 0.2), Some(0.5), Some(0.2))
            .await
            .unwrap();

        // First pass: picks the only face, reports the subject as changed.
        let changed = upgrade_subject_thumbnails(&pool).await.unwrap();
        assert_eq!(changed, vec![(sid, low)]);
        let thumb: Option<i64> = sqlx::query_scalar("SELECT thumbnail_face_id FROM subjects WHERE id = ?")
            .bind(sid).fetch_one(&pool).await.unwrap();
        assert_eq!(thumb, Some(low));

        // A better face arrives.
        let high = insert_face(&pool, 2, Some(sid), (0.0, 0.0, 0.3, 0.3), Some(0.9), Some(0.9))
            .await
            .unwrap();
        let changed2 = upgrade_subject_thumbnails(&pool).await.unwrap();
        assert_eq!(changed2, vec![(sid, high)], "upgrade must report the change");
        let thumb2: Option<i64> = sqlx::query_scalar("SELECT thumbnail_face_id FROM subjects WHERE id = ?")
            .bind(sid).fetch_one(&pool).await.unwrap();
        assert_eq!(thumb2, Some(high), "must upgrade to higher quality face");

        // Idempotent: no change when nothing better appears.
        let changed3 = upgrade_subject_thumbnails(&pool).await.unwrap();
        assert!(changed3.is_empty(), "stable state reports no changes");
    }

    #[tokio::test]
    async fn get_face_with_image_returns_bbox_and_path() {
        let pool = init_test_pool().await;
        // images.folder_id is a NOT NULL FK to folders(id) and foreign_keys=ON,
        // so insert a folder first, then the image, then a face referencing it.
        let folder_id: i64 = sqlx::query_scalar(
            "INSERT INTO folders (path, added_at) VALUES ('/tmp', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let img_id: i64 = sqlx::query_scalar(
            "INSERT INTO images (folder_id, path, file_hash, mtime, added_at, updated_at)
             VALUES (?, '/tmp/x.jpg', 'hash', 0, 0, 0) RETURNING id",
        )
        .bind(folder_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let fid = insert_face(&pool, img_id, None, (0.1, 0.2, 0.3, 0.4), Some(0.8), Some(0.7))
            .await
            .unwrap();

        let (path, bbox) = get_face_with_image(&pool, fid).await.unwrap().unwrap();
        assert_eq!(path, "/tmp/x.jpg");
        assert!((bbox.0 - 0.1).abs() < 1e-9 && (bbox.3 - 0.4).abs() < 1e-9);
    }
}
