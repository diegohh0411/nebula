//! Persistence foundation: pool, init, schema, sqlite-vec registration.
//! All domain queries live in per-slice repo.rs modules (TT-63).

#[cfg(test)]
mod tests;

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::sync::Once;

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
    hash_status            TEXT NOT NULL DEFAULT 'PENDING',
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
CREATE INDEX IF NOT EXISTS idx_images_done     ON images(semantic_analysis_done, subject_analysis_done) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS embedding_queue (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id     INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    pipeline     TEXT NOT NULL DEFAULT 'semantic',
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    scheduled_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_queue_scheduled ON embedding_queue(scheduled_at);
CREATE INDEX IF NOT EXISTS idx_queue_image     ON embedding_queue(image_id);

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
    added_at    INTEGER NOT NULL,
    det_score      REAL,
    quality_score  REAL
);

CREATE INDEX IF NOT EXISTS idx_faces_image ON faces(image_id);
CREATE INDEX IF NOT EXISTS idx_faces_subject ON faces(subject_id);

CREATE VIRTUAL TABLE IF NOT EXISTS face_vectors USING vec0(embedding float[512]);

CREATE TABLE IF NOT EXISTS tags (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    name_normalized TEXT NOT NULL UNIQUE,
    added_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS subject_tags (
    subject_id INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (subject_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_subject_tags_tag ON subject_tags(tag_id);

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

CREATE TABLE IF NOT EXISTS dismissed_pairs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    dismissed_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_dismissed_pair ON dismissed_pairs(subject_id_a, subject_id_b);

CREATE TABLE IF NOT EXISTS constraints (
    face_a      INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    face_b      INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK(kind IN ('must_link', 'cannot_link')),
    source      TEXT NOT NULL CHECK(source IN ('merge', 'manual_assign', 'removal', 'dismiss')),
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (face_a, face_b, kind)
);

CREATE TABLE IF NOT EXISTS face_edges (
    face_a  INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    face_b  INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    weight  REAL NOT NULL,
    PRIMARY KEY (face_a, face_b)
);

CREATE TABLE IF NOT EXISTS saved_reports (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT NOT NULL,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    added_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS saved_report_tags (
    report_id INTEGER NOT NULL REFERENCES saved_reports(id) ON DELETE CASCADE,
    tag_id    INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (report_id, tag_id)
);
"#;

const VERSIONED_MIGRATIONS: &[(u32, &str)] = &[
    (
        1,
        "CREATE INDEX IF NOT EXISTS idx_images_done ON images(semantic_analysis_done, subject_analysis_done) WHERE deleted_at IS NULL",
    ),
    (
        2,
        "CREATE INDEX IF NOT EXISTS idx_queue_image ON embedding_queue(image_id)",
    ),
    (
        3,
        "CREATE TABLE IF NOT EXISTS saved_reports (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE, added_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS saved_report_tags (report_id INTEGER NOT NULL REFERENCES saved_reports(id) ON DELETE CASCADE, tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE, PRIMARY KEY (report_id, tag_id));"
    ),
    (
        4,
        "CREATE TABLE subjects_new (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, thumbnail_face_id INTEGER REFERENCES faces(id) ON DELETE SET NULL, type TEXT NOT NULL DEFAULT 'person', added_at INTEGER NOT NULL); \
         INSERT INTO subjects_new (id, name, thumbnail_face_id, type, added_at) SELECT id, name, thumbnail_face_id, type, added_at FROM subjects; \
         DROP TABLE subjects; \
         ALTER TABLE subjects_new RENAME TO subjects; \
         CREATE TABLE faces_new (id INTEGER PRIMARY KEY AUTOINCREMENT, image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE, subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL, bbox_x REAL NOT NULL, bbox_y REAL NOT NULL, bbox_w REAL NOT NULL, bbox_h REAL NOT NULL, added_at INTEGER NOT NULL, det_score REAL, quality_score REAL); \
         INSERT INTO faces_new (id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at, det_score, quality_score) SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at, det_score, quality_score FROM faces; \
         DROP TABLE faces; \
         ALTER TABLE faces_new RENAME TO faces; \
         CREATE INDEX IF NOT EXISTS idx_faces_image ON faces(image_id); \
         CREATE INDEX IF NOT EXISTS idx_faces_subject ON faces(subject_id);"
    ),
];

pub async fn init_db(data_dir: &Path) -> Result<SqlitePool> {
    ensure_sqlite_vec_registered();
    let db_path = data_dir.join("nebula.db");
    let opts = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true)
        // Applied to every connection the pool opens (not just the first),
        // so FK-based cascades stay enforced regardless of pool size.
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    sqlx::query("PRAGMA journal_mode=WAL;")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA synchronous=NORMAL;")
        .execute(&pool)
        .await?;

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

    let mut conn = pool.acquire().await?;
    for &(version, sql) in VERSIONED_MIGRATIONS {
        if current < version {
            sqlx::query("PRAGMA foreign_keys=OFF;").execute(&mut *conn).await?;
            for stmt in sql.split(';') {
                let s = stmt.trim();
                if !s.is_empty() {
                    sqlx::query(s).execute(&mut *conn).await?;
                }
            }
            sqlx::query("UPDATE schema_version SET version = ? WHERE rowid = 1")
                .bind(version)
                .execute(&mut *conn)
                .await?;
            sqlx::query("PRAGMA foreign_keys=ON;").execute(&mut *conn).await?;
        }
    }

    Ok(pool)
}
