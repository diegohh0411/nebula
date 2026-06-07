use anyhow::Result;
use hdbscan::{Hdbscan, HdbscanHyperParams};
use sqlx::SqlitePool;
use std::collections::HashMap;

use crate::db;

pub async fn cluster_unassigned_faces(pool: &SqlitePool) -> Result<ReclusterResult> {
    // 1. Build anchor centroids first.
    let manual_raw = db::get_manual_face_embeddings_by_subject(pool).await?;
    let manual_decoded: Vec<(i64, Vec<f32>)> = manual_raw
        .into_iter()
        .filter_map(|(sid, blob)| {
            crate::embedder::bytes_to_f32_vec(&blob).ok().map(|e| (sid, e))
        })
        .collect();

    let all_raw = db::get_subject_embeddings(pool).await?;
    let all_decoded: Vec<(i64, Vec<f32>)> = all_raw
        .into_iter()
        .filter_map(|(sid, blob)| {
            crate::embedder::bytes_to_f32_vec(&blob).ok().map(|e| (sid, e))
        })
        .collect();

    let anchor_centroids = compute_anchor_centroids(&manual_decoded, &all_decoded);

    // 2. Fetch ONLY unassigned faces
    let unassigned = db::get_unassigned_faces_with_embeddings(pool).await?;

    let mut residual_faces = Vec::new();
    let mut new_clusters_count = 0;
    let mut noise_count = 0;

    // 3. Pass 1: Greedy Centroid Match
    for (face_id, emb_blob) in unassigned {
        if let Ok(emb) = crate::embedder::bytes_to_f32_vec(&emb_blob) {
            if let Some(sid) = find_nearest_anchor(&emb, &anchor_centroids, ANCHOR_MATCH_THRESHOLD) {
                // Match found, assign immediately
                db::update_face_subject(pool, face_id, Some(sid)).await?;
            } else {
                // No match, keep for HDBSCAN
                residual_faces.push((face_id, emb));
            }
        } else {
            eprintln!("[clustering] Failed to decode embedding for face {}", face_id);
            noise_count += 1;
        }
    }

    // 4. Pass 2: Residual HDBSCAN
    if residual_faces.len() >= 2 {
        let (residual_ids, embeddings): (Vec<i64>, Vec<Vec<f32>>) = residual_faces.into_iter().unzip();
        let hyper_params = HdbscanHyperParams::builder()
            .min_cluster_size(2)
            .min_samples(2)
            .build();

        let clusterer = Hdbscan::new(&embeddings, hyper_params);
        match clusterer.cluster() {
            Ok(labels) => {
                let mut cluster_to_face_indices: HashMap<i32, Vec<usize>> = HashMap::new();
                for (idx, &label) in labels.iter().enumerate() {
                    cluster_to_face_indices.entry(label).or_default().push(idx);
                }

                for (&label, indices) in &cluster_to_face_indices {
                    if label < 0 {
                        noise_count += indices.len();
                        continue;
                    }
                    new_clusters_count += 1;
                    let new_subject_id = db::insert_subject(pool, None, "person").await?;
                    for &idx in indices {
                        let face_id = residual_ids[idx];
                        db::update_face_subject(pool, face_id, Some(new_subject_id)).await?;
                    }
                }
            }
            Err(e) => {
                eprintln!("[clustering] HDBSCAN failed on residual faces: {}", e);
                noise_count += embeddings.len();
            }
        }
    } else {
        noise_count += residual_faces.len();
    }

    let deleted = db::delete_subjects_with_no_faces(pool).await?;

    let _ = db::auto_assign_missing_thumbnails(pool).await;

    let _ = find_merge_suggestions(pool).await;

    Ok(ReclusterResult {
        clusters: new_clusters_count,
        noise: noise_count,
        merged: 0,
        deleted,
    })
}

const MERGE_CENTROID_SIMILARITY_THRESHOLD: f32 = 0.65;

pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    crate::db::clear_merge_suggestions(pool).await?;

    let named_flags = crate::db::get_subject_named_flags(pool).await?;

    let manual_raw = db::get_manual_face_embeddings_by_subject(pool).await?;
    let manual_decoded: Vec<(i64, Vec<f32>)> = manual_raw
        .into_iter()
        .filter_map(|(sid, blob)| crate::embedder::bytes_to_f32_vec(&blob).ok().map(|e| (sid, e)))
        .collect();

    let all_raw = db::get_subject_embeddings(pool).await?;
    let all_decoded: Vec<(i64, Vec<f32>)> = all_raw
        .into_iter()
        .filter_map(|(sid, blob)| crate::embedder::bytes_to_f32_vec(&blob).ok().map(|e| (sid, e)))
        .collect();

    let anchor_centroids = compute_anchor_centroids(&manual_decoded, &all_decoded);

    let mut subject_embeddings: Vec<(i64, Vec<f32>)> = anchor_centroids.into_iter().collect();
    subject_embeddings.sort_unstable_by_key(|(id, _)| *id);

    for i in 0..subject_embeddings.len() {
        for j in (i + 1)..subject_embeddings.len() {
            let (id_a, emb_a) = &subject_embeddings[i];
            let (id_b, emb_b) = &subject_embeddings[j];

            let a_named = named_flags.get(id_a).copied().unwrap_or(false);
            let b_named = named_flags.get(id_b).copied().unwrap_or(false);
            if !a_named && !b_named {
                continue;
            }

            let sim = crate::embedder::cosine_similarity(emb_a, emb_b);
            if sim > MERGE_CENTROID_SIMILARITY_THRESHOLD {
                crate::db::insert_merge_suggestion(pool, *id_a, *id_b, sim as f64).await?;
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

const ANCHOR_MATCH_THRESHOLD: f32 = 0.75;

fn compute_anchor_centroids(
    manual: &[(i64, Vec<f32>)],
    all: &[(i64, Vec<f32>)],
) -> HashMap<i64, Vec<f32>> {
    let mut by_manual: HashMap<i64, Vec<&Vec<f32>>> = HashMap::new();
    for (id, emb) in manual {
        by_manual.entry(*id).or_default().push(emb);
    }

    let mut by_all: HashMap<i64, Vec<&Vec<f32>>> = HashMap::new();
    for (id, emb) in all {
        by_all.entry(*id).or_default().push(emb);
    }

    let mut subject_ids: std::collections::HashSet<i64> = by_manual.keys().copied().collect();
    subject_ids.extend(by_all.keys().copied());

    subject_ids
        .into_iter()
        .filter_map(|id| {
            let faces = by_manual
                .get(&id)
                .map(|v| v.as_slice())
                .or_else(|| by_all.get(&id).map(|v| v.as_slice()))?;
            if faces.is_empty() {
                return None;
            }
            let dim = faces[0].len();
            let mut centroid = vec![0.0f32; dim];
            for emb in faces {
                for (i, &v) in emb.iter().enumerate() {
                    centroid[i] += v;
                }
            }
            let n = faces.len() as f32;
            for v in &mut centroid {
                *v /= n;
            }
            Some((id, centroid))
        })
        .collect()
}

fn find_nearest_anchor(
    cluster_centroid: &[f32],
    anchors: &HashMap<i64, Vec<f32>>,
    threshold: f32,
) -> Option<i64> {
    anchors
        .iter()
        .map(|(&id, emb)| (id, crate::embedder::cosine_similarity(cluster_centroid, emb)))
        .filter(|(_, sim)| *sim > threshold)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emb(vals: &[f32]) -> Vec<f32> {
        vals.to_vec()
    }

    #[test]
    fn anchor_centroid_is_mean_of_manual_faces() {
        // Subject 1 has two manual faces; centroid should be their mean.
        let manual = vec![
            (1i64, emb(&[1.0, 0.0])),
            (1i64, emb(&[0.0, 1.0])),
        ];
        let all: Vec<(i64, Vec<f32>)> = vec![];

        let centroids = compute_anchor_centroids(&manual, &all);

        let c = centroids.get(&1).expect("subject 1 should have a centroid");
        assert!((c[0] - 0.5).abs() < 1e-6);
        assert!((c[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn anchor_centroid_falls_back_to_all_faces_when_no_manual() {
        // Subject 2 has no manual faces — should fall back to all-faces mean.
        let manual: Vec<(i64, Vec<f32>)> = vec![];
        let all = vec![
            (2i64, emb(&[0.0, 1.0])),
            (2i64, emb(&[1.0, 0.0])),
        ];

        let centroids = compute_anchor_centroids(&manual, &all);

        let c = centroids.get(&2).expect("subject 2 should have a centroid");
        assert!((c[0] - 0.5).abs() < 1e-6);
        assert!((c[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn manual_faces_take_priority_over_all_faces() {
        // Subject 3 has one manual face at [1,0] and one non-manual at [0,1].
        // Centroid should be [1,0] (manual only), not [0.5, 0.5].
        let manual = vec![(3i64, emb(&[1.0, 0.0]))];
        let all = vec![
            (3i64, emb(&[1.0, 0.0])),
            (3i64, emb(&[0.0, 1.0])),
        ];

        let centroids = compute_anchor_centroids(&manual, &all);

        let c = centroids.get(&3).expect("subject 3 should have a centroid");
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert!((c[1] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn nearest_anchor_assigns_cluster_to_matching_subject() {
        // Anchor for subject 10 is [1,0,0], cluster centroid is close to it.
        let mut anchors = HashMap::new();
        anchors.insert(10i64, emb(&[1.0, 0.0, 0.0]));

        let cluster_centroid = emb(&[0.9, 0.1, 0.0]);
        let result = find_nearest_anchor(&cluster_centroid, &anchors, ANCHOR_MATCH_THRESHOLD);

        assert_eq!(result, Some(10));
    }

    #[test]
    fn nearest_anchor_returns_none_when_below_threshold() {
        // Cluster centroid is orthogonal to anchor — should not match.
        let mut anchors = HashMap::new();
        anchors.insert(10i64, emb(&[1.0, 0.0, 0.0]));

        let cluster_centroid = emb(&[0.0, 1.0, 0.0]);
        let result = find_nearest_anchor(&cluster_centroid, &anchors, ANCHOR_MATCH_THRESHOLD);

        assert_eq!(result, None);
    }

    #[test]
    fn anchor_guided_assignment_prefers_anchor_over_majority() {
        let mut anchors = HashMap::new();
        anchors.insert(1i64, emb(&[1.0, 0.0, 0.0]));

        // Cluster A centroid: near subject 1 anchor
        let a = find_nearest_anchor(&emb(&[0.95, 0.05, 0.0]), &anchors, ANCHOR_MATCH_THRESHOLD);
        assert_eq!(a, Some(1), "cluster A should match subject 1");

        // Cluster B centroid: orthogonal — no match
        let b = find_nearest_anchor(&emb(&[0.0, 1.0, 0.0]), &anchors, ANCHOR_MATCH_THRESHOLD);
        assert_eq!(b, None, "cluster B should get no match (creates new subject)");
    }

    #[test]
    fn unnamed_unnamed_pair_is_skipped() {
        let mut named_flags = std::collections::HashMap::new();
        named_flags.insert(1i64, true);   // Alice — named
        named_flags.insert(2i64, false);  // unnamed
        named_flags.insert(3i64, false);  // unnamed

        let is_unnamed_pair = |a: i64, b: i64| -> bool {
            !named_flags.get(&a).copied().unwrap_or(false)
                && !named_flags.get(&b).copied().unwrap_or(false)
        };

        assert!(!is_unnamed_pair(1, 2), "named+unnamed should not be skipped");
        assert!(!is_unnamed_pair(1, 1), "named+named should not be skipped");
        assert!(is_unnamed_pair(2, 3),  "unnamed+unnamed must be skipped");
    }
}
