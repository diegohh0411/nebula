use anyhow::Result;
use log::{debug, info, warn};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::people::repo as people_repo;

pub const TAU_SIM: f32 = 0.45;
pub const K_NEAREST: usize = 5;
pub const MIN_COMPONENT_SIZE: usize = 2;

struct UnionFind {
    parent: HashMap<i64, i64>,
    rank: HashMap<i64, u32>,
}

impl UnionFind {
    fn new() -> Self {
        Self {
            parent: HashMap::new(),
            rank: HashMap::new(),
        }
    }

    fn add(&mut self, x: i64) {
        self.parent.entry(x).or_insert(x);
        self.rank.entry(x).or_insert(0);
    }

    fn find(&mut self, x: i64) -> i64 {
        self.add(x);
        if self.parent[&x] != x {
            let root = self.find(self.parent[&x]);
            self.parent.insert(x, root);
        }
        self.parent[&x]
    }

    fn union(&mut self, a: i64, b: i64) {
        self.add(a);
        self.add(b);
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        match self.rank[&ra].cmp(&self.rank[&rb]) {
            std::cmp::Ordering::Less => {
                self.parent.insert(ra, rb);
            }
            std::cmp::Ordering::Greater => {
                self.parent.insert(rb, ra);
            }
            std::cmp::Ordering::Equal => {
                self.parent.insert(rb, ra);
                *self.rank.entry(ra).or_insert(0) += 1;
            }
        }
    }

    #[allow(dead_code)]
    fn connected(&mut self, a: i64, b: i64) -> bool {
        self.find(a) == self.find(b)
    }

    fn components(&mut self, nodes: &[i64]) -> HashMap<i64, Vec<i64>> {
        let mut groups: HashMap<i64, Vec<i64>> = HashMap::new();
        for &node in nodes {
            let root = self.find(node);
            groups.entry(root).or_default().push(node);
        }
        groups
    }
}

fn compute_mutual_sim_edges(
    all_knn: &HashMap<i64, Vec<(i64, f32)>>,
    tau_sim: f32,
) -> Vec<(i64, i64, f32)> {
    let mut edges = Vec::new();
    for (&face_a, neighbors) in all_knn {
        for &(face_b, sim) in neighbors {
            if sim < tau_sim {
                continue;
            }
            if face_a >= face_b {
                continue;
            } // deduplicate: only emit with a < b
              // Check mutuality: face_a must appear in face_b's knn
            let is_mutual = all_knn
                .get(&face_b)
                .is_some_and(|nb| nb.iter().any(|(id, _)| *id == face_a));
            if is_mutual {
                edges.push((face_a, face_b, sim));
            }
        }
    }
    edges
}

