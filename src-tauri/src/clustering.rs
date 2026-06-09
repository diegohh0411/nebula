use anyhow::Result;
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};

use crate::db;

pub const TAU_SIM: f32 = 0.55;
pub const K_NEAREST: usize = 5;
pub const MIN_COMPONENT_SIZE: usize = 2;

struct UnionFind {
    parent: HashMap<i64, i64>,
    rank: HashMap<i64, u32>,
}

impl UnionFind {
    fn new() -> Self {
        Self { parent: HashMap::new(), rank: HashMap::new() }
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
        if ra == rb { return; }
        match self.rank[&ra].cmp(&self.rank[&rb]) {
            std::cmp::Ordering::Less    => { self.parent.insert(ra, rb); }
            std::cmp::Ordering::Greater => { self.parent.insert(rb, ra); }
            std::cmp::Ordering::Equal   => {
                self.parent.insert(rb, ra);
                *self.rank.entry(ra).or_insert(0) += 1;
            }
        }
    }

    fn connected(&mut self, a: i64, b: i64) -> bool {
        self.add(a); self.add(b);
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
            if sim < tau_sim { continue; }
            if face_a >= face_b { continue; }  // deduplicate: only emit with a < b
            // Check mutuality: face_a must appear in face_b's knn
            let is_mutual = all_knn
                .get(&face_b)
                .map_or(false, |nb| nb.iter().any(|(id, _)| *id == face_a));
            if is_mutual {
                edges.push((face_a, face_b, sim));
            }
        }
    }
    edges
}

#[derive(Debug)]
enum LabelAction {
    AssignAll { faces: Vec<i64>, subject_id: i64 },
    NewSubject { faces: Vec<i64> },
    Noise { faces: Vec<i64> },
    SuggestMerge { subject_ids: Vec<i64> },
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
                    actions.push(LabelAction::NewSubject { faces: faces.clone() });
                } else {
                    actions.push(LabelAction::Noise { faces: faces.clone() });
                }
            }
            1 => {
                let &sid = subject_to_faces.keys().next().unwrap();
                if !unlabeled.is_empty() {
                    actions.push(LabelAction::AssignAll { faces: unlabeled, subject_id: sid });
                }
            }
            _ => {
                let any_named = subject_to_faces.keys().any(|sid| {
                    subject_names.get(sid).and_then(|n| n.as_ref()).is_some()
                });
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
    for &f in all_faces { uf.add(f); }

    // Kruskal: strongest-first, skip any edge that would newly co-locate a cannot-linked pair
    sim_edges.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    for (fa, fb, _) in &sim_edges {
        let root_fa = uf.find(*fa);
        let root_fb = uf.find(*fb);
        if root_fa == root_fb { continue; }

        let would_violate = cannot_links.iter().any(|&(ca, cb)| {
            let root_ca = uf.find(ca);
            let root_cb = uf.find(cb);
            if root_ca == root_cb { return false; }  // already co-located (pre-existing)
            let root_fa2 = uf.find(*fa);
            let root_fb2 = uf.find(*fb);
            (root_ca == root_fa2 || root_ca == root_fb2) &&
            (root_cb == root_fa2 || root_cb == root_fb2)
        });

        if !would_violate {
            uf.union(*fa, *fb);
        }
    }

    // Must-link: always apply (flag contradiction but don't block)
    for &(fa, fb) in must_links {
        let ordered = if fa < fb { (fa, fb) } else { (fb, fa) };
        if cannot_links.contains(&ordered) {
            eprintln!("[clustering] WARNING: must_link/cannot_link contradiction for faces {} and {}", fa, fb);
        }
        uf.union(fa, fb);
    }

    uf
}

