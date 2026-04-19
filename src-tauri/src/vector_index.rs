use std::path::Path;
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"NEBULAVX";
const VERSION: u8 = 1;

// TODO: swap FlatIndex for HnswIndex if library grows past ~200k images
pub trait VectorIndex: Send + Sync {
    fn add(&mut self, id: i64, embedding: &[f32]);
    fn remove(&mut self, id: i64);
    fn search(&self, query: &[f32], limit: usize) -> Vec<(i64, f32)>;
    fn save(&self, path: &Path) -> anyhow::Result<()>;
    fn len(&self) -> usize;
}

pub type IndexStore = Arc<std::sync::RwLock<Box<dyn VectorIndex>>>;

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

pub struct FlatIndex {
    pub(crate) dim: usize,
    pub(crate) ids: Vec<i64>,   // -1 = tombstone
    pub(crate) vecs: Vec<f32>,  // flat: entry i occupies vecs[i*dim .. (i+1)*dim]
}

impl FlatIndex {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            ids: Vec::new(),
            vecs: Vec::new(),
        }
    }

    pub fn tombstone_count(&self) -> usize {
        self.ids.iter().filter(|&&id| id == -1).count()
    }

    /// Rebuild without tombstones. Vectors are already normalized; no re-normalization needed.
    pub fn compact(&self) -> Self {
        let mut new_idx = Self::new(self.dim);
        for (i, &id) in self.ids.iter().enumerate() {
            if id != -1 {
                let start = i * self.dim;
                new_idx.ids.push(id);
                new_idx.vecs.extend_from_slice(&self.vecs[start..start + self.dim]);
            }
        }
        new_idx
    }

    /// Add a pre-normalized vector directly (used by load/compact paths to skip re-normalization).
    pub(crate) fn add_raw(&mut self, id: i64, normalized: &[f32]) {
        if let Some(pos) = self.ids.iter().position(|&x| x == id) {
            let start = pos * self.dim;
            self.vecs[start..start + self.dim].copy_from_slice(normalized);
            return;
        }
        if let Some(pos) = self.ids.iter().position(|&x| x == -1) {
            self.ids[pos] = id;
            let start = pos * self.dim;
            self.vecs[start..start + self.dim].copy_from_slice(normalized);
            return;
        }
        self.ids.push(id);
        self.vecs.extend_from_slice(normalized);
    }
}

impl VectorIndex for FlatIndex {
    fn add(&mut self, id: i64, embedding: &[f32]) {
        let normalized = normalize(embedding);
        self.add_raw(id, &normalized);
    }

    fn remove(&mut self, id: i64) {
        if let Some(pos) = self.ids.iter().position(|&x| x == id) {
            self.ids[pos] = -1;
            let start = pos * self.dim;
            for v in &mut self.vecs[start..start + self.dim] {
                *v = 0.0;
            }
        }
    }

    fn search(&self, query: &[f32], limit: usize) -> Vec<(i64, f32)> {
        use rayon::prelude::*;
        let query_norm = normalize(query);
        let dim = self.dim;
        let mut scored: Vec<(i64, f32)> = self
            .ids
            .par_iter()
            .enumerate()
            .filter(|(_, &id)| id != -1)
            .map(|(i, &id)| {
                let start = i * dim;
                let vec = &self.vecs[start..start + dim];
                let dot: f32 = query_norm.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
                (id, dot)
            })
            .collect();
        scored.sort_unstable_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        scored
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        // Stub — implemented in Task 2
        let _ = path;
        Ok(())
    }

    fn len(&self) -> usize {
        self.ids.iter().filter(|&&id| id != -1).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index() -> FlatIndex {
        FlatIndex::new(3)
    }

    #[test]
    fn search_empty_returns_empty() {
        let idx = make_index();
        assert_eq!(idx.search(&[1.0, 0.0, 0.0], 10), vec![]);
    }

    #[test]
    fn add_and_search_returns_best_match() {
        let mut idx = make_index();
        idx.add(1, &[1.0, 0.0, 0.0]);
        idx.add(2, &[0.0, 1.0, 0.0]);
        let results = idx.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results[0].0, 1);
        assert!(results[0].1 > 0.99, "expected cosine ~1.0, got {}", results[0].1);
        assert_eq!(results[1].0, 2);
        assert!((results[1].1).abs() < 0.01, "expected cosine ~0.0, got {}", results[1].1);
    }

    #[test]
    fn search_respects_limit() {
        let mut idx = make_index();
        for i in 0..10i64 {
            idx.add(i, &[i as f32, 0.0, 0.0]);
        }
        let results = idx.search(&[1.0, 0.0, 0.0], 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn remove_tombstones_entry() {
        let mut idx = make_index();
        idx.add(1, &[1.0, 0.0, 0.0]);
        idx.add(2, &[0.8, 0.6, 0.0]);
        idx.remove(1);
        let results = idx.search(&[1.0, 0.0, 0.0], 10);
        assert!(!results.iter().any(|(id, _)| *id == 1), "removed id 1 should not appear");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn len_excludes_tombstones() {
        let mut idx = make_index();
        idx.add(1, &[1.0, 0.0, 0.0]);
        idx.add(2, &[0.0, 1.0, 0.0]);
        idx.remove(1);
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn add_same_id_updates_in_place() {
        let mut idx = make_index();
        idx.add(1, &[1.0, 0.0, 0.0]);
        idx.add(1, &[0.0, 1.0, 0.0]);
        assert_eq!(idx.len(), 1);
        let results = idx.search(&[0.0, 1.0, 0.0], 1);
        assert_eq!(results[0].0, 1);
        assert!(results[0].1 > 0.99);
    }
}