/// Build the per-face neighbor map that feeds the similarity-edge graph.
///
/// A neighbor sharing the same (non-null) subject as the query face is dropped
/// *before* the top-k cut, so a dominant subject's own near-duplicate faces
/// cannot crowd a genuine cross-subject neighbor out of the list. Without this,
/// a person who appears in many photos saturates every top-k with their own
/// faces and the cross-subject "bridge" edge a merge suggestion depends on never
/// forms (TT-57). Unassigned faces (no subject) are never filtered, so
/// new-subject formation and assign-to-subject behavior are unchanged.
///
/// Cost note: when a single subject owns most of the library, `candidate_k`
/// approaches the face count and the per-face loop trends toward O(N²). That is
/// acceptable for a desktop photo library (N up to ~10k) and is dominated by the
/// sqlite-vec query cost.
async fn build_subject_aware_knn(
    pool: &SqlitePool,
    all_face_ids: &[i64],
    faces_to_query: &[i64],
    face_subjects: &HashMap<i64, i64>,
    k: usize,
    cancel: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<Option<HashMap<i64, Vec<(i64, f32)>>>> {
    // Subject sizes are counted from the *full* vectorized set so candidate_k is
    // correct even when we only query a subset (incremental pass).
    let mut subject_sizes: HashMap<i64, usize> = HashMap::new();
    for &fid in all_face_ids {
        if let Some(&sid) = face_subjects.get(&fid) {
            *subject_sizes.entry(sid).or_insert(0) += 1;
        }
    }

    let total = faces_to_query.len();
    let knn_start = Instant::now();
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    for (i, &fid) in faces_to_query.iter().enumerate() {
        if i > 0 && i % 250 == 0 {
            if let Some(c) = cancel {
                if c() {
                    debug!("[clustering] knn cancelled at {i}/{total} faces");
                    return Ok(None);
                }
            }
            debug!(
                "[clustering] knn progress {i}/{total} faces in {:.1}s",
                knn_start.elapsed().as_secs_f32()
            );
        }
        let own_subject = face_subjects.get(&fid).copied();
        let candidate_k = match own_subject {
            Some(sid) => k + subject_sizes.get(&sid).copied().unwrap_or(0),
            None => k,
        };
        let neighbors: Vec<(i64, f32)> =
            crate::people::face_store::knn_cosine_sim(pool, fid, candidate_k)
                .await?
                .into_iter()
                .filter(|(nid, _)| match own_subject {
                    Some(sid) => face_subjects.get(nid).copied() != Some(sid),
                    None => true,
                })
                .take(k)
                .collect();
        all_knn.insert(fid, neighbors);
    }
    Ok(Some(all_knn))
}

#[derive(Debug)]
enum LabelAction {
    AssignAll {
        faces: Vec<i64>,
        subject_id: i64,
    },
    NewSubject {
        faces: Vec<i64>,
    },
    Noise {
        faces: Vec<i64>,
    },
    SuggestMerge {
        #[allow(dead_code)]
        subject_ids: Vec<i64>,
    },
}

fn compute_label_actions(
    components: &HashMap<i64, Vec<i64>>,
    face_subjects: &HashMap<i64, i64>,
    subject_names: &HashMap<i64, Option<String>>,
    min_component_size: usize,
) -> Vec<LabelAction> {
    let mut actions = Vec::new();
    for faces in components.values() {
        let mut subject_to_faces: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut unlabeled: Vec<i64> = Vec::new();
        for &fid in faces {
            match face_subjects.get(&fid) {
                Some(&sid) => subject_to_faces.entry(sid).or_default().push(fid),
                None => unlabeled.push(fid),
            }
        }
        match subject_to_faces.len() {
            0 => {
                if faces.len() >= min_component_size {
                    actions.push(LabelAction::NewSubject {
                        faces: faces.clone(),
                    });
                } else {
                    actions.push(LabelAction::Noise {
                        faces: faces.clone(),
                    });
                }
            }
            1 => {
                let &sid = subject_to_faces.keys().next().unwrap();
                if !unlabeled.is_empty() {
                    actions.push(LabelAction::AssignAll {
                        faces: unlabeled,
                        subject_id: sid,
                    });
                }
            }
            _ => {
                let any_named = subject_to_faces
                    .keys()
                    .any(|sid| subject_names.get(sid).and_then(|n| n.as_ref()).is_some());
                if any_named {
                    actions.push(LabelAction::SuggestMerge {
                        subject_ids: subject_to_faces.keys().copied().collect(),
                    });
                }
                // Unlabeled faces in multi-subject component stay unassigned (precision-leaning)
            }
        }
    }
    actions
}

fn build_components_with_constraints(
    mut sim_edges: Vec<(i64, i64, f32)>,
    must_links: &[(i64, i64)],
    cannot_links: &HashSet<(i64, i64)>,
    all_faces: &[i64],
) -> UnionFind {
    let mut uf = UnionFind::new();
    for &f in all_faces {
        uf.add(f);
    }

    // Kruskal: strongest-first, skip any edge that would newly co-locate a cannot-linked pair
    sim_edges.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    for (fa, fb, _) in &sim_edges {
        let root_fa = uf.find(*fa);
        let root_fb = uf.find(*fb);
        if root_fa == root_fb {
            continue;
        }

        let would_violate = cannot_links.iter().any(|&(ca, cb)| {
            let root_ca = uf.find(ca);
            let root_cb = uf.find(cb);
            if root_ca == root_cb {
                return false;
            } // already co-located (pre-existing)
            let root_fa2 = uf.find(*fa);
            let root_fb2 = uf.find(*fb);
            (root_ca == root_fa2 || root_ca == root_fb2)
                && (root_cb == root_fa2 || root_cb == root_fb2)
        });

        if !would_violate {
            uf.union(*fa, *fb);
        }
    }

    // Must-link: always apply (flag contradiction but don't block)
    for &(fa, fb) in must_links {
        let ordered = if fa < fb { (fa, fb) } else { (fb, fa) };
        if cannot_links.contains(&ordered) {
            warn!(
                "[clustering] must_link/cannot_link contradiction for faces {} and {}",
                fa, fb
            );
        }
        uf.union(fa, fb);
    }

    uf
}

/// KNN-free back half of clustering: rebuild components from the *persisted*
/// `face_edges` graph + constraints, apply label actions, then cleanup,
/// thumbnails, and merge suggestions. In-memory union-find over all faces plus a
/// few writes — milliseconds even at ~14k faces.
pub async fn relabel_from_edges(pool: &SqlitePool) -> Result<ReclusterResult> {
    let started = Instant::now();
    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;
    let sim_edges = people_repo::get_all_similarity_edges(pool).await?;
    let must_links = people_repo::get_all_must_link_pairs(pool).await?;
    let cannot_links = people_repo::get_all_cannot_link_pairs(pool).await?;

    let mut uf =
        build_components_with_constraints(sim_edges, &must_links, &cannot_links, &all_face_ids);
    let components = uf.components(&all_face_ids);

    let subject_rows = sqlx::query("SELECT id, name FROM subjects")
        .fetch_all(pool)
        .await?;
    let subject_names: HashMap<i64, Option<String>> = subject_rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<Option<String>, _>("name")))
        .collect();

    let actions = compute_label_actions(
        &components,
        &face_subjects,
        &subject_names,
        MIN_COMPONENT_SIZE,
    );
    let mut new_clusters_count = 0usize;
    let mut noise_count = 0usize;
    for action in actions {
        match action {
            LabelAction::AssignAll { faces, subject_id } => {
                for fid in faces {
                    people_repo::update_face_subject(pool, fid, Some(subject_id)).await?;
                }
            }
            LabelAction::NewSubject { faces } => {
                let sid = people_repo::insert_subject(pool, None, "person").await?;
                for fid in &faces {
                    people_repo::update_face_subject(pool, *fid, Some(sid)).await?;
                }
                new_clusters_count += 1;
            }
            LabelAction::Noise { faces } => {
                for fid in &faces {
                    people_repo::update_face_subject(pool, *fid, None).await?;
                }
                noise_count += faces.len();
            }
            LabelAction::SuggestMerge { .. } => {}
        }
    }

    let deleted = people_repo::delete_subjects_with_no_faces(pool).await?;
    let _ = people_repo::auto_assign_missing_thumbnails(pool).await;
    let _ = find_merge_suggestions(pool).await;

    info!(
        "[clustering] relabel done in {:.1}s: {} new clusters, {} noise faces, {} subjects deleted",
        started.elapsed().as_secs_f32(),
        new_clusters_count,
        noise_count,
        deleted
    );

    Ok(ReclusterResult {
        clusters: new_clusters_count,
        noise: noise_count,
        merged: 0,
        deleted,
    })
}

