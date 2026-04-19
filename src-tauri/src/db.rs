use anyhow::Result;
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, Row, SqlitePool};
use std::path::Path;

use crate::models::{ProcessingStatus, Folder, FolderWithCount, Image, Face, Subject};

const MIGRATIONS: &str = r#"
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
    date_taken             INTEGER,
    date_file              INTEGER NOT NULL,
    thumbnail_path         TEXT,
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
    added_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_faces_image ON faces(image_id);
CREATE INDEX IF NOT EXISTS idx_faces_subject ON faces(subject_id);

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
    cross_match_count INTEGER NOT NULL,
    total_pairs INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_merge_pair ON merge_suggestions(
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END
);
"#;

const POST_MIGRATIONS: &str = r#"
ALTER TABLE faces ADD COLUMN is_manual INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS face_corrections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    face_id INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    old_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    new_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_corrections_face ON face_corrections(face_id);
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

    for stmt in POST_MIGRATIONS.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            let _ = sqlx::query(s).execute(&pool).await;
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
        semantic_analysis_done: r.get::<i32, _>("semantic_analysis_done") != 0,
        subject_analysis_done: r.get::<i32, _>("subject_analysis_done") != 0,
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
                "UPDATE images SET file_hash = ?, date_file = ?, semantic_analysis_done = 0,
                 subject_analysis_done = 0, embedding = NULL, updated_at = ?, deleted_at = NULL WHERE id = ?",
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
            "INSERT INTO images (folder_id, path, file_hash, date_file, added_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
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
                    semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
             FROM images WHERE folder_id = ? AND deleted_at IS NULL
             ORDER BY COALESCE(date_taken, date_file) DESC",
        )
        .bind(fid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, folder_id, path, file_hash, date_taken, date_file, thumbnail_path,
                    semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
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
                semantic_analysis_done, subject_analysis_done, added_at, updated_at, deleted_at
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

pub async fn mark_semantic_analysis_done(pool: &SqlitePool, image_id: i64, embedding: &[u8]) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "UPDATE images SET embedding = ?, semantic_analysis_done = 1, updated_at = ? WHERE id = ?",
    )
    .bind(embedding)
    .bind(now)
    .bind(image_id)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM embedding_queue WHERE image_id = ? AND pipeline = 'semantic'")
        .bind(image_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_subject_analysis_done(pool: &SqlitePool, image_id: i64) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE images SET subject_analysis_done = 1, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(image_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM embedding_queue WHERE image_id = ? AND pipeline = 'subject'")
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
           (SELECT COUNT(*) FROM embedding_queue WHERE pipeline = 'semantic') as semantic_pending,
           (SELECT COUNT(*) FROM embedding_queue WHERE pipeline = 'subject') as subject_pending,
           (SELECT COUNT(*) FROM images WHERE semantic_analysis_done = 1 AND subject_analysis_done = 1 AND deleted_at IS NULL) as done",
    )
    .fetch_one(pool)
    .await?;
    Ok(ProcessingStatus {
        semantic_pending: row.get("semantic_pending"),
        subject_pending: row.get("subject_pending"),
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
        r#"SELECT DISTINCT i.id, i.folder_id, i.path, i.file_hash, i.date_taken, i.date_file, i.thumbnail_path,
                           i.semantic_analysis_done, i.subject_analysis_done, i.added_at, i.updated_at, i.deleted_at
           FROM images i
           JOIN faces f ON f.image_id = i.id
           WHERE f.subject_id = ? AND i.deleted_at IS NULL
           ORDER BY COALESCE(i.date_taken, i.date_file) DESC"#,
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

pub async fn get_all_faces_with_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Option<i64>, Vec<u8>, bool)>> {
    let rows = sqlx::query(
        "SELECT id, subject_id, embedding, is_manual FROM faces WHERE embedding IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let id: i64 = r.get("id");
            let subject_id: Option<i64> = r.get("subject_id");
            let emb: Option<Vec<u8>> = r.get("embedding");
            let is_manual: bool = r.get::<i32, _>("is_manual") != 0;
            emb.map(|e| (id, subject_id, e, is_manual))
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
    cross_match_count: i64,
    total_pairs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let (lo, hi) = if subject_id_a < subject_id_b {
        (subject_id_a, subject_id_b)
    } else {
        (subject_id_b, subject_id_a)
    };
    sqlx::query(
        "INSERT OR IGNORE INTO merge_suggestions (subject_id_a, subject_id_b, cross_match_count, total_pairs, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(lo)
    .bind(hi)
    .bind(cross_match_count)
    .bind(total_pairs)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_merge_suggestions(pool: &SqlitePool) -> Result<Vec<crate::models::MergeSuggestion>> {
    let rows = sqlx::query(
        r#"SELECT ms.id, ms.cross_match_count, ms.total_pairs,
                  sa.id as sa_id, sa.name as sa_name, sa.thumbnail_face_id as sa_thumbnail_face_id, sa.type as sa_type, sa.added_at as sa_added_at,
                  sb.id as sb_id, sb.name as sb_name, sb.thumbnail_face_id as sb_thumbnail_face_id, sb.type as sb_type, sb.added_at as sb_added_at
           FROM merge_suggestions ms
           JOIN subjects sa ON ms.subject_id_a = sa.id
           JOIN subjects sb ON ms.subject_id_b = sb.id"#,
    )
    .fetch_all(pool)
    .await?;

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
            cross_match_count: r.get("cross_match_count"),
            total_pairs: r.get("total_pairs"),
        })
        .collect())
}

pub async fn merge_subjects(pool: &SqlitePool, target_id: i64, source_id: i64) -> Result<()> {
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

pub async fn dismiss_merge_suggestion(pool: &SqlitePool, id: i64) -> Result<()> {
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

pub async fn get_faces_by_subject(pool: &SqlitePool, subject_id: i64) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT id, embedding FROM faces WHERE subject_id = ? AND embedding IS NOT NULL",
    )
    .bind(subject_id)
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

pub async fn unassign_face(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = NULL, is_manual = 1 WHERE id = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn reset_all_embeddings(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE images SET embedding = NULL, semantic_analysis_done = 0, subject_analysis_done = 0 WHERE deleted_at IS NULL")
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
