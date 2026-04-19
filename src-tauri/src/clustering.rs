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

    // Build anchor centroids from manual corrections + all-face fallback.
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

    let mut subjects_merged = 0i64;

    for (&label, face_indices) in &cluster_to_face_indices {
        if label < 0 {
            continue;
        }

        let cluster_centroid = {
            let vecs: Vec<&Vec<f32>> = face_indices.iter().map(|&i| &embeddings[i]).collect();
            let dim = vecs[0].len();
            let mut c = vec![0.0f32; dim];
            for v in &vecs {
                for (i, &x) in v.iter().enumerate() {
                    c[i] += x;
                }
            }
            let n = vecs.len() as f32;
            c.iter_mut().for_each(|v| *v /= n);
            c
        };

        let chosen_subject_id =
            match find_nearest_anchor(&cluster_centroid, &anchor_centroids, ANCHOR_MATCH_THRESHOLD) {
                Some(sid) => sid,
                None => db::insert_subject(pool, None, "person").await?,
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
}