/// Cheap per-batch edge update: compute mutual-kNN edges for the *new* faces and
/// their immediate neighbors only, and upsert them. Does NOT clear edges and does
/// NOT remove now-stale edges — the idle full sweep reconciles any drift.
///
/// The affected set `S = new_face_ids ∪ {candidate neighbors of each new face}`
/// is queried so both endpoints of every candidate new edge have a neighbor list,
/// which is what lets `compute_mutual_sim_edges` evaluate mutuality correctly.
#[allow(dead_code)]
pub async fn update_edges_incremental(pool: &SqlitePool, new_face_ids: &[i64]) -> Result<()> {
    if new_face_ids.is_empty() {
        return Ok(());
    }

    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;

    // Build the affected set S.
    let mut affected: HashSet<i64> = new_face_ids.iter().copied().collect();
    for &fid in new_face_ids {
        // Over-fetch by one: knn excludes the query face itself.
        let neighbors =
            crate::people::face_store::knn_cosine_sim(pool, fid, K_NEAREST + 1).await?;
        for (nid, _) in neighbors {
            affected.insert(nid);
        }
    }
    let faces_to_query: Vec<i64> = affected.into_iter().collect();

    // Subject-aware KNN over S only (full id list still drives subject_sizes).
    let local_knn = build_subject_aware_knn(
        pool,
        &all_face_ids,
        &faces_to_query,
        &face_subjects,
        K_NEAREST,
        None,
    )
    .await?
    .expect("build_subject_aware_knn returns Some when cancel is None");

    let edges = compute_mutual_sim_edges(&local_knn, TAU_SIM);
    for &(fa, fb, weight) in &edges {
        people_repo::upsert_face_edge(pool, fa, fb, weight).await?;
    }
    debug!(
        "[clustering] incremental: {} new faces, {} queried, {} edges upserted",
        new_face_ids.len(),
        faces_to_query.len(),
        edges.len()
    );
    Ok(())
}

