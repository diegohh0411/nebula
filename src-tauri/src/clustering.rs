use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};

use crate::db;
use crate::embedder::cosine_similarity;

#[derive(Debug, serde::Serialize)]
pub struct ReclusterResult {
    pub clusters: usize,
    pub noise: usize,
    pub merged: i64,
    pub deleted: u64,
}

/// Minimum cosine similarity to create an edge between two faces.
const EDGE_THRESHOLD: f32 = 0.55;

/// Number of nearest neighbors to consider per face when building the graph.
const K_NEAREST: usize = 5;

/// Minimum component size to form a new subject.
const MIN_COMPONENT_SIZE: usize = 2;

/// Minimum cosine similarity for a representative face to match an existing subject.
const SUBJECT_MATCH_THRESHOLD: f32 = 0.55;

/// Minimum cosine similarity between subject centroids to generate a merge suggestion.
const MERGE_CENTROID_SIMILARITY_THRESHOLD: f32 = 0.65;

/// Union-Find (Disjoint Set Union) with path compression and union by rank.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return;
        }
        if self.rank[rx] < self.rank[ry] {
            self.parent[rx] = ry;
        } else if self.rank[rx] > self.rank[ry] {
            self.parent[ry] = rx;
        } else {
            self.parent[ry] = rx;
            self.rank[rx] += 1;
        }
    }

    /// Returns connected components as {root_index: [member_indices]}.
    fn components(&mut self) -> HashMap<usize, Vec<usize>> {
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..self.parent.len() {
            let root = self.find(i);
            groups.entry(root).or_default().push(i);
        }
        groups
    }
}

/// Build k-NN graph edges and merge connected pairs into the Union-Find.
fn build_knn_graph(
    faces: &[(usize, Vec<f32>)],
    k: usize,
    threshold: f32,
    uf: &mut UnionFind,
) {
    let n = faces.len();
    if n == 0 {
        return;
    }

    for i in 0..n {
        let mut sims: Vec<(usize, f32)> = Vec::with_capacity(n - 1);
        for j in 0..n {
            if i == j {
                continue;
            }
            let sim = cosine_similarity(&faces[i].1, &faces[j].1);
            if sim > threshold {
                sims.push((j, sim));
            }
        }
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (j, _sim) in sims.into_iter().take(k) {
            uf.union(i, j);
        }
    }
}

/// Inject must-link constraint edges into the Union-Find.
fn inject_must_link_edges(pairs: &[(usize, usize)], uf: &mut UnionFind) {
    for &(i, j) in pairs {
        uf.union(i, j);
    }
}

/// Result of deciding what to do with a connected component.
#[derive(Debug, PartialEq)]
enum ComponentAssignment {
    /// All unassigned faces in this component should be assigned to this existing subject.
    AssignExisting(i64),
    /// This component represents a new subject — create one.
    CreateNew,
}

/// Decide what to do with a connected component based on its manual-face composition.
fn assign_component(
    face_subjects: &[Option<i64>],
    is_manual: &[bool],
    _manual_by_subject: &HashMap<i64, Vec<i64>>,
) -> Option<ComponentAssignment> {
    let mut manual_subjects: HashSet<i64> = HashSet::new();
    for (i, &manual) in is_manual.iter().enumerate() {
        if manual {
            if let Some(sid) = face_subjects[i] {
                manual_subjects.insert(sid);
            }
        }
    }

    match manual_subjects.len() {
        0 => {
            let component_size: usize = face_subjects.iter().filter(|s| s.is_none()).count();
            if component_size < MIN_COMPONENT_SIZE {
                None
            } else {
                Some(ComponentAssignment::CreateNew)
            }
        }
        1 => {
            Some(ComponentAssignment::AssignExisting(
                *manual_subjects.iter().next().unwrap(),
            ))
        }
        _ => {
            eprintln!(
                "[clustering] Conflict: component contains manual faces from {} subjects ({:?}). Faces stay unassigned.",
                manual_subjects.len(),
                manual_subjects,
            );
            None
        }
    }
}

/// Build an adjacency list from k-NN graph edges (for degree computation).
fn build_adjacency(
    faces: &[(usize, Vec<f32>)],
    k: usize,
    threshold: f32,
) -> Vec<HashSet<usize>> {
    let n = faces.len();
    let mut adj: Vec<HashSet<usize>> = vec![HashSet::new(); n];

    for i in 0..n {
        let mut sims: Vec<(usize, f32)> = Vec::with_capacity(n - 1);
        for j in 0..n {
            if i == j {
                continue;
            }
            let sim = cosine_similarity(&faces[i].1, &faces[j].1);
            if sim > threshold {
                sims.push((j, sim));
            }
        }
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (j, _sim) in sims.into_iter().take(k) {
            adj[i].insert(j);
            adj[j].insert(i);
        }
    }

    adj
}

