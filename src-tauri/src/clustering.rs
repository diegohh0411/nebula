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

    let face_ids: Vec<i64> = faces.iter().map(|(id, _, _)| *id).collect();
    let old_subject_ids: Vec<Option<i64>> = faces.iter().map(|(_, sid, _)| *sid).collect();

    let embeddings: Vec<Vec<f32>> = faces
        .iter()
        .filter_map(|(_, _, emb_blob)| crate::embedder::bytes_to_f32_vec(emb_blob).ok())
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

    Ok(ReclusterResult {
        clusters: cluster_to_face_indices.keys().filter(|&&l| l >= 0).count(),
        noise: noise_count,
        merged: subjects_merged,
        deleted,
    })
}

#[derive(Debug, serde::Serialize)]
pub struct ReclusterResult {
    pub clusters: usize,
    pub noise: usize,
    pub merged: i64,
    pub deleted: u64,
}
