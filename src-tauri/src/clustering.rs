use anyhow::Result;
use hdbscan::{Hdbscan, HdbscanHyperParams};
use sqlx::SqlitePool;
use std::collections::HashMap;

use crate::db;

pub async fn recluster_all(pool: &SqlitePool) -> Result<ReclusterResult> {
    let faces = db::get_all_faces_with_embeddings(pool).await?;

    if faces.is_empty() {
        return Ok(ReclusterResult {
            clusters: 0,
            noise: 0,
            merged: 0,
            deleted: 0,
        });
    }

    let face_ids: Vec<i64> = faces.iter().map(|(id, _, _, _)| *id).collect();
    let old_subject_ids: Vec<Option<i64>> = faces.iter().map(|(_, sid, _, _)| *sid).collect();
    let is_manual_flags: Vec<bool> = faces.iter().map(|(_, _, _, m)| *m).collect();

    let embeddings: Vec<Vec<f32>> = faces
        .iter()
        .filter_map(|(_, _, emb_blob, _)| crate::embedder::bytes_to_f32_vec(emb_blob).ok())
        .collect();

    if embeddings.len() != face_ids.len() {
        anyhow::bail!(
            "Embedding decode mismatch: {} faces but {} decoded embeddings",
            face_ids.len(),
            embeddings.len()
        );
    }

    let hyper_params = HdbscanHyperParams::builder()
        .min_cluster_size(2)
        .min_samples(2)
        .build();

    let clusterer = Hdbscan::new(&embeddings, hyper_params);
    let labels = clusterer.cluster().map_err(|e| anyhow::anyhow!("HDBSCAN failed: {}", e))?;

    let mut cluster_to_face_indices: HashMap<i32, Vec<usize>> = HashMap::new();
    for (idx, &label) in labels.iter().enumerate() {
        if is_manual_flags[idx] {
            continue;
        }
        cluster_to_face_indices.entry(label).or_default().push(idx);
    }

    let mut subjects_merged = 0i64;

    for (&label, face_indices) in &cluster_to_face_indices {
        if label < 0 {
            continue;
        }

        let existing_subject_ids: Vec<Option<i64>> = face_indices
            .iter()
            .map(|&idx| old_subject_ids[idx])
            .collect();

        let non_none: Vec<i64> = existing_subject_ids.iter().filter_map(|&s| s).collect();

        let chosen_subject_id = if !non_none.is_empty() {
            let mut counts: HashMap<i64, usize> = HashMap::new();
            for &sid in &non_none {
                *counts.entry(sid).or_default() += 1;
            }
            let best = counts.into_iter().max_by_key(|(_, c)| *c).map(|(s, _)| s).unwrap();
            subjects_merged += non_none.iter().filter(|&&s| s != best).count() as i64;
            best
        } else {
            db::insert_subject(pool, None, "person").await?
        };

        for &idx in face_indices {
            db::update_face_subject(pool, face_ids[idx], Some(chosen_subject_id)).await?;
        }
    }

    let noise_count = cluster_to_face_indices.get(&-1).map(|v| v.len()).unwrap_or(0);
    if let Some(noise_indices) = cluster_to_face_indices.get(&-1) {
        for &idx in noise_indices {
            db::update_face_subject(pool, face_ids[idx], None).await?;
        }
    }

    let deleted = db::delete_subjects_with_no_faces(pool).await?;

    let _ = db::auto_assign_missing_thumbnails(pool).await;

    let _ = find_merge_suggestions(pool).await;

    Ok(ReclusterResult {
        clusters: cluster_to_face_indices.keys().filter(|&&l| l >= 0).count(),
        noise: noise_count,
        merged: subjects_merged,
        deleted,
    })
}

const MERGE_SIMILARITY_THRESHOLD: f32 = 0.35;
const MERGE_MIN_CROSS_MATCHES: i64 = 2;
const MERGE_MIN_CROSS_RATIO: f32 = 0.20;

pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    // TODO(perf): Throttle this to run at most once every 12-24 hours rather than
    // after every recluster batch. For now it runs every time since the dataset is
    // small, but as face count grows this O(n*m) per subject pair will get expensive.
    // Consider a `last_merge_scan_at` timestamp in the DB or a dedicated periodic task.

    let subjects = crate::db::list_all_subjects(pool).await?;

    let subject_embeddings: Vec<(i64, Vec<Vec<f32>>)> = {
        let mut result = Vec::new();
        for subject in &subjects {
            let faces = crate::db::get_faces_by_subject(pool, subject.id).await?;
            let embeddings: Vec<Vec<f32>> = faces
                .into_iter()
                .filter_map(|(_, blob)| crate::embedder::bytes_to_f32_vec(&blob).ok())
                .collect();
            if !embeddings.is_empty() {
                result.push((subject.id, embeddings));
            }
        }
        result
    };

    crate::db::clear_merge_suggestions(pool).await?;

    for i in 0..subject_embeddings.len() {
        for j in (i + 1)..subject_embeddings.len() {
            let (_, emb_a) = &subject_embeddings[i];
            let (id_b, emb_b) = &subject_embeddings[j];

            let total_pairs = (emb_a.len() * emb_b.len()) as i64;
            let mut cross_match_count: i64 = 0;

            for a_face in emb_a.iter() {
                for b_face in emb_b.iter() {
                    let sim = crate::embedder::cosine_similarity(a_face, b_face);
                    if sim > MERGE_SIMILARITY_THRESHOLD {
                        cross_match_count += 1;
                    }
                }
            }

            let ratio = if total_pairs > 0 {
                cross_match_count as f32 / total_pairs as f32
            } else {
                0.0
            };

            if cross_match_count >= MERGE_MIN_CROSS_MATCHES && ratio >= MERGE_MIN_CROSS_RATIO {
                crate::db::insert_merge_suggestion(
                    pool,
                    subject_embeddings[i].0,
                    *id_b,
                    cross_match_count,
                    total_pairs,
                )
                .await?;
            }
        }
    }

    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct ReclusterResult {
    pub clusters: usize,
    pub noise: usize,
    pub merged: i64,
    pub deleted: u64,
}