/// Full authoritative sweep. Serves as the idle backstop. Runs the read-heavy
/// KNN first (so `face_edges` stays populated during the entire multi-minute
/// computation), checks `cancel` periodically, and only swaps the edge graph
/// once KNN completes uncancelled.
///
/// Returns `Ok(None)` if a `cancel()` check fired mid-KNN (new work entered the
/// queue); the caller should leave `clustering_dirty` set and retry later.
pub async fn cluster_unassigned_faces(
    pool: &SqlitePool,
    cancel: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<Option<ReclusterResult>> {
    let started = Instant::now();
    let all_face_ids = people_repo::get_all_face_ids_with_vectors(pool).await?;
    let face_subjects = people_repo::get_assigned_face_subject_map(pool).await?;
    info!(
        "[clustering] recluster start: {} vectorized faces, {} already assigned",
        all_face_ids.len(),
        face_subjects.len()
    );

    // KNN first — does NOT touch face_edges, so the table stays valid the whole time.
    let knn_started = Instant::now();
    let all_knn = match build_subject_aware_knn(
        pool,
        &all_face_ids,
        &all_face_ids,
        &face_subjects,
        K_NEAREST,
        cancel,
    )
    .await?
    {
        Some(map) => map,
        None => {
            info!("[clustering] full sweep cancelled — new work entered the queue");
            return Ok(None);
        }
    };
    debug!(
        "[clustering] knn graph built for {} faces in {:.1}s",
        all_face_ids.len(),
        knn_started.elapsed().as_secs_f32()
    );

    // Compute mutual edges and atomically swap the graph.
    let sim_edges = compute_mutual_sim_edges(&all_knn, TAU_SIM);
    people_repo::replace_all_face_edges(pool, &sim_edges).await?;

    // Back half reads the freshly-persisted edges.
    let result = relabel_from_edges(pool).await?;
    info!(
        "[clustering] recluster done in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    Ok(Some(result))
}

pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    people_repo::clear_merge_suggestions(pool).await?;

    let now = chrono::Utc::now().timestamp();

    // Cross-subject similarity edges: face_a and face_b belong to different subjects,
    // at least one subject is named, and the face pair is not cannot-linked.
    let rows = sqlx::query(
        r#"SELECT
               MIN(f1.subject_id, f2.subject_id) AS sid_a,
               MAX(f1.subject_id, f2.subject_id) AS sid_b,
               MAX(fe.weight)                    AS score
           FROM face_edges fe
           JOIN faces f1 ON f1.id = fe.face_a
           JOIN faces f2 ON f2.id = fe.face_b
           JOIN subjects s1 ON s1.id = f1.subject_id
           JOIN subjects s2 ON s2.id = f2.subject_id
           WHERE f1.subject_id IS NOT NULL
             AND f2.subject_id IS NOT NULL
             AND f1.subject_id != f2.subject_id
             AND (s1.name IS NOT NULL OR s2.name IS NOT NULL)
             AND NOT EXISTS (
                 SELECT 1 FROM constraints c
                 WHERE c.kind = 'cannot_link'
                   AND c.face_a = MIN(fe.face_a, fe.face_b)
                   AND c.face_b = MAX(fe.face_a, fe.face_b)
             )
           GROUP BY MIN(f1.subject_id, f2.subject_id), MAX(f1.subject_id, f2.subject_id)"#,
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        let sid_a: i64 = row.get("sid_a");
        let sid_b: i64 = row.get("sid_b");
        let score: f64 = row.get("score");

        // Skip dismissed pairs (still track via dismissed_pairs table for UI)
        let dismissed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dismissed_pairs
             WHERE (subject_id_a = ? AND subject_id_b = ?) OR (subject_id_a = ? AND subject_id_b = ?)"
        )
        .bind(sid_a).bind(sid_b).bind(sid_b).bind(sid_a)
        .fetch_one(pool).await?;
        if dismissed > 0 {
            continue;
        }

        sqlx::query(
            "INSERT OR IGNORE INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at)
             VALUES (?, ?, ?, ?)"
        )
        .bind(sid_a).bind(sid_b).bind(score).bind(now)
        .execute(pool).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_find_transitive_chain() {
        let mut uf = UnionFind::new();
        uf.union(1, 2);
        uf.union(2, 3);
        assert!(
            uf.connected(1, 3),
            "A-B + B-C edges must put A and C in same component"
        );
        let comps = uf.components(&[1, 2, 3]);
        assert_eq!(comps.len(), 1, "all three must be in one component");
    }

    #[test]
    fn union_find_independent_components_stay_separate() {
        let mut uf = UnionFind::new();
        uf.union(1, 2);
        uf.union(3, 4);
        assert!(!uf.connected(1, 3));
        let comps = uf.components(&[1, 2, 3, 4]);
        assert_eq!(comps.len(), 2);
    }

    #[test]
    fn union_find_components_groups_by_root() {
        let mut uf = UnionFind::new();
        uf.union(1, 2);
        uf.union(1, 3);
        let comps = uf.components(&[1, 2, 3, 4]);
        assert_eq!(comps.len(), 2, "component {{1,2,3}} and singleton {{4}}");
        let sizes: Vec<usize> = {
            let mut s: Vec<usize> = comps.values().map(|v| v.len()).collect();
            s.sort_unstable();
            s
        };
        assert_eq!(sizes, vec![1, 3]);
    }

    #[test]
    fn mutual_knn_both_directions_required() {
        // Face 1's knn contains face 2; face 2's knn does NOT contain face 1 → no edge
        let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
        all_knn.insert(1, vec![(2, 0.9), (3, 0.8)]);
        all_knn.insert(2, vec![(3, 0.85), (4, 0.7)]); // face 1 absent from face 2's list
        all_knn.insert(3, vec![(2, 0.85), (1, 0.8)]);
        all_knn.insert(4, vec![(2, 0.7), (3, 0.6)]);

        let edges = compute_mutual_sim_edges(&all_knn, 0.55);
        let has_1_2 = edges
            .iter()
            .any(|(a, b, _)| (*a == 1 && *b == 2) || (*a == 2 && *b == 1));
        assert!(
            !has_1_2,
            "1-2 must not be an edge: non-mutual (face 1 not in face 2's knn)"
        );
    }

    #[test]
    fn mutual_knn_creates_edge_when_both_in_each_others_topk() {
        let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
        all_knn.insert(1, vec![(2, 0.9)]);
        all_knn.insert(2, vec![(1, 0.9)]);

        let edges = compute_mutual_sim_edges(&all_knn, 0.55);
        assert_eq!(edges.len(), 1);
        assert!((edges[0].2 - 0.9).abs() < 1e-6);
    }

    #[test]
    fn mutual_knn_filters_below_tau_sim() {
        // Both in each other's top-k but similarity is below τ_sim
        let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
        all_knn.insert(1, vec![(2, 0.4)]); // 0.4 < 0.55
        all_knn.insert(2, vec![(1, 0.4)]);

        let edges = compute_mutual_sim_edges(&all_knn, 0.55);
        assert!(
            edges.is_empty(),
            "below-tau mutual pair must not create an edge"
        );
    }

    #[test]
    fn mutual_knn_no_duplicate_edges() {
        // Each side would generate (1,2) and (2,1); function must deduplicate
        let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
        all_knn.insert(1, vec![(2, 0.9)]);
        all_knn.insert(2, vec![(1, 0.9)]);

        let edges = compute_mutual_sim_edges(&all_knn, 0.55);
        assert_eq!(edges.len(), 1, "each pair must appear at most once");
    }

    #[test]
    fn must_link_joins_below_threshold() {
        // sim=0.3 < TAU_SIM — would be filtered out of sim_edges
        // but must_link forces co-location
        let sim_edges: Vec<(i64, i64, f32)> = vec![]; // no similarity edges pass tau
        let must_links = vec![(1i64, 2i64)];
        let cannot_links: HashSet<(i64, i64)> = HashSet::new();
        let mut uf =
            build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2, 3]);
        assert!(
            uf.connected(1, 2),
            "must_link must co-locate faces regardless of similarity"
        );
        assert!(
            !uf.connected(1, 3),
            "face 3 has no edges — must stay isolated"
        );
    }

    #[test]
    fn cannot_link_prevents_weakest_bridge_edge() {
        // Chain 1-2 (sim=0.9), 2-3 (sim=0.6); cannot-link (1,3)
        // Kruskal: add (1,2,0.9) → ok. Try (2,3,0.6) → would put 1 and 3 together → skip.
        let sim_edges = vec![(1i64, 2, 0.9_f32), (2, 3, 0.6)];
        let must_links: Vec<(i64, i64)> = vec![];
        let mut cannot_links: HashSet<(i64, i64)> = HashSet::new();
        cannot_links.insert((1i64, 3i64));
        let mut uf =
            build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2, 3]);
        assert!(uf.connected(1, 2));
        assert!(!uf.connected(2, 3));
        assert!(!uf.connected(1, 3));
    }

    #[test]
    fn cannot_link_between_independent_groups_not_violated() {
        // Group A: 1-2; Group B: 3-4; cannot-link (1,3); no cross edges → fine
        let sim_edges = vec![(1i64, 2, 0.9_f32), (3, 4, 0.85)];
        let mut cannot_links: HashSet<(i64, i64)> = HashSet::new();
        cannot_links.insert((1i64, 3i64));
        let mut uf =
            build_components_with_constraints(sim_edges, &[], &cannot_links, &[1, 2, 3, 4]);
        assert!(uf.connected(1, 2));
        assert!(uf.connected(3, 4));
        assert!(!uf.connected(1, 3));
    }

    #[test]
    fn must_link_always_overrides_even_with_cannot_link_warning() {
        // Direct contradiction: must_link AND cannot_link on same pair.
        // Per spec: flag (we eprintln), but must_link still wins.
        let sim_edges: Vec<(i64, i64, f32)> = vec![];
        let must_links = vec![(1i64, 2i64)];
        let mut cannot_links: HashSet<(i64, i64)> = HashSet::new();
        cannot_links.insert((1i64, 2i64));
        let mut uf =
            build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2]);
        // must_link is applied after sim-edge pass, so 1 and 2 end up connected
        assert!(
            uf.connected(1, 2),
            "must_link wins over cannot_link contradiction"
        );
    }

    #[test]
    fn label_assign_one_subject_fills_unlabeled_faces() {
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2, 3])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = [(1i64, 10i64)].into_iter().collect(); // face 1 → subject 10
        let subject_names: HashMap<i64, Option<String>> =
            [(10i64, Some("Alice".to_string()))].into_iter().collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        let assign = actions
            .iter()
            .find(|a| matches!(a, LabelAction::AssignAll { .. }));
        assert!(
            assign.is_some(),
            "should emit AssignAll for unlabeled faces 2 and 3"
        );
        if let Some(LabelAction::AssignAll { faces, subject_id }) = assign {
            assert!(faces.contains(&2) && faces.contains(&3));
            assert_eq!(*subject_id, 10);
        }
    }

    #[test]
    fn label_two_named_subjects_emits_suggestion_no_fuse() {
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2, 3])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = [(1i64, 10i64), (2i64, 20i64)].into_iter().collect();
        let subject_names: HashMap<i64, Option<String>> = [
            (10i64, Some("Alice".to_string())),
            (20i64, Some("Bob".to_string())),
        ]
        .into_iter()
        .collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, LabelAction::SuggestMerge { .. })),
            "two named subjects must emit a suggestion"
        );
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, LabelAction::AssignAll { .. })),
            "unlabeled face 3 must NOT be auto-assigned in a two-subject component"
        );
    }

    #[test]
    fn label_unlabeled_small_component_is_noise() {
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = HashMap::new();
        let subject_names: HashMap<i64, Option<String>> = HashMap::new();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, LabelAction::Noise { .. })),
            "size-1 unlabeled component must be noise (below MIN_COMPONENT_SIZE=2)"
        );
    }

    #[test]
    fn label_unlabeled_large_enough_component_gets_new_subject() {
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = HashMap::new();
        let subject_names: HashMap<i64, Option<String>> = HashMap::new();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, LabelAction::NewSubject { .. })),
            "size>=2 unlabeled component must trigger new subject creation"
        );
    }

    #[test]
    fn label_user_labeled_size_one_is_not_noise() {
        // A user-assigned face in a size-1 component must NOT be classified as noise.
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = [(1i64, 10i64)].into_iter().collect();
        let subject_names: HashMap<i64, Option<String>> =
            [(10i64, Some("Alice".to_string()))].into_iter().collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, LabelAction::Noise { faces } if faces.contains(&1))),
            "user-labeled face must not become noise even at size 1"
        );
    }

    #[test]
    fn label_suggestion_requires_at_least_one_named_subject() {
        // Two UNNAMED subjects in same component → no suggestion emitted (neither is named)
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = [(1i64, 10i64), (2i64, 20i64)].into_iter().collect();
        let subject_names: HashMap<i64, Option<String>> =
            [(10i64, None::<String>), (20i64, None::<String>)]
                .into_iter()
                .collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, LabelAction::SuggestMerge { .. })),
            "two unnamed subjects must not generate a merge suggestion"
        );
    }

    fn emb_bytes(vals: &[f32]) -> Vec<u8> {
        vals.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    async fn make_integration_pool() -> sqlx::SqlitePool {
        crate::db::ensure_sqlite_vec_registered();
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        for stmt in [
            "CREATE TABLE subjects (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, type TEXT NOT NULL DEFAULT 'person', thumbnail_face_id INTEGER, added_at INTEGER NOT NULL DEFAULT 0)",
            "CREATE TABLE faces (id INTEGER PRIMARY KEY AUTOINCREMENT, image_id INTEGER NOT NULL DEFAULT 0, subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL, bbox_x REAL NOT NULL DEFAULT 0, bbox_y REAL NOT NULL DEFAULT 0, bbox_w REAL NOT NULL DEFAULT 0.5, bbox_h REAL NOT NULL DEFAULT 0.5, added_at INTEGER NOT NULL DEFAULT 0)",
            "CREATE VIRTUAL TABLE face_vectors USING vec0(embedding float[3])",
            "CREATE TABLE constraints (face_a INTEGER NOT NULL, face_b INTEGER NOT NULL, kind TEXT NOT NULL, source TEXT NOT NULL, created_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (face_a, face_b, kind))",
            "CREATE TABLE face_edges (face_a INTEGER NOT NULL, face_b INTEGER NOT NULL, weight REAL NOT NULL, PRIMARY KEY (face_a, face_b))",
            "CREATE TABLE merge_suggestions (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_id_a INTEGER NOT NULL, subject_id_b INTEGER NOT NULL, score REAL NOT NULL, created_at INTEGER NOT NULL DEFAULT 0)",
            "CREATE UNIQUE INDEX idx_merge_pair ON merge_suggestions(CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END, CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END)",
            "CREATE TABLE dismissed_pairs (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_id_a INTEGER NOT NULL, subject_id_b INTEGER NOT NULL, dismissed_at INTEGER NOT NULL DEFAULT 0)",
            "CREATE UNIQUE INDEX idx_dismissed_pair ON dismissed_pairs(subject_id_a, subject_id_b)",
        ] {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn integration_remove_face_then_recluster_not_reassigned() {
        let pool = make_integration_pool().await;

        let subject_s: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('S', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let anchor_s: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(subject_s)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(anchor_s)
            .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let subject_s2: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('S2', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let anchor_s2: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id",
        )
        .bind(subject_s2)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(anchor_s2)
            .bind(emb_bytes(&[0.998f32, 0.063, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let face_f: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (3, NULL, 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(face_f)
            .bind(emb_bytes(&[0.999f32, 0.045, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        crate::people::repo::add_cannot_link(&pool, face_f, anchor_s, "removal")
            .await
            .unwrap();

        cluster_unassigned_faces(&pool, None).await.unwrap();

        let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(face_f)
            .fetch_one(&pool)
            .await
            .unwrap();

        assert!(
            assigned != Some(subject_s),
            "face_f must NOT be reassigned to forbidden subject S (got {:?})",
            assigned
        );
    }

    #[tokio::test]
    async fn integration_merge_distant_groups_stays_merged_after_recluster() {
        let pool = make_integration_pool().await;

        let subject_a: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('A', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let fa1: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(subject_a)
        .fetch_one(&pool)
        .await
        .unwrap();
        let fa2: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id",
        )
        .bind(subject_a)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa1)
            .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa2)
            .bind(emb_bytes(&[0.99f32, 0.14, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let subject_b: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('B', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let fb1: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (3, ?, 0) RETURNING id",
        )
        .bind(subject_b)
        .fetch_one(&pool)
        .await
        .unwrap();
        let fb2: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (4, ?, 0) RETURNING id",
        )
        .bind(subject_b)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb1)
            .bind(emb_bytes(&[0.0f32, 1.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb2)
            .bind(emb_bytes(&[0.14f32, 0.99, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        crate::people::repo::add_must_link(&pool, fa1, fb1, "merge")
            .await
            .unwrap();
        crate::people::repo::add_must_link(&pool, fa1, fb2, "merge")
            .await
            .unwrap();
        crate::people::repo::add_must_link(&pool, fa2, fb1, "merge")
            .await
            .unwrap();
        crate::people::repo::add_must_link(&pool, fa2, fb2, "merge")
            .await
            .unwrap();
        sqlx::query("UPDATE faces SET subject_id = ? WHERE subject_id = ?")
            .bind(subject_a)
            .bind(subject_b)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM subjects WHERE id = ?")
            .bind(subject_b)
            .execute(&pool)
            .await
            .unwrap();

        cluster_unassigned_faces(&pool, None).await.unwrap();

        let subjects: Vec<Option<i64>> =
            sqlx::query_scalar("SELECT subject_id FROM faces ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        let distinct: HashSet<Option<i64>> = subjects.into_iter().collect();
        assert_eq!(
            distinct.len(),
            1,
            "all four faces must share one subject after recluster (must_link is durable)"
        );
        assert!(
            distinct.iter().next().unwrap().is_some(),
            "subject must not be NULL"
        );
    }

    #[tokio::test]
    async fn graph_suggestions_emitted_for_cross_subject_edges() {
        let pool = make_integration_pool().await;

        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let bob: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let fa: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa)
            .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let fb: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id",
        )
        .bind(bob)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb)
            .bind(emb_bytes(&[0.99f32, 0.14, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        // Manually insert a face_edge between them (as recluster would)
        people_repo::upsert_face_edge(&pool, fa, fb, 0.99)
            .await
            .unwrap();

        find_merge_suggestions(&pool).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "one suggestion expected for Alice-Bob cross edge");
    }

    #[tokio::test]
    async fn graph_suggestions_skipped_for_cannot_link_pair() {
        let pool = make_integration_pool().await;

        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let bob: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let fa: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa)
            .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        let fb: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id",
        )
        .bind(bob)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb)
            .bind(emb_bytes(&[0.99f32, 0.14, 0.0]))
            .execute(&pool)
            .await
            .unwrap();

        people_repo::upsert_face_edge(&pool, fa, fb, 0.99)
            .await
            .unwrap();
        crate::people::repo::add_cannot_link(&pool, fa, fb, "dismiss")
            .await
            .unwrap();

        find_merge_suggestions(&pool).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "dismissed pair (cannot_link) must not be suggested"
        );
    }

    fn unit(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    /// Group all vectorized faces by subject, returning sorted groups of face ids.
    /// Subject *identity* is ignored — only the partition structure is compared,
    /// which is the right equivalence for from-scratch unassigned imports.
    async fn subject_partition(pool: &sqlx::SqlitePool) -> Vec<Vec<i64>> {
        let rows: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT id, subject_id FROM faces ORDER BY id")
                .fetch_all(pool)
                .await
                .unwrap();
        let mut groups: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut singletons: Vec<Vec<i64>> = Vec::new();
        for (fid, sid) in rows {
            match sid {
                Some(s) => groups.entry(s).or_default().push(fid),
                None => singletons.push(vec![fid]),
            }
        }
        let mut out: Vec<Vec<i64>> = groups.into_values().collect();
        out.extend(singletons);
        for g in &mut out {
            g.sort_unstable();
        }
        out.sort();
        out
    }

    async fn insert_face_with_vector(
        pool: &sqlx::SqlitePool,
        subject_id: Option<i64>,
        v: &[f32],
    ) -> i64 {
        let fid: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(subject_id)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fid)
            .bind(emb_bytes(&unit(v)))
            .execute(pool)
            .await
            .unwrap();
        fid
    }

    #[tokio::test]
    async fn update_edges_incremental_links_new_face_into_existing_cluster() {
        let pool = make_integration_pool().await;
        let alex: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alex', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // Two assigned Alex faces already vectorized.
        let _a1 = insert_face_with_vector(&pool, Some(alex), &[1.0, 0.0, 0.0]).await;
        let _a2 = insert_face_with_vector(&pool, Some(alex), &[1.0, 0.02, 0.0]).await;
        // A new, unassigned face inside the cluster.
        let new_face = insert_face_with_vector(&pool, None, &[1.0, 0.01, 0.0]).await;

        update_edges_incremental(&pool, &[new_face]).await.unwrap();

        // An edge between the new face and an Alex face must have been upserted.
        let edge_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM face_edges WHERE face_a = ? OR face_b = ?",
        )
        .bind(new_face)
        .bind(new_face)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(edge_count >= 1, "new face must gain at least one mutual edge");

        // And relabel must then assign it to Alex.
        relabel_from_edges(&pool).await.unwrap();
        let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(new_face)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(assigned, Some(alex));
    }

    #[tokio::test]
    async fn incremental_then_idle_converges_to_full_sweep() {
        // Two well-separated clusters: {A1,A2} near x-axis, {B1,B2} near y-axis.
        let va1 = [1.0f32, 0.0, 0.0];
        let va2 = [0.99f32, 0.14, 0.0];
        let vb1 = [0.0f32, 1.0, 0.0];
        let vb2 = [0.14f32, 0.99, 0.0];

        // Pool 1: incremental in two batches, then a final full sweep.
        let inc = make_integration_pool().await;
        let f1 = insert_face_with_vector(&inc, None, &va1).await;
        let f2 = insert_face_with_vector(&inc, None, &va2).await;
        update_edges_incremental(&inc, &[f1, f2]).await.unwrap();
        relabel_from_edges(&inc).await.unwrap();
        let f3 = insert_face_with_vector(&inc, None, &vb1).await;
        let f4 = insert_face_with_vector(&inc, None, &vb2).await;
        update_edges_incremental(&inc, &[f3, f4]).await.unwrap();
        relabel_from_edges(&inc).await.unwrap();
        cluster_unassigned_faces(&inc, None).await.unwrap();
        let inc_partition = subject_partition(&inc).await;

        // Pool 2: single full sweep over all four faces.
        let full = make_integration_pool().await;
        insert_face_with_vector(&full, None, &va1).await;
        insert_face_with_vector(&full, None, &va2).await;
        insert_face_with_vector(&full, None, &vb1).await;
        insert_face_with_vector(&full, None, &vb2).await;
        cluster_unassigned_faces(&full, None).await.unwrap();
        let full_partition = subject_partition(&full).await;

        assert_eq!(
            inc_partition, full_partition,
            "idle backstop must reconcile incremental state to match a single full sweep"
        );
        // Sanity: the two clusters are distinct.
        assert_eq!(full_partition.len(), 2, "expected two subjects");
    }

    #[tokio::test]
    async fn crowded_subject_still_yields_cross_subject_merge_suggestion() {
        // Regression for TT-57. A named subject that owns more than K_NEAREST
        // near-duplicate faces must not crowd a genuine cross-subject neighbor
        // out of the mutual-kNN graph. Before the subject-aware neighbor filter,
        // every top-K_NEAREST list for an "Alex" face was saturated with other
        // Alex faces, so the bridge edge to the unnamed duplicate subject never
        // formed and no merge suggestion was produced — exactly the bug the user
        // hit after processing 200 photos.
        let pool = make_integration_pool().await;

        let alex: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alex', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // Six tightly-clustered Alex faces (> K_NEAREST = 5). Every pair is more
        // similar to each other (cos ~0.999) than any is to the duplicate below
        // (cos ~0.99), so the duplicate lands just outside each face's top-5.
        let alex_vectors = [
            unit(&[1.0, 0.00, 0.00]),
            unit(&[1.0, 0.02, 0.00]),
            unit(&[1.0, 0.04, 0.00]),
            unit(&[1.0, 0.00, 0.02]),
            unit(&[1.0, 0.00, 0.04]),
            unit(&[1.0, 0.02, 0.02]),
        ];
        for v in &alex_vectors {
            let fid: i64 = sqlx::query_scalar(
                "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
            )
            .bind(alex)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
                .bind(fid)
                .bind(emb_bytes(v))
                .execute(&pool)
                .await
                .unwrap();
        }

        // A second, *unnamed* subject that is clearly the same person (cos ~0.99
        // to every Alex face, well above TAU_SIM) but sits outside each Alex
        // face's top-5 because the six Alex faces are nearer to one another.
        let dup: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let dup_face: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id",
        )
        .bind(dup)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(dup_face)
            .bind(emb_bytes(&unit(&[1.0, 0.1, 0.1])))
            .execute(&pool)
            .await
            .unwrap();

        cluster_unassigned_faces(&pool, None).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1,
            "named Alex subject must be suggested for merge with its unnamed duplicate despite top-k crowding");
    }

    #[tokio::test]
    async fn unassigned_face_still_assigned_to_crowded_subject() {
        // Guards the `None => true` pass-through in build_subject_aware_knn: the
        // subject filter must NOT apply to unassigned faces. An unlabeled face
        // sitting inside a dominant subject's tight cluster must still be
        // assigned to that subject — if the filter wrongly dropped these edges,
        // AssignAll would silently stop firing and faces would pile up as noise.
        let pool = make_integration_pool().await;

        let alex: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alex', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // Six tightly-clustered assigned Alex faces (> K_NEAREST = 5).
        let alex_vectors = [
            unit(&[1.0, 0.00, 0.00]),
            unit(&[1.0, 0.02, 0.00]),
            unit(&[1.0, 0.04, 0.00]),
            unit(&[1.0, 0.00, 0.02]),
            unit(&[1.0, 0.00, 0.04]),
            unit(&[1.0, 0.02, 0.02]),
        ];
        for v in &alex_vectors {
            let fid: i64 = sqlx::query_scalar(
                "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
            )
            .bind(alex)
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
                .bind(fid)
                .bind(emb_bytes(v))
                .execute(&pool)
                .await
                .unwrap();
        }

        // A brand-new *unassigned* face sitting inside the Alex cluster.
        let orphan: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, NULL, 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(orphan)
            .bind(emb_bytes(&unit(&[1.0, 0.03, 0.01])))
            .execute(&pool)
            .await
            .unwrap();

        cluster_unassigned_faces(&pool, None).await.unwrap();

        let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(orphan)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            assigned,
            Some(alex),
            "unassigned face inside a crowded subject's cluster must be assigned to that subject"
        );
    }

    #[tokio::test]
    async fn relabel_from_edges_assigns_unlabeled_in_single_subject_component() {
        let pool = make_integration_pool().await;

        // One named subject with an assigned anchor face.
        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let anchor: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id",
        )
        .bind(alice)
        .fetch_one(&pool)
        .await
        .unwrap();
        // An unlabeled face.
        let orphan: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, NULL, 0) RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // Vectors so both appear in get_all_face_ids_with_vectors.
        for fid in [anchor, orphan] {
            sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
                .bind(fid)
                .bind(emb_bytes(&[1.0f32, 0.0, 0.0]))
                .execute(&pool)
                .await
                .unwrap();
        }
        // Seed the edge directly — relabel must consume persisted edges, no KNN.
        people_repo::upsert_face_edge(&pool, anchor, orphan, 0.9)
            .await
            .unwrap();

        let result = relabel_from_edges(&pool).await.unwrap();
        assert_eq!(result.noise, 0);

        let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(orphan)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            assigned,
            Some(alice),
            "orphan in a single-subject component must be assigned to that subject"
        );
    }
}