pub async fn cluster_unassigned_faces(pool: &SqlitePool) -> Result<ReclusterResult> {
    // 1. Rebuild similarity edge graph
    db::clear_all_face_edges(pool).await?;
    let all_face_ids = db::get_all_face_ids_with_vectors(pool).await?;

    // Build knn map: face_id → Vec<(neighbor_id, cosine_sim)>
    let mut all_knn: HashMap<i64, Vec<(i64, f32)>> = HashMap::new();
    for &fid in &all_face_ids {
        let neighbors = crate::face_store::knn_cosine_sim(pool, fid, K_NEAREST).await?;
        all_knn.insert(fid, neighbors);
    }

    // Compute mutual sim edges and persist
    let sim_edges = compute_mutual_sim_edges(&all_knn, TAU_SIM);
    for &(fa, fb, weight) in &sim_edges {
        db::upsert_face_edge(pool, fa, fb, weight).await?;
    }

    // 2. Load constraints
    let must_links = db::get_all_must_link_pairs(pool).await?;
    let cannot_links = db::get_all_cannot_link_pairs(pool).await?;

    // 3. Build Union-Find with constraint enforcement
    let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &all_face_ids);

    // 4. Compute components and load current assignments
    let components = uf.components(&all_face_ids);
    let face_subjects = db::get_assigned_face_subject_map(pool).await?;
    let subject_rows = sqlx::query("SELECT id, name FROM subjects")
        .fetch_all(pool).await?;
    let subject_names: HashMap<i64, Option<String>> = subject_rows.into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<Option<String>, _>("name")))
        .collect();

    // 5. Apply label rules
    let actions = compute_label_actions(&components, &face_subjects, &subject_names, MIN_COMPONENT_SIZE);
    let mut new_clusters_count = 0usize;
    let mut noise_count = 0usize;

    for action in actions {
        match action {
            LabelAction::AssignAll { faces, subject_id } => {
                for fid in faces {
                    db::update_face_subject(pool, fid, Some(subject_id)).await?;
                }
            }
            LabelAction::NewSubject { faces } => {
                let sid = db::insert_subject(pool, None, "person").await?;
                for fid in &faces {
                    db::update_face_subject(pool, *fid, Some(sid)).await?;
                }
                new_clusters_count += 1;
            }
            LabelAction::Noise { faces } => {
                for fid in &faces {
                    db::update_face_subject(pool, *fid, None).await?;
                }
                noise_count += faces.len();
            }
            LabelAction::SuggestMerge { .. } => {}  // handled by find_merge_suggestions
        }
    }

    // 6. Cleanup
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
    db::clear_merge_suggestions(pool).await?;

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
        if dismissed > 0 { continue; }

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

    fn emb(vals: &[f32]) -> Vec<f32> {
        vals.to_vec()
    }

    #[test]
    fn union_find_transitive_chain() {
        let mut uf = UnionFind::new();
        uf.union(1, 2);
        uf.union(2, 3);
        assert!(uf.connected(1, 3), "A-B + B-C edges must put A and C in same component");
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
        all_knn.insert(2, vec![(3, 0.85), (4, 0.7)]);  // face 1 absent from face 2's list
        all_knn.insert(3, vec![(2, 0.85), (1, 0.8)]);
        all_knn.insert(4, vec![(2, 0.7), (3, 0.6)]);

        let edges = compute_mutual_sim_edges(&all_knn, 0.55);
        let has_1_2 = edges.iter().any(|(a, b, _)| (*a == 1 && *b == 2) || (*a == 2 && *b == 1));
        assert!(!has_1_2, "1-2 must not be an edge: non-mutual (face 1 not in face 2's knn)");
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
        all_knn.insert(1, vec![(2, 0.4)]);  // 0.4 < 0.55
        all_knn.insert(2, vec![(1, 0.4)]);

        let edges = compute_mutual_sim_edges(&all_knn, 0.55);
        assert!(edges.is_empty(), "below-tau mutual pair must not create an edge");
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
        let sim_edges: Vec<(i64, i64, f32)> = vec![];  // no similarity edges pass tau
        let must_links = vec![(1i64, 2i64)];
        let cannot_links: HashSet<(i64, i64)> = HashSet::new();
        let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2, 3]);
        assert!(uf.connected(1, 2), "must_link must co-locate faces regardless of similarity");
        assert!(!uf.connected(1, 3), "face 3 has no edges — must stay isolated");
    }

    #[test]
    fn cannot_link_prevents_weakest_bridge_edge() {
        // Chain 1-2 (sim=0.9), 2-3 (sim=0.6); cannot-link (1,3)
        // Kruskal: add (1,2,0.9) → ok. Try (2,3,0.6) → would put 1 and 3 together → skip.
        let sim_edges = vec![(1i64, 2, 0.9_f32), (2, 3, 0.6)];
        let must_links: Vec<(i64, i64)> = vec![];
        let mut cannot_links: HashSet<(i64, i64)> = HashSet::new();
        cannot_links.insert((1i64, 3i64));
        let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2, 3]);
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
        let mut uf = build_components_with_constraints(sim_edges, &[], &cannot_links, &[1, 2, 3, 4]);
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
        let mut uf = build_components_with_constraints(sim_edges, &must_links, &cannot_links, &[1, 2]);
        // must_link is applied after sim-edge pass, so 1 and 2 end up connected
        assert!(uf.connected(1, 2), "must_link wins over cannot_link contradiction");
    }

    #[test]
    fn label_assign_one_subject_fills_unlabeled_faces() {
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2, 3])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = [(1i64, 10i64)].into_iter().collect(); // face 1 → subject 10
        let subject_names: HashMap<i64, Option<String>> = [(10i64, Some("Alice".to_string()))].into_iter().collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        let assign = actions.iter().find(|a| matches!(a, LabelAction::AssignAll { .. }));
        assert!(assign.is_some(), "should emit AssignAll for unlabeled faces 2 and 3");
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
        ].into_iter().collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(actions.iter().any(|a| matches!(a, LabelAction::SuggestMerge { .. })),
            "two named subjects must emit a suggestion");
        assert!(!actions.iter().any(|a| matches!(a, LabelAction::AssignAll { .. })),
            "unlabeled face 3 must NOT be auto-assigned in a two-subject component");
    }

    #[test]
    fn label_unlabeled_small_component_is_noise() {
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = HashMap::new();
        let subject_names: HashMap<i64, Option<String>> = HashMap::new();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(actions.iter().any(|a| matches!(a, LabelAction::Noise { .. })),
            "size-1 unlabeled component must be noise (below MIN_COMPONENT_SIZE=2)");
    }

    #[test]
    fn label_unlabeled_large_enough_component_gets_new_subject() {
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = HashMap::new();
        let subject_names: HashMap<i64, Option<String>> = HashMap::new();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(actions.iter().any(|a| matches!(a, LabelAction::NewSubject { .. })),
            "size>=2 unlabeled component must trigger new subject creation");
    }

    #[test]
    fn label_user_labeled_size_one_is_not_noise() {
        // A user-assigned face in a size-1 component must NOT be classified as noise.
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = [(1i64, 10i64)].into_iter().collect();
        let subject_names: HashMap<i64, Option<String>> = [(10i64, Some("Alice".to_string()))].into_iter().collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(!actions.iter().any(|a| matches!(a, LabelAction::Noise { faces } if faces.contains(&1))),
            "user-labeled face must not become noise even at size 1");
    }

    #[test]
    fn label_suggestion_requires_at_least_one_named_subject() {
        // Two UNNAMED subjects in same component → no suggestion emitted (neither is named)
        let components: HashMap<i64, Vec<i64>> = [(1i64, vec![1i64, 2])].into_iter().collect();
        let face_subjects: HashMap<i64, i64> = [(1i64, 10i64), (2i64, 20i64)].into_iter().collect();
        let subject_names: HashMap<i64, Option<String>> = [
            (10i64, None::<String>),
            (20i64, None::<String>),
        ].into_iter().collect();

        let actions = compute_label_actions(&components, &face_subjects, &subject_names, 2);
        assert!(!actions.iter().any(|a| matches!(a, LabelAction::SuggestMerge { .. })),
            "two unnamed subjects must not generate a merge suggestion");
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
            "INSERT INTO subjects (name, type, added_at) VALUES ('S', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let anchor_s: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
        ).bind(subject_s).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(anchor_s).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();

        let subject_s2: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('S2', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let anchor_s2: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
        ).bind(subject_s2).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(anchor_s2).bind(emb_bytes(&[0.998f32, 0.063, 0.0])).execute(&pool).await.unwrap();

        let face_f: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (3, NULL, 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(face_f).bind(emb_bytes(&[0.999f32, 0.045, 0.0])).execute(&pool).await.unwrap();

        crate::db::add_cannot_link(&pool, face_f, anchor_s, "removal").await.unwrap();

        cluster_unassigned_faces(&pool).await.unwrap();

        let assigned: Option<i64> = sqlx::query_scalar("SELECT subject_id FROM faces WHERE id = ?")
            .bind(face_f).fetch_one(&pool).await.unwrap();

        assert!(assigned != Some(subject_s),
            "face_f must NOT be reassigned to forbidden subject S (got {:?})", assigned);
    }

    #[tokio::test]
    async fn integration_merge_distant_groups_stays_merged_after_recluster() {
        let pool = make_integration_pool().await;

        let subject_a: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('A', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let fa1: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
        ).bind(subject_a).fetch_one(&pool).await.unwrap();
        let fa2: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
        ).bind(subject_a).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa1).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa2).bind(emb_bytes(&[0.99f32, 0.14, 0.0])).execute(&pool).await.unwrap();

        let subject_b: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('B', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let fb1: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (3, ?, 0) RETURNING id"
        ).bind(subject_b).fetch_one(&pool).await.unwrap();
        let fb2: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (4, ?, 0) RETURNING id"
        ).bind(subject_b).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb1).bind(emb_bytes(&[0.0f32, 1.0, 0.0])).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb2).bind(emb_bytes(&[0.14f32, 0.99, 0.0])).execute(&pool).await.unwrap();

        crate::db::add_must_link(&pool, fa1, fb1, "merge").await.unwrap();
        crate::db::add_must_link(&pool, fa1, fb2, "merge").await.unwrap();
        crate::db::add_must_link(&pool, fa2, fb1, "merge").await.unwrap();
        crate::db::add_must_link(&pool, fa2, fb2, "merge").await.unwrap();
        sqlx::query("UPDATE faces SET subject_id = ? WHERE subject_id = ?")
            .bind(subject_a).bind(subject_b).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM subjects WHERE id = ?")
            .bind(subject_b).execute(&pool).await.unwrap();

        cluster_unassigned_faces(&pool).await.unwrap();

        let subjects: Vec<Option<i64>> = sqlx::query_scalar("SELECT subject_id FROM faces ORDER BY id")
            .fetch_all(&pool).await.unwrap();
        let distinct: HashSet<Option<i64>> = subjects.into_iter().collect();
        assert_eq!(distinct.len(), 1, "all four faces must share one subject after recluster (must_link is durable)");
        assert!(distinct.iter().next().unwrap().is_some(), "subject must not be NULL");
    }

    #[tokio::test]
    async fn graph_suggestions_emitted_for_cross_subject_edges() {
        let pool = make_integration_pool().await;

        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let bob: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let fa: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
        ).bind(alice).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();

        let fb: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
        ).bind(bob).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb).bind(emb_bytes(&[0.99f32, 0.14, 0.0])).execute(&pool).await.unwrap();

        // Manually insert a face_edge between them (as recluster would)
        db::upsert_face_edge(&pool, fa, fb, 0.99).await.unwrap();

        find_merge_suggestions(&pool).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1, "one suggestion expected for Alice-Bob cross edge");
    }

    #[tokio::test]
    async fn graph_suggestions_skipped_for_cannot_link_pair() {
        let pool = make_integration_pool().await;

        let alice: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let bob: i64 = sqlx::query_scalar(
            "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let fa: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (1, ?, 0) RETURNING id"
        ).bind(alice).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fa).bind(emb_bytes(&[1.0f32, 0.0, 0.0])).execute(&pool).await.unwrap();

        let fb: i64 = sqlx::query_scalar(
            "INSERT INTO faces (image_id, subject_id, added_at) VALUES (2, ?, 0) RETURNING id"
        ).bind(bob).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO face_vectors(rowid, embedding) VALUES (?, ?)")
            .bind(fb).bind(emb_bytes(&[0.99f32, 0.14, 0.0])).execute(&pool).await.unwrap();

        db::upsert_face_edge(&pool, fa, fb, 0.99).await.unwrap();
        crate::db::add_cannot_link(&pool, fa, fb, "dismiss").await.unwrap();

        find_merge_suggestions(&pool).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM merge_suggestions")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0, "dismissed pair (cannot_link) must not be suggested");
    }
}
