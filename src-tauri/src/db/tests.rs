use sqlx::SqlitePool;

use crate::db::init_db;
use crate::library::repo::{
    insert_folder, insert_image, get_image_by_id,
    update_thumbnail_path, update_preview_path, images_needing_preview,
};
use crate::people::repo::{
    get_merge_suggestions, dismiss_merge_suggestion, get_dismissed_pair_set,
    merge_subjects, upsert_face_edge, clear_all_face_edges, get_all_similarity_edges,
    add_cannot_link, add_must_link, insert_face, upgrade_subject_thumbnails,
    get_face_with_image,
};
use crate::search::text::{normalize, like_pattern};
use crate::tags::repo::{
    add_subject_tag, get_tag_image_ids_ordered, search_subjects_matching,
    get_subject_tags, list_tags_with_counts, rename_tag, remove_subject_tag, delete_tag,
};

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
    sqlx::query(
        "CREATE TABLE tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            name_normalized TEXT NOT NULL UNIQUE,
            added_at INTEGER NOT NULL
        )"
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "CREATE TABLE subject_tags (
            subject_id INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            added_at INTEGER NOT NULL,
            PRIMARY KEY (subject_id, tag_id)
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
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test]
async fn update_thumbnail_path_persists_and_is_readable() {
    let pool = make_images_pool().await;

    let folder_id: i64 = sqlx::query_scalar(
        "INSERT INTO folders (path, added_at) VALUES ('/test/photos', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let image_id = insert_image(&pool, folder_id, "/test/photos/img.jpg", "abc123", 1024, 0)
        .await
        .unwrap();

    let before = get_image_by_id(&pool, image_id).await.unwrap().unwrap();
    assert!(
        before.thumbnail_path.is_none(),
        "thumbnail_path should be NULL before update"
    );

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

    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    sqlx::query(
        "INSERT INTO dismissed_pairs (subject_id_a, subject_id_b, dismissed_at) VALUES (?, ?, 0)"
    ).bind(lo).bind(hi).execute(&pool).await.unwrap();

    sqlx::query(
        "INSERT INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at) VALUES (?, ?, 0.95, 0)"
    ).bind(lo).bind(hi).execute(&pool).await.unwrap();

    let dismissed = get_dismissed_pair_set(&pool).await.unwrap();
    assert!(dismissed.contains(&(lo, hi)), "dismissed set should include the pair");

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
async fn upsert_face_edge_normalizes_order_and_deduplicates() {
    let pool = init_test_pool().await;
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1,1,0,0,1,1,0),(2,1,0,0,1,1,0)")
        .execute(&pool).await.unwrap();

    upsert_face_edge(&pool, 2, 1, 0.8).await.unwrap();
    upsert_face_edge(&pool, 1, 2, 0.9).await.unwrap();

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

async fn init_test_pool() -> SqlitePool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    crate::db::ensure_sqlite_vec_registered();
    let tmp = std::env::temp_dir().join(format!("nebula_test_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&tmp).unwrap();
    init_db(&tmp).await.unwrap()
}

#[tokio::test]
async fn constraint_enforces_face_a_less_than_face_b() {
    let pool = init_test_pool().await;
    sqlx::query("INSERT INTO faces (id, image_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (3, 1, 0,0,1,1,0), (5, 1, 0,0,1,1,0)").execute(&pool).await.unwrap();

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
    add_cannot_link(&pool, 1, 2, "removal").await.unwrap();

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
    add_cannot_link(&pool, 1, 2, "removal").await.unwrap();

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
    let pool = make_merge_pool().await;

    let target = insert_subject(&pool, Some("Alice")).await;
    let source = insert_subject(&pool, Some("Bob")).await;

    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) \
         VALUES (1, ?, 0, 0, 0.5, 0.5, 0), (2, ?, 0, 0, 0.5, 0.5, 0)",
    )
    .bind(target)
    .bind(target)
    .execute(&pool)
    .await
    .unwrap();

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

    let (stored_a, stored_b): (i64, i64) = sqlx::query_as(
        "SELECT face_a, face_b FROM constraints WHERE kind = 'must_link'"
    ).fetch_one(&pool).await.unwrap();
    let expected_a = fa.min(fb);
    let expected_b = fa.max(fb);
    assert_eq!(stored_a, expected_a);
    assert_eq!(stored_b, expected_b);
}

#[tokio::test]
async fn merge_subjects_preserves_and_unifies_tags() {
    let pool = init_test_pool().await;

    let target: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Target', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let source: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Source', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();

    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, ?, 0,0,1,1,0)"
    ).bind(target).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (2, ?, 0,0,1,1,0)"
    ).bind(source).execute(&pool).await.unwrap();

    let target_only = add_subject_tag(&pool, target, "target-only").await.unwrap();
    let source_only = add_subject_tag(&pool, source, "source-only").await.unwrap();
    let shared = add_subject_tag(&pool, target, "shared").await.unwrap();
    add_subject_tag(&pool, source, "shared").await.unwrap();

    merge_subjects(&pool, target, source).await.unwrap();

    let surviving_tags = get_subject_tags(&pool, target).await.unwrap();
    let tag_names: Vec<String> = surviving_tags.iter().map(|t| t.name.clone()).collect();
    assert!(tag_names.contains(&"target-only".to_string()));
    assert!(tag_names.contains(&"source-only".to_string()), "source-only tag must be transferred to target");
    assert!(tag_names.contains(&"shared".to_string()));
    assert_eq!(surviving_tags.len(), 3, "shared tag must not be duplicated");

    let source_tags = get_subject_tags(&pool, source).await.unwrap();
    assert!(source_tags.is_empty(), "source subject tags should be gone after merge");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subjects")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);

    let list = list_tags_with_counts(&pool).await.unwrap();
    assert_eq!(list.len(), 3);
    for t in [&target_only, &source_only, &shared] {
        let found = list.iter().find(|lt| lt.id == t.id).expect("tag still exists");
        assert_eq!(found.subject_count, 1, "each surviving tag should be attached exactly once");
    }
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

    let sid: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let low = insert_face(&pool, 1, Some(sid), (0.0, 0.0, 0.2, 0.2), Some(0.5), Some(0.2))
        .await
        .unwrap();

    let changed = upgrade_subject_thumbnails(&pool).await.unwrap();
    assert_eq!(changed, vec![(sid, low)]);
    let thumb: Option<i64> = sqlx::query_scalar("SELECT thumbnail_face_id FROM subjects WHERE id = ?")
        .bind(sid).fetch_one(&pool).await.unwrap();
    assert_eq!(thumb, Some(low));

    let high = insert_face(&pool, 2, Some(sid), (0.0, 0.0, 0.3, 0.3), Some(0.9), Some(0.9))
        .await
        .unwrap();
    let changed2 = upgrade_subject_thumbnails(&pool).await.unwrap();
    assert_eq!(changed2, vec![(sid, high)], "upgrade must report the change");
    let thumb2: Option<i64> = sqlx::query_scalar("SELECT thumbnail_face_id FROM subjects WHERE id = ?")
        .bind(sid).fetch_one(&pool).await.unwrap();
    assert_eq!(thumb2, Some(high), "must upgrade to higher quality face");

    let changed3 = upgrade_subject_thumbnails(&pool).await.unwrap();
    assert!(changed3.is_empty(), "stable state reports no changes");
}

