//! People service: assignment / merge orchestration, and per-image face
//! reprocessing that preserves ids across a face-recognition model switch.

use crate::people::repo as people_repo;
use anyhow::Result;
use sqlx::SqlitePool;

/// One face detected in the current reprocessing pass: relative bbox
/// (matching `faces.bbox_x/y/w/h`), detector confidence, composite quality
/// score, and the embedding vector produced by the active embedder.
pub struct DetectedFaceInput {
    pub bbox: (f64, f64, f64, f64),
    pub det_score: f64,
    pub quality_score: f64,
    pub embedding: Vec<f32>,
}

/// IoU threshold above which a new detection is considered the same physical
/// face as an existing row, so its id (and therefore `subject_id`,
/// constraints, and thumbnail references) survives a model switch.
pub const MATCH_IOU_THRESHOLD: f64 = 0.5;

/// Reconcile one image's face detections against its existing `faces` rows:
/// greedy highest-IoU-first matching (threshold `MATCH_IOU_THRESHOLD`), then:
/// - matched -> update the existing row in place (bbox/scores/embedder_id) and
///   replace its vector; the face id, `subject_id`, and any constraints survive.
/// - unmatched detection -> insert a fresh unassigned row + vector.
/// - unmatched existing row -> delete it (FK cascade removes constraints/edges;
///   its `face_vectors` row is deleted explicitly since vec0 has no FK support).
///
/// Serves both first-time analysis (`existing` empty -> everything inserts)
/// and re-analysis after a switch — no separate migration mode. Safe to retry:
/// each call re-reads `existing` from the database, so a detection that was
/// inserted by a prior failed attempt is matched (not duplicated) on retry.
pub async fn reprocess_image_faces(
    pool: &SqlitePool,
    image_id: i64,
    embedder_id: &str,
    detections: Vec<DetectedFaceInput>,
    existing: Vec<crate::people::models::Face>,
) -> Result<Vec<i64>> {
    let mut candidates: Vec<(usize, usize, f64)> = Vec::new();
    for (di, d) in detections.iter().enumerate() {
        for (ei, e) in existing.iter().enumerate() {
            let ebbox = (e.bbox_x, e.bbox_y, e.bbox_w, e.bbox_h);
            let iou = crate::people::bbox::iou(d.bbox, ebbox);
            if iou >= MATCH_IOU_THRESHOLD {
                candidates.push((di, ei, iou));
            }
        }
    }
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut used_detections: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut used_existing: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut matches: Vec<(usize, usize)> = Vec::new();
    for (di, ei, _iou) in candidates {
        if used_detections.contains(&di) || used_existing.contains(&ei) {
            continue;
        }
        used_detections.insert(di);
        used_existing.insert(ei);
        matches.push((di, ei));
    }

    let mut touched: Vec<i64> = Vec::new();

    for (di, ei) in &matches {
        let d = &detections[*di];
        let face_id = existing[*ei].id;
        people_repo::update_face_detection(pool, face_id, d.bbox, d.det_score, d.quality_score, embedder_id)
            .await?;
        crate::people::face_store::upsert_vector(pool, face_id, &d.embedding).await?;
        touched.push(face_id);
    }

    for (di, d) in detections.iter().enumerate() {
        if used_detections.contains(&di) {
            continue;
        }
        let face_id = people_repo::insert_face(
            pool,
            image_id,
            None,
            d.bbox,
            Some(d.det_score),
            Some(d.quality_score),
            embedder_id,
        )
        .await?;
        crate::people::face_store::upsert_vector(pool, face_id, &d.embedding).await?;
        touched.push(face_id);
    }

    for (ei, e) in existing.iter().enumerate() {
        if used_existing.contains(&ei) {
            continue;
        }
        people_repo::delete_face(pool, e.id).await?;
        crate::people::face_store::delete_vector(pool, e.id).await?;
    }

    Ok(touched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::people::models::Face;

    async fn init_test_pool() -> SqlitePool {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        crate::db::ensure_sqlite_vec_registered();
        let tmp = std::env::temp_dir().join(format!("nebula_service_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&tmp).unwrap();
        crate::db::init_db(&tmp).await.unwrap()
    }

    async fn seed_image(pool: &SqlitePool) -> i64 {
        let folder_id: i64 =
            sqlx::query_scalar("INSERT INTO folders (path, added_at) VALUES ('/tmp', 0) RETURNING id")
                .fetch_one(pool)
                .await
                .unwrap();
        sqlx::query_scalar(
            "INSERT INTO images (folder_id, path, file_hash, mtime, added_at, updated_at)
             VALUES (?, '/tmp/x.jpg', 'hash', 0, 0, 0) RETURNING id",
        )
        .bind(folder_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn emb(seed: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; 512];
        v[0] = seed;
        v
    }

    fn det(bbox: (f64, f64, f64, f64), seed: f32) -> DetectedFaceInput {
        DetectedFaceInput {
            bbox,
            det_score: 0.9,
            quality_score: 0.8,
            embedding: emb(seed),
        }
    }

    #[tokio::test]
    async fn first_time_analysis_inserts_all_detections_unassigned() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;

        let touched = reprocess_image_faces(
            &pool,
            image_id,
            "buffalo_s_recognition",
            vec![det((0.1, 0.1, 0.2, 0.2), 1.0), det((0.6, 0.6, 0.2, 0.2), 2.0)],
            vec![],
        )
        .await
        .unwrap();

        assert_eq!(touched.len(), 2);
        let rows: Vec<(Option<i64>, String)> =
            sqlx::query_as("SELECT subject_id, embedder_id FROM faces ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        for (subject_id, embedder_id) in rows {
            assert_eq!(subject_id, None, "first-time faces must be unassigned");
            assert_eq!(embedder_id, "buffalo_s_recognition");
        }
    }

    #[tokio::test]
    async fn matched_detection_preserves_face_id_and_subject_and_updates_embedder() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;
        let sid: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let face_id = crate::people::repo::insert_face(
            &pool,
            image_id,
            Some(sid),
            (0.10, 0.10, 0.20, 0.20),
            Some(0.5),
            Some(0.4),
            "buffalo_s_recognition",
        )
        .await
        .unwrap();
        crate::people::face_store::upsert_vector(&pool, face_id, &emb(1.0))
            .await
            .unwrap();
        let existing = vec![Face {
            id: face_id,
            image_id,
            subject_id: Some(sid),
            bbox_x: 0.10,
            bbox_y: 0.10,
            bbox_w: 0.20,
            bbox_h: 0.20,
            added_at: 0,



        }];

        // Slightly shifted bbox from the new model, but same physical face (IoU > 0.5).
        let touched = reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.11, 0.11, 0.20, 0.20), 9.0)],
            existing,
        )
        .await
        .unwrap();

        assert_eq!(touched, vec![face_id], "must reuse the existing face id, not insert a new one");
        let (subject_id, embedder_id, bbox_x): (Option<i64>, String, f64) =
            sqlx::query_as("SELECT subject_id, embedder_id, bbox_x FROM faces WHERE id = ?")
                .bind(face_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(subject_id, Some(sid), "subject_id must survive the match");
        assert_eq!(embedder_id, "antelopev2_recognition");
        assert_eq!(bbox_x, 0.11, "bbox must be updated to the new detection");
        let vec_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_vectors WHERE rowid = ?")
            .bind(face_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(vec_count, 1, "exactly one vector row must remain for the matched face");
    }

    #[tokio::test]
    async fn matched_face_preserves_constraint_rows() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;
        let face_a = crate::people::repo::insert_face(
            &pool, image_id, None, (0.0, 0.0, 0.2, 0.2), Some(0.5), Some(0.4), "buffalo_s_recognition",
        ).await.unwrap();
        let face_b = crate::people::repo::insert_face(
            &pool, image_id, None, (0.5, 0.5, 0.2, 0.2), Some(0.5), Some(0.4), "buffalo_s_recognition",
        ).await.unwrap();
        crate::people::repo::add_must_link(&pool, face_a, face_b, "merge").await.unwrap();

        let existing = vec![
            Face { id: face_a, image_id, subject_id: None, bbox_x: 0.0, bbox_y: 0.0, bbox_w: 0.2, bbox_h: 0.2, added_at: 0 },
            Face { id: face_b, image_id, subject_id: None, bbox_x: 0.5, bbox_y: 0.5, bbox_w: 0.2, bbox_h: 0.2, added_at: 0 },
        ];

        reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.0, 0.0, 0.2, 0.2), 1.0), det((0.5, 0.5, 0.2, 0.2), 2.0)],
            existing,
        )
        .await
        .unwrap();

        let constraint_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM constraints")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(constraint_count, 1, "must_link between two matched faces must survive by id");
    }

    #[tokio::test]
    async fn unmatched_existing_face_is_deleted_with_its_vector() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;
        let stale_face = crate::people::repo::insert_face(
            &pool, image_id, None, (0.9, 0.9, 0.05, 0.05), Some(0.5), Some(0.4), "buffalo_s_recognition",
        ).await.unwrap();
        crate::people::face_store::upsert_vector(&pool, stale_face, &emb(1.0)).await.unwrap();
        let existing = vec![Face { id: stale_face, image_id, subject_id: None, bbox_x: 0.9, bbox_y: 0.9, bbox_w: 0.05, bbox_h: 0.05, added_at: 0 }];

        // New detection is nowhere near the stale face's bbox -> no match.
        reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.0, 0.0, 0.2, 0.2), 1.0)],
            existing,
        )
        .await
        .unwrap();

        let face_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM faces WHERE id = ?")
            .bind(stale_face)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(face_count, 0, "unmatched existing face must be deleted");
        let vec_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM face_vectors WHERE rowid = ?")
            .bind(stale_face)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(vec_count, 0, "its face_vectors row must be deleted explicitly (no FK cascade on vec0)");
    }

    #[tokio::test]
    async fn retry_after_partial_insert_matches_instead_of_duplicating() {
        let pool = init_test_pool().await;
        let image_id = seed_image(&pool).await;

        // Simulate a prior partially-successful attempt: one detection already inserted.
        let already_inserted = crate::people::repo::insert_face(
            &pool, image_id, None, (0.1, 0.1, 0.2, 0.2), Some(0.9), Some(0.8), "antelopev2_recognition",
        ).await.unwrap();
        crate::people::face_store::upsert_vector(&pool, already_inserted, &emb(1.0)).await.unwrap();
        let existing = vec![Face { id: already_inserted, image_id, subject_id: None, bbox_x: 0.1, bbox_y: 0.1, bbox_w: 0.2, bbox_h: 0.2, added_at: 0 }];

        // Retry re-runs detection from scratch and finds the same face again.
        let touched = reprocess_image_faces(
            &pool,
            image_id,
            "antelopev2_recognition",
            vec![det((0.1, 0.1, 0.2, 0.2), 1.0)],
            existing,
        )
        .await
        .unwrap();

        assert_eq!(touched, vec![already_inserted], "retry must match the already-inserted row, not duplicate it");
        let face_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM faces")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(face_count, 1);
    }
}
