//! People persistence: subjects, faces, face-graph edges, merge suggestions.
use crate::library::repo::row_to_image;
use crate::models::Image;
use crate::models::{MergeSuggestion, SubjectDetail};
use crate::people::models::{Face, Subject};
use anyhow::Result;
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};

pub async fn insert_subject(
    pool: &SqlitePool,
    name: Option<&str>,
    subject_type: &str,
) -> Result<i64> {
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

pub async fn update_subject_thumbnail_face(
    pool: &SqlitePool,
    subject_id: i64,
    face_id: i64,
) -> Result<()> {
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

pub async fn get_subject_detail_with_counts(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<SubjectDetail>> {
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

    Ok(row.map(|r| SubjectDetail {
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
        r#"SELECT DISTINCT i.id, i.folder_id, i.path, i.file_hash, i.hash_status, i.file_size, i.date_taken, i.mtime, i.thumbnail_path, i.preview_path,
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

pub async fn list_faces_for_subject_with_images(
    pool: &SqlitePool,
    subject_id: i64,
) -> Result<Vec<crate::models::SubjectPhotoFace>> {
    let rows = sqlx::query(
        r#"SELECT f.id AS face_id, i.id AS image_id, i.path, i.thumbnail_path, i.preview_path,
                  i.date_taken, i.mtime,
                  f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h
           FROM faces f
           JOIN images i ON i.id = f.image_id
           WHERE f.subject_id = ? AND i.deleted_at IS NULL
           ORDER BY COALESCE(i.date_taken, i.mtime) DESC"#,
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| crate::models::SubjectPhotoFace {
            face_id: r.get("face_id"),
            image_id: r.get("image_id"),
            path: r.get("path"),
            thumbnail_path: r.get("thumbnail_path"),
            preview_path: r.get("preview_path"),
            date_taken: r.get("date_taken"),
            mtime: r.get("mtime"),
            face_bbox: crate::models::FaceBBox {
                x: r.get("bbox_x"),
                y: r.get("bbox_y"),
                w: r.get("bbox_w"),
                h: r.get("bbox_h"),
            },
        })
        .collect())
}

pub async fn get_largest_face_for_subject(
    pool: &SqlitePool,
    subject_id: i64,
) -> Result<Option<i64>> {
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

pub async fn update_face_subject(
    pool: &SqlitePool,
    face_id: i64,
    subject_id: Option<i64>,
) -> Result<()> {
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
    let rows = sqlx::query("SELECT s.id FROM subjects s WHERE s.thumbnail_face_id IS NULL")
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

pub async fn clear_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_merge_suggestions(
    pool: &SqlitePool,
    limit: Option<i64>,
) -> Result<Vec<MergeSuggestion>> {
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
        .map(|r| MergeSuggestion {
            id: r.get("id"),
            subject_a: Subject {
                id: r.get("sa_id"),
                name: r.get("sa_name"),
                thumbnail_face_id: r.get("sa_thumbnail_face_id"),
                subject_type: r.get("sa_type"),
                added_at: r.get("sa_added_at"),
            },
            subject_b: Subject {
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

    // Read all face ids up front so the transaction below spans only writes.
    let target_faces = get_face_ids_for_subject(pool, target_id).await?;
    let source_faces = get_face_ids_for_subject(pool, source_id).await?;

    // Wrap all mutations in a single transaction so the merge is atomic: if any step
    // fails, the dropped transaction rolls back, leaving no partially-merged subject.
    let mut tx = pool.begin().await?;

    // Rule: named subject's name always survives.
    // If target is unnamed and source is named, copy the source name to target.
    if target_name.is_none() && source_name.is_some() {
        sqlx::query("UPDATE subjects SET name = ? WHERE id = ? AND name IS NULL")
            .bind(&source_name)
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
    }

    // Write must_link between all faces of target and all faces of source (durable merge)
    let now_c = chrono::Utc::now().timestamp();
    for &tf in &target_faces {
        for &sf in &source_faces {
            let (a, b) = if tf < sf { (tf, sf) } else { (sf, tf) };
            sqlx::query(
                "INSERT OR IGNORE INTO constraints (face_a, face_b, kind, source, created_at) VALUES (?, ?, 'must_link', 'merge', ?)"
            ).bind(a).bind(b).bind(now_c).execute(&mut *tx).await?;
        }
    }

    sqlx::query("UPDATE faces SET subject_id = ? WHERE subject_id = ?")
        .bind(target_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT OR IGNORE INTO subject_tags (subject_id, tag_id, added_at) \
         SELECT ?, tag_id, added_at FROM subject_tags WHERE subject_id = ?",
    )
    .bind(target_id)
    .bind(source_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM merge_suggestions WHERE subject_id_a = ? OR subject_id_b = ? OR subject_id_a = ? OR subject_id_b = ?")
        .bind(target_id)
        .bind(target_id)
        .bind(source_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM subjects WHERE id = ?")
        .bind(source_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let _ = auto_assign_missing_thumbnails(pool).await;
    Ok(())
}

#[allow(dead_code)]
pub async fn get_dismissed_pair_set(pool: &SqlitePool) -> Result<HashSet<(i64, i64)>> {
    let rows = sqlx::query("SELECT subject_id_a, subject_id_b FROM dismissed_pairs")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let a = r.get::<i64, _>("subject_id_a");
            let b = r.get::<i64, _>("subject_id_b");
            if a < b {
                (a, b)
            } else {
                (b, a)
            }
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
        let (lo, hi) = if sid_a < sid_b {
            (sid_a, sid_b)
        } else {
            (sid_b, sid_a)
        };
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
        let rep_a: Option<i64> =
            sqlx::query_scalar("SELECT id FROM faces WHERE subject_id = ? LIMIT 1")
                .bind(lo)
                .fetch_optional(pool)
                .await?;
        let rep_b: Option<i64> =
            sqlx::query_scalar("SELECT id FROM faces WHERE subject_id = ? LIMIT 1")
                .bind(hi)
                .fetch_optional(pool)
                .await?;
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

pub async fn find_subject_by_name(
    pool: &SqlitePool,
    name: &str,
    exclude_id: i64,
) -> Result<Option<Subject>> {
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

pub async fn assign_face_to_subject(
    pool: &SqlitePool,
    face_id: i64,
    subject_id: i64,
) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = ? WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_subject_for_face(
    pool: &SqlitePool,
    face_id: i64,
    name: Option<&str>,
) -> Result<Subject> {
    let subject_id = insert_subject(pool, name, "person").await?;
    sqlx::query("UPDATE faces SET subject_id = ? WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    let row = sqlx::query(
        "SELECT id, name, thumbnail_face_id, type, added_at FROM subjects WHERE id = ?",
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

fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

pub async fn add_must_link(
    pool: &SqlitePool,
    face_a: i64,
    face_b: i64,
    source: &str,
) -> Result<()> {
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

pub async fn add_cannot_link(
    pool: &SqlitePool,
    face_a: i64,
    face_b: i64,
    source: &str,
) -> Result<()> {
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

pub async fn upsert_face_edge(
    pool: &SqlitePool,
    face_a: i64,
    face_b: i64,
    weight: f32,
) -> Result<()> {
    let (a, b) = if face_a < face_b {
        (face_a, face_b)
    } else {
        (face_b, face_a)
    };
    sqlx::query("INSERT OR REPLACE INTO face_edges (face_a, face_b, weight) VALUES (?, ?, ?)")
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

#[allow(dead_code)]
pub async fn get_all_similarity_edges(pool: &SqlitePool) -> Result<Vec<(i64, i64, f32)>> {
    let rows = sqlx::query("SELECT face_a, face_b, weight FROM face_edges")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get("face_a"), r.get("face_b"), r.get::<f32, _>("weight")))
        .collect())
}

pub async fn get_all_must_link_pairs(pool: &SqlitePool) -> Result<Vec<(i64, i64)>> {
    let rows = sqlx::query("SELECT face_a, face_b FROM constraints WHERE kind = 'must_link'")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get("face_a"), r.get("face_b")))
        .collect())
}

pub async fn get_all_cannot_link_pairs(pool: &SqlitePool) -> Result<HashSet<(i64, i64)>> {
    let rows = sqlx::query("SELECT face_a, face_b FROM constraints WHERE kind = 'cannot_link'")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let a: i64 = r.get("face_a");
            let b: i64 = r.get("face_b");
            if a < b {
                (a, b)
            } else {
                (b, a)
            }
        })
        .collect())
}

pub async fn get_assigned_face_subject_map(pool: &SqlitePool) -> Result<HashMap<i64, i64>> {
    let rows = sqlx::query("SELECT id, subject_id FROM faces WHERE subject_id IS NOT NULL")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<i64, _>("subject_id")))
        .collect())
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

pub async fn reset_all_subject_data(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM constraints")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM face_vectors")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM faces").execute(&mut *tx).await?;
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