#[tokio::test]
async fn get_face_with_image_returns_bbox_and_path() {
    let pool = init_test_pool().await;
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

#[tokio::test]
async fn test_tag_image_ids_ordered_by_subject_count() {
    let dir = std::env::temp_dir().join(format!("nebula_tagimgs_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();

    let folder_id = insert_folder(&pool, "/tmp/tag_test").await.unwrap();
    let img_a = insert_image(&pool, folder_id, "/tmp/tag_test/a.jpg", "ha", 1, 1).await.unwrap();
    let img_b = insert_image(&pool, folder_id, "/tmp/tag_test/b.jpg", "hb", 1, 1).await.unwrap();
    let img_c = insert_image(&pool, folder_id, "/tmp/tag_test/c.jpg", "hc", 1, 1).await.unwrap();
    let img_d = insert_image(&pool, folder_id, "/tmp/tag_test/d.jpg", "hd", 1, 1).await.unwrap();
    sqlx::query("UPDATE images SET deleted_at = 1 WHERE id = ?").bind(img_d).execute(&pool).await.unwrap();

    let s1: i64 = sqlx::query_scalar("INSERT INTO subjects (name, type, added_at) VALUES ('Sub1', 'person', 0) RETURNING id").fetch_one(&pool).await.unwrap();
    let s2: i64 = sqlx::query_scalar("INSERT INTO subjects (name, type, added_at) VALUES ('Sub2', 'person', 0) RETURNING id").fetch_one(&pool).await.unwrap();

    sqlx::query("INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (?, ?, 0,0,1,1,0)").bind(img_a).bind(s1).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (?, ?, 0,0,1,1,0)").bind(img_a).bind(s2).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (?, ?, 0,0,1,1,0)").bind(img_b).bind(s1).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (?, ?, 0,0,1,1,0)").bind(img_d).bind(s1).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (?, ?, 0,0,1,1,0)").bind(img_d).bind(s2).execute(&pool).await.unwrap();

    add_subject_tag(&pool, s1, "cabin-9").await.unwrap();
    add_subject_tag(&pool, s2, "cabin-9").await.unwrap();

    let ids = get_tag_image_ids_ordered(&pool, "cabin 9").await.unwrap();
    assert_eq!(ids, vec![img_a, img_b]);

    let _ = img_c;
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_normalize_strips_accents_and_case() {
    assert_eq!(normalize("Cabaña-21"), "cabana-21");
    assert_eq!(normalize("JOSÉ"), "jose");
    assert_eq!(normalize("  Über  "), "uber");
    assert_eq!(normalize("plain"), "plain");
    assert_eq!(normalize(""), "");
}

#[test]
fn test_like_pattern_wildcards_and_escaping() {
    assert_eq!(like_pattern("cabin 9").as_deref(), Some("%cabin%9%"));
    assert_eq!(like_pattern("50%_off").as_deref(), Some("%50\\%\\_off%"));
    assert_eq!(like_pattern("   "), None);
}

#[tokio::test]
async fn test_search_subjects_matching_multiword_and_wildcards() {
    let dir = std::env::temp_dir().join(format!("nebula_subjmw_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();

    let s: i64 = sqlx::query_scalar("INSERT INTO subjects (name, type, added_at) VALUES ('Ana', 'person', 0) RETURNING id").fetch_one(&pool).await.unwrap();
    add_subject_tag(&pool, s, "Cabin-9").await.unwrap();

    let hits = search_subjects_matching(&pool, "cabin 9").await.unwrap();
    assert_eq!(hits.len(), 1);
    let imgs_query = like_pattern("cabin 9");
    assert_eq!(imgs_query.as_deref(), Some("%cabin%9%"));

    assert!(search_subjects_matching(&pool, "ca%n").await.unwrap().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn test_search_subjects_matching() {
    let dir = std::env::temp_dir().join(format!("nebula_subjtag_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();

    let jose: i64 = sqlx::query_scalar("INSERT INTO subjects (name, type, added_at) VALUES ('José', 'person', 0) RETURNING id").fetch_one(&pool).await.unwrap();
    let maria: i64 = sqlx::query_scalar("INSERT INTO subjects (name, type, added_at) VALUES ('Maria', 'person', 0) RETURNING id").fetch_one(&pool).await.unwrap();
    add_subject_tag(&pool, maria, "Cabaña-21").await.unwrap();

    let hits = search_subjects_matching(&pool, "jose").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].subject.name.as_deref(), Some("José"));

    let hits = search_subjects_matching(&pool, "cabana").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].subject.name.as_deref(), Some("Maria"));
    assert_eq!(hits[0].tags[0].name, "Cabaña-21");

    let hits = search_subjects_matching(&pool, "maria").await.unwrap();
    assert_eq!(hits.len(), 1);

    assert!(search_subjects_matching(&pool, "zzz").await.unwrap().is_empty());

    let _ = jose;
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn compute_blake3_is_deterministic_and_content_sensitive() {
    use crate::library::hasher::compute_blake3;

    let dir = std::env::temp_dir().join(format!("nebula_blake3_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    std::fs::write(&a, b"hello world").unwrap();
    std::fs::write(&b, b"hello worlD").unwrap();

    let h1 = compute_blake3(&a).await.unwrap();
    let h2 = compute_blake3(&a).await.unwrap();
    let h3 = compute_blake3(&b).await.unwrap();

    assert_eq!(h1, h2, "same content must hash identically");
    assert_ne!(h1, h3, "different content must hash differently");
    assert_eq!(h1.len(), 64, "BLAKE3 hex digest is 64 chars");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn test_tag_crud() {
    let dir = std::env::temp_dir().join(format!("nebula_tagcrud_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();

    let s1: i64 = sqlx::query_scalar("INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id").fetch_one(&pool).await.unwrap();
    let s2: i64 = sqlx::query_scalar("INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id").fetch_one(&pool).await.unwrap();

    let t1 = add_subject_tag(&pool, s1, "Cabaña-21").await.unwrap();
    let t2 = add_subject_tag(&pool, s2, "cabana-21").await.unwrap();
    assert_eq!(t1.id, t2.id);
    assert_eq!(t2.name, "Cabaña-21");

    add_subject_tag(&pool, s1, "cabaña-21").await.unwrap();
    let tags = get_subject_tags(&pool, s1).await.unwrap();
    assert_eq!(tags.len(), 1);

    let all = list_tags_with_counts(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].subject_count, 2);

    assert!(add_subject_tag(&pool, s1, "   ").await.is_err());

    let other = add_subject_tag(&pool, s1, "cabin-3").await.unwrap();
    assert!(rename_tag(&pool, other.id, "CABAÑA-21").await.is_err());
    rename_tag(&pool, other.id, "cabin-4").await.unwrap();

    remove_subject_tag(&pool, s2, t1.id).await.unwrap();
    assert_eq!(list_tags_with_counts(&pool).await.unwrap().iter().find(|t| t.id == t1.id).unwrap().subject_count, 1);
    delete_tag(&pool, t1.id).await.unwrap();
    assert!(get_subject_tags(&pool, s1).await.unwrap().iter().all(|t| t.id != t1.id));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn pending_hash_batch_and_apply_results_round_trip() {
    use crate::library::repo::{get_pending_hash_batch, apply_hash_results};

    let dir = std::env::temp_dir().join(format!("nebula_hashbatch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    let fid = insert_folder(&pool, "/tmp/f").await.unwrap();

    // Three PENDING images (insert_image leaves hash_status at its 'PENDING' default).
    let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "", 10, 100).await.unwrap();
    let b = insert_image(&pool, fid, "/tmp/f/b.jpg", "", 20, 200).await.unwrap();
    let c = insert_image(&pool, fid, "/tmp/f/c.jpg", "", 30, 300).await.unwrap();

    // Soft-delete c: it must NOT appear in the pending batch.
    sqlx::query("UPDATE images SET deleted_at = 1 WHERE id = ?").bind(c).execute(&pool).await.unwrap();

    let batch = get_pending_hash_batch(&pool, 10).await.unwrap();
    let ids: Vec<i64> = batch.iter().map(|(id, _, _)| *id).collect();
    assert!(ids.contains(&a) && ids.contains(&b), "live PENDING rows must be returned");
    assert!(!ids.contains(&c), "soft-deleted rows must be excluded");
    // mtime is carried so writes can be guarded against concurrent modification.
    assert!(batch.iter().any(|(id, _, m)| *id == a && *m == 100));

    // Apply: a succeeds with a hash, b fails (None).
    apply_hash_results(&pool, &[(a, 100, Some("deadbeef".to_string())), (b, 200, None)])
        .await
        .unwrap();

    let img_a = get_image_by_id(&pool, a).await.unwrap().unwrap();
    let img_b = get_image_by_id(&pool, b).await.unwrap().unwrap();
    assert_eq!(img_a.file_hash, "deadbeef");
    assert_eq!(img_a.hash_status, "DONE");
    assert_eq!(img_b.hash_status, "FAILED");

    // a is no longer PENDING, so a re-read returns only nothing new.
    let after = get_pending_hash_batch(&pool, 10).await.unwrap();
    assert!(after.iter().all(|(id, _, _)| *id != a && *id != b));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn apply_hash_results_is_guarded_by_mtime() {
    use crate::library::repo::apply_hash_results;

    let dir = std::env::temp_dir().join(format!("nebula_hashguard_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    let fid = insert_folder(&pool, "/tmp/f").await.unwrap();
    let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "", 10, 100).await.unwrap();

    // The file was re-touched while hashing was in flight: mtime is now 999.
    sqlx::query("UPDATE images SET mtime = 999 WHERE id = ?").bind(a).execute(&pool).await.unwrap();

    // Applying a result computed against the OLD mtime (100) must be a no-op.
    apply_hash_results(&pool, &[(a, 100, Some("stale".to_string()))]).await.unwrap();

    let img = get_image_by_id(&pool, a).await.unwrap().unwrap();
    assert_ne!(img.file_hash, "stale", "stale-mtime write must not clobber a re-touched file");
    assert_eq!(img.hash_status, "PENDING", "row stays PENDING so the worker re-hashes it");

    std::fs::remove_dir_all(&dir).ok();
}