/// Find the face with the highest degree in a component (most edges = natural hub).
fn find_highest_degree_node(component_indices: &[usize], adj: &[HashSet<usize>]) -> usize {
    *component_indices
        .iter()
        .max_by_key(|&&idx| adj[idx].len())
        .unwrap()
}

/// Find the best matching subject for a representative embedding.
fn find_best_subject_match(
    rep_emb: &[f32],
    subject_centroids: &HashMap<i64, Vec<f32>>,
    threshold: f32,
) -> Option<i64> {
    subject_centroids
        .iter()
        .map(|(&sid, centroid)| (sid, cosine_similarity(rep_emb, centroid)))
        .filter(|(_, sim)| *sim > threshold)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(sid, _)| sid)
}

pub async fn cluster_unassigned_faces(pool: &SqlitePool) -> Result<ReclusterResult> {
    // 1. Load ALL faces with embeddings (assigned + unassigned)
    let all_faces_raw = db::get_all_faces_with_embeddings(pool).await?;

    // Decode embeddings
    let mut all_faces: Vec<(i64, Option<i64>, Vec<f32>, bool)> = Vec::new();
    for (face_id, subject_id, emb_blob, is_manual) in all_faces_raw {
        match crate::embedder::bytes_to_f32_vec(&emb_blob) {
            Ok(emb) => all_faces.push((face_id, subject_id, emb, is_manual)),
            Err(_) => eprintln!("[clustering] Failed to decode embedding for face {}", face_id),
        }
    }

    if all_faces.is_empty() {
        return Ok(ReclusterResult {
            clusters: 0,
            noise: 0,
            merged: 0,
            deleted: 0,
        });
    }

    let n = all_faces.len();

    // 2. Load manual face groups by subject for must-link and conflict detection
    let manual_groups_raw = db::get_manual_faces_by_subject(pool).await?;
    let _manual_face_to_subject: HashMap<i64, i64> = manual_groups_raw
        .iter()
        .flat_map(|(sid, face_ids)| face_ids.iter().map(|&fid| (fid, *sid)))
        .collect();
    let manual_by_subject: HashMap<i64, Vec<i64>> = manual_groups_raw
        .into_iter()
        .map(|(sid, fids)| (sid, fids))
        .collect();

    // Build index: face_id -> local index
    let face_id_to_idx: HashMap<i64, usize> = all_faces
        .iter()
        .enumerate()
        .map(|(idx, (fid, _, _, _))| (*fid, idx))
        .collect();

    // 3. Build graph faces list for k-NN
    let graph_faces: Vec<(usize, Vec<f32>)> = all_faces
        .iter()
        .enumerate()
        .map(|(idx, (_, _, emb, _))| (idx, emb.clone()))
        .collect();

    // 4. Build Union-Find and k-NN graph
    let mut uf = UnionFind::new(n);
    let effective_k = K_NEAREST.min(n.saturating_sub(1));
    build_knn_graph(&graph_faces, effective_k, EDGE_THRESHOLD, &mut uf);

    // 5. Inject must-link edges
    let mut must_link_pairs: Vec<(usize, usize)> = Vec::new();

    // 5a. is_manual faces within the same subject are must-linked
    for (_subject_id, manual_face_ids) in &manual_by_subject {
        let sampled: Vec<i64> = if manual_face_ids.len() > 50 {
            let step = manual_face_ids.len() as f64 / 50.0;
            (0..50)
                .map(|i| manual_face_ids[(i as f64 * step) as usize])
                .collect()
        } else {
            manual_face_ids.clone()
        };

        for i in 0..sampled.len() {
            for j in (i + 1)..sampled.len() {
                if let (Some(&idx_i), Some(&idx_j)) =
                    (face_id_to_idx.get(&sampled[i]), face_id_to_idx.get(&sampled[j]))
                {
                    must_link_pairs.push((idx_i, idx_j));
                }
            }
        }
    }

    inject_must_link_edges(&must_link_pairs, &mut uf);

    // 6. Compute connected components
    let components = uf.components();

    // 7. Compute subject centroids for matching new components to existing subjects
    let mut subject_emb_groups: HashMap<i64, Vec<&Vec<f32>>> = HashMap::new();
    for (_, subject_id, emb, _) in &all_faces {
        if let Some(sid) = subject_id {
            subject_emb_groups.entry(*sid).or_default().push(emb);
        }
    }
    let subject_centroids: HashMap<i64, Vec<f32>> = subject_emb_groups
        .iter()
        .map(|(&sid, embs)| {
            if embs.is_empty() {
                return (sid, vec![]);
            }
            let dim = embs[0].len();
            let mut centroid = vec![0.0f32; dim];
            for emb in embs {
                for (i, &v) in emb.iter().enumerate() {
                    centroid[i] += v;
                }
            }
            let count = embs.len() as f32;
            for v in &mut centroid {
                *v /= count;
            }
            (sid, centroid)
        })
        .filter(|(_, c)| !c.is_empty())
        .collect();

    // Build adjacency for degree computation
    let adj = build_adjacency(&graph_faces, effective_k, EDGE_THRESHOLD);

    // 8. Process each component
    let mut new_clusters_count = 0;
    let mut noise_count = 0;

    for (_root, member_indices) in &components {
        let comp_face_subjects: Vec<Option<i64>> = member_indices
            .iter()
            .map(|&idx| all_faces[idx].1)
            .collect();
        let comp_is_manual: Vec<bool> = member_indices
            .iter()
            .map(|&idx| all_faces[idx].3)
            .collect();

        let assignment = assign_component(&comp_face_subjects, &comp_is_manual, &manual_by_subject);

        match assignment {
            Some(ComponentAssignment::AssignExisting(subject_id)) => {
                for &idx in member_indices {
                    if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                        db::update_face_subject(pool, all_faces[idx].0, Some(subject_id)).await?;
                    }
                }
            }
            Some(ComponentAssignment::CreateNew) => {
                let hub_idx = find_highest_degree_node(member_indices, &adj);
                let rep_emb = &all_faces[hub_idx].2;

                let subject_match = find_best_subject_match(rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);

                if let Some(existing_sid) = subject_match {
                    for &idx in member_indices {
                        if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                            db::update_face_subject(pool, all_faces[idx].0, Some(existing_sid))
                                .await?;
                        }
                    }
                } else {
                    new_clusters_count += 1;
                    let new_sid = db::insert_subject(pool, None, "person").await?;
                    for &idx in member_indices {
                        if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                            db::update_face_subject(pool, all_faces[idx].0, Some(new_sid)).await?;
                        }
                    }
                }
            }
            None => {
                for &idx in member_indices {
                    if all_faces[idx].1.is_none() && !all_faces[idx].3 {
                        noise_count += 1;
                    }
                }
            }
        }
    }

    // 9. Post-processing
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

pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    crate::db::clear_merge_suggestions(pool).await?;

    // Load all faces grouped by subject and compute centroids
    let all_raw = db::get_subject_embeddings(pool).await?;
    let mut groups: HashMap<i64, Vec<Vec<f32>>> = HashMap::new();
    for (sid, blob) in all_raw {
        if let Ok(emb) = crate::embedder::bytes_to_f32_vec(&blob) {
            groups.entry(sid).or_default().push(emb);
        }
    }

    let centroids: HashMap<i64, Vec<f32>> = groups
        .into_iter()
        .filter_map(|(sid, embs)| {
            if embs.is_empty() {
                return None;
            }
            let dim = embs[0].len();
            let mut centroid = vec![0.0f32; dim];
            for emb in &embs {
                for (i, &v) in emb.iter().enumerate() {
                    centroid[i] += v;
                }
            }
            let n = embs.len() as f32;
            for v in &mut centroid {
                *v /= n;
            }
            Some((sid, centroid))
        })
        .collect();

    let mut subject_list: Vec<(i64, Vec<f32>)> = centroids.into_iter().collect();
    subject_list.sort_unstable_by_key(|(id, _)| *id);

    for i in 0..subject_list.len() {
        for j in (i + 1)..subject_list.len() {
            let (id_a, emb_a) = &subject_list[i];
            let (id_b, emb_b) = &subject_list[j];
            let sim = cosine_similarity(emb_a, emb_b);
            if sim > MERGE_CENTROID_SIMILARITY_THRESHOLD {
                crate::db::insert_merge_suggestion(pool, *id_a, *id_b, sim as f64).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emb(vals: &[f32]) -> Vec<f32> {
        vals.to_vec()
    }

    // === UnionFind tests ===

    #[test]
    fn union_find_single_element() {
        let mut uf = UnionFind::new(3);
        assert_eq!(uf.find(0), 0);
        assert_eq!(uf.find(1), 1);
        assert_eq!(uf.find(2), 2);
    }

    #[test]
    fn union_find_merges_sets() {
        let mut uf = UnionFind::new(4);
        uf.union(0, 1);
        uf.union(1, 2);
        assert_eq!(uf.find(0), uf.find(2), "0 and 2 should be in the same set");
        assert_ne!(uf.find(0), uf.find(3), "3 should be in a different set");
    }

    #[test]
    fn union_find_components_group_correctly() {
        let mut uf = UnionFind::new(5);
        uf.union(0, 1);
        uf.union(1, 2);
        uf.union(3, 4);

        let components = uf.components();
        assert_eq!(components.len(), 2);
        let mut sets: Vec<Vec<usize>> = components.into_values().collect();
        sets.sort_by_key(|s| s[0]);
        assert_eq!(sets[0], vec![0, 1, 2]);
        assert_eq!(sets[1], vec![3, 4]);
    }

    // === Graph construction tests ===

    #[test]
    fn build_knn_graph_transitive_chain() {
        let faces: Vec<(usize, Vec<f32>)> = vec![
            (0, emb(&[1.0, 0.0])),
            (1, emb(&[0.9, 0.1])),
            (2, emb(&[0.8, 0.2])),
        ];

        let mut uf = UnionFind::new(3);
        build_knn_graph(&faces, 2, 0.55, &mut uf);

        let components = uf.components();
        assert_eq!(components.len(), 1, "all 3 faces should form one component");
    }

    #[test]
    fn build_knn_graph_isolated_nodes_stay_separate() {
        let faces: Vec<(usize, Vec<f32>)> = vec![
            (0, emb(&[1.0, 0.0])),
            (1, emb(&[0.0, 1.0])),
            (2, emb(&[-1.0, 0.0])),
        ];

        let mut uf = UnionFind::new(3);
        build_knn_graph(&faces, 2, 0.55, &mut uf);

        let components = uf.components();
        assert_eq!(components.len(), 3, "all faces should be isolated");
    }

    // === Must-link injection tests ===

    #[test]
    fn inject_must_link_edges_bypasses_threshold() {
        let mut uf = UnionFind::new(2);
        inject_must_link_edges(&[(0, 1)], &mut uf);
        assert_eq!(uf.find(0), uf.find(1), "must-link faces should be in same component");
    }

    #[test]
    fn inject_must_link_transitive_closure() {
        let mut uf = UnionFind::new(4);
        inject_must_link_edges(&[(0, 1), (2, 3), (1, 2)], &mut uf);
        assert_eq!(uf.find(0), uf.find(3), "transitive must-link should connect all");
    }

    // === Component assignment tests ===

    #[test]
    fn assign_component_single_manual_subject() {
        let face_subjects: Vec<Option<i64>> = vec![Some(10), Some(10), None, None];
        let is_manual: Vec<bool> = vec![true, false, false, false];
        let manual_by_subject: HashMap<i64, Vec<i64>> = {
            let mut m = HashMap::new();
            m.insert(10, vec![0]);
            m
        };

        let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
        assert_eq!(result, Some(ComponentAssignment::AssignExisting(10)));
    }

    #[test]
    fn assign_component_conflict_multiple_manual_subjects() {
        let face_subjects: Vec<Option<i64>> = vec![Some(10), Some(20), None];
        let is_manual: Vec<bool> = vec![true, true, false];
        let manual_by_subject: HashMap<i64, Vec<i64>> = {
            let mut m = HashMap::new();
            m.insert(10, vec![0]);
            m.insert(20, vec![1]);
            m
        };

        let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
        assert_eq!(result, None, "conflict should return None (noise)");
    }

    #[test]
    fn assign_component_no_manual_new_subject() {
        let face_subjects: Vec<Option<i64>> = vec![None, None, None];
        let is_manual: Vec<bool> = vec![false, false, false];
        let manual_by_subject: HashMap<i64, Vec<i64>> = HashMap::new();

        let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
        assert_eq!(result, Some(ComponentAssignment::CreateNew));
    }

    #[test]
    fn assign_component_no_manual_too_small() {
        let face_subjects: Vec<Option<i64>> = vec![None];
        let is_manual: Vec<bool> = vec![false];
        let manual_by_subject: HashMap<i64, Vec<i64>> = HashMap::new();

        let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
        assert_eq!(result, None, "single unassigned face should be noise");
    }

    // === Adjacency and degree tests ===

    #[test]
    fn find_highest_degree_node_picks_hub() {
        let faces: Vec<(usize, Vec<f32>)> = vec![
            (0, emb(&[1.0, 0.0])),  // hub: closest to 1 and 2
            (1, emb(&[0.9, 0.1])),  // closest to 0
            (2, emb(&[0.9, -0.1])), // closest to 0
            (3, emb(&[0.0, 1.0])),
            (4, emb(&[0.0, -1.0])),
        ];

        let mut uf = UnionFind::new(5);
        uf.union(0, 1);
        uf.union(0, 2);

        // k=1 so each node picks only its single nearest neighbor
        let adj = build_adjacency(&faces, 1, EDGE_THRESHOLD);
        let component_indices = vec![0, 1, 2];

        let hub = find_highest_degree_node(&component_indices, &adj);
        assert_eq!(hub, 0, "node 0 should be the hub (degree {})", adj[0].len());
    }

    // === Subject matching tests ===

    #[test]
    fn find_best_subject_match_above_threshold() {
        let rep_emb = emb(&[1.0, 0.0]);
        let subject_centroids: HashMap<i64, Vec<f32>> = {
            let mut m = HashMap::new();
            m.insert(10, emb(&[0.95, 0.05]));
            m.insert(20, emb(&[0.0, 1.0]));
            m
        };

        let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
        assert_eq!(result, Some(10));
    }

    #[test]
    fn find_best_subject_match_below_threshold() {
        let rep_emb = emb(&[1.0, 0.0]);
        let subject_centroids: HashMap<i64, Vec<f32>> = {
            let mut m = HashMap::new();
            m.insert(20, emb(&[0.0, 1.0]));
            m
        };

        let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
        assert_eq!(result, None, "should not match when similarity is below threshold");
    }

    #[test]
    fn transitive_chain_a_b_c() {
        // A-B edge, B-C edge, no A-C edge. All 3 in same component.
        let faces: Vec<(usize, Vec<f32>)> = vec![
            (0, emb(&[1.0, 0.0, 0.0])),
            (1, emb(&[0.95, 0.05, 0.0])),  // near face 0
            (2, emb(&[0.90, 0.10, 0.0])),  // near face 1, not near face 0
        ];

        let mut uf = UnionFind::new(3);
        build_knn_graph(&faces, 2, 0.55, &mut uf);

        let components = uf.components();
        assert_eq!(components.len(), 1, "transitive chain should produce one component");

        let mut members: Vec<usize> = components.values().flatten().copied().collect();
        members.sort();
        assert_eq!(members, vec![0, 1, 2]);
    }

    #[test]
    fn must_link_edge_below_threshold() {
        // Two faces with cosine similarity ~0.0 but must-linked
        let mut uf = UnionFind::new(2);
        inject_must_link_edges(&[(0, 1)], &mut uf);
        assert_eq!(uf.find(0), uf.find(1));
    }

    #[test]
    fn cannot_link_conflict_prevents_assignment() {
        // Component has manual faces from subject 10 and subject 20
        let face_subjects: Vec<Option<i64>> = vec![Some(10), Some(20), None];
        let is_manual: Vec<bool> = vec![true, true, false];
        let manual_by_subject: HashMap<i64, Vec<i64>> = {
            let mut m = HashMap::new();
            m.insert(10, vec![0]);
            m.insert(20, vec![1]);
            m
        };

        let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
        assert_eq!(result, None, "conflict component should stay unassigned");
    }

    #[test]
    fn single_node_noise() {
        let face_subjects: Vec<Option<i64>> = vec![None];
        let is_manual: Vec<bool> = vec![false];
        let manual_by_subject: HashMap<i64, Vec<i64>> = HashMap::new();

        let result = assign_component(&face_subjects, &is_manual, &manual_by_subject);
        assert_eq!(result, None, "single unassigned face should be noise");
    }

    #[test]
    fn subject_match_via_representative() {
        // Component of 4 unassigned faces near subject 10's centroid
        let rep_emb = emb(&[0.95, 0.05, 0.0]);
        let subject_centroids: HashMap<i64, Vec<f32>> = {
            let mut m = HashMap::new();
            m.insert(10, emb(&[1.0, 0.0, 0.0]));
            m.insert(20, emb(&[0.0, 1.0, 0.0]));
            m
        };

        let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
        assert_eq!(result, Some(10), "should match subject 10 via representative");
    }

    #[test]
    fn new_subject_creation_no_near_subject() {
        // Component of 3 unassigned faces with no nearby subject
        let rep_emb = emb(&[0.0, 0.0, 1.0]); // orthogonal to all subjects
        let subject_centroids: HashMap<i64, Vec<f32>> = {
            let mut m = HashMap::new();
            m.insert(10, emb(&[1.0, 0.0, 0.0]));
            m
        };

        let result = find_best_subject_match(&rep_emb, &subject_centroids, SUBJECT_MATCH_THRESHOLD);
        assert_eq!(result, None, "should not match any subject");
    }
}
