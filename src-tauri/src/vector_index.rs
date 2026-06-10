use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use log::{info, warn, error, debug};

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

fn normalize(v: &[f32]) -> Option<Vec<f32>> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-10 {
        return None;
    }
    Some(v.iter().map(|x| x / norm).collect())
}

pub struct FlatIndex {
    pub(crate) dim: usize,
    pub(crate) ids: Vec<i64>,  // -1 = tombstone
    pub(crate) vecs: Vec<f32>, // flat: entry i occupies vecs[i*dim .. (i+1)*dim]
}

impl FlatIndex {
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "FlatIndex dim must be positive");
        Self { dim, ids: Vec::new(), vecs: Vec::new() }
    }

    pub fn tombstone_count(&self) -> usize {
        self.ids.iter().filter(|&&id| id == -1).count()
    }

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

    pub(crate) fn add_raw(&mut self, id: i64, normalized: &[f32]) {
        debug_assert_eq!(normalized.len(), self.dim, "embedding dim mismatch");
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

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let mut f = std::fs::File::open(path)?;

        let mut magic = [0u8; 8];
        f.read_exact(&mut magic)?;
        anyhow::ensure!(&magic == MAGIC, "invalid .idx magic bytes");

        let mut version = [0u8; 1];
        f.read_exact(&mut version)?;
        anyhow::ensure!(version[0] == VERSION, "unsupported .idx version {}", version[0]);

        let mut dim_bytes = [0u8; 4];
        f.read_exact(&mut dim_bytes)?;
        let dim = u32::from_le_bytes(dim_bytes) as usize;
        anyhow::ensure!(dim > 0 && dim <= 4096, "suspicious dim={}", dim);

        let mut count_bytes = [0u8; 8];
        f.read_exact(&mut count_bytes)?;
        let count = u64::from_le_bytes(count_bytes) as usize;

        let mut ids = Vec::with_capacity(count);
        let mut vecs = Vec::with_capacity(count * dim);

        let mut id_bytes = [0u8; 8];
        let mut f16_bytes = [0u8; 2];

        for _ in 0..count {
            f.read_exact(&mut id_bytes)?;
            let id = i64::from_le_bytes(id_bytes);
            let mut entry = Vec::with_capacity(dim);
            for _ in 0..dim {
                f.read_exact(&mut f16_bytes)?;
                entry.push(half::f16::from_le_bytes(f16_bytes).to_f32());
            }
            if id != -1 {
                ids.push(id);
                vecs.extend_from_slice(&entry);
            }
        }

        Ok(Self { dim, ids, vecs })
    }

    pub async fn load_or_rebuild(
        data_dir: &Path,
        pool: &sqlx::SqlitePool,
    ) -> anyhow::Result<Self> {
        let idx_path = data_dir.join("nebula.idx");

        if idx_path.exists() {
            let path = idx_path.clone();
            match tokio::task::spawn_blocking(move || Self::load(&path)).await? {
                Ok(index) => {
                    let tomb = index.tombstone_count();
                    let total = index.ids.len();
                    if total > 0 && tomb * 10 > total {
                        info!("[vector-index] Compacting {tomb} tombstones out of {total}");
                        let compacted = index.compact();
                        let snap = compacted.snapshot();
                        let path2 = idx_path.clone();
                        tokio::task::spawn_blocking(move || snap.save(&path2)).await??;
                        return Ok(compacted);
                    }
                    info!("[vector-index] Loaded {} entries from disk", index.len());
                    return Ok(index);
                }
                Err(e) => error!("[vector-index] Failed to load .idx (rebuilding): {e}"),
            }
        }

        info!("[vector-index] Rebuilding index from SQLite…");
        let all = crate::db::get_all_embeddings(pool).await?;

        let dim = all
            .first()
            .map(|(_, blob)| blob.len() / 4)
            .unwrap_or(768);

        let mut index = Self::new(dim);
        for (id, blob) in all {
            if let Ok(vec) = crate::embedder::bytes_to_f32_vec(&blob) {
                index.add(id, &vec);
            }
        }

        let snap = index.snapshot();
        let path = idx_path;
        if let Err(e) = tokio::task::spawn_blocking(move || snap.save(&path)).await? {
            error!("[vector-index] Failed to save .idx: {e}");
        }

        info!("[vector-index] Built index with {} entries", index.len());
        Ok(index)
    }

    /// Returns a saveable snapshot used for serialization in spawn_blocking.
    pub fn snapshot(&self) -> FlatIndexSnapshot {
        FlatIndexSnapshot {
            dim: self.dim,
            ids: self.ids.clone(),
            vecs: self.vecs.clone(),
        }
    }
}

/// A sendable, cloneable snapshot of `FlatIndex` used only for saving to disk.
pub struct FlatIndexSnapshot {
    dim: usize,
    ids: Vec<i64>,
    vecs: Vec<f32>,
}

impl FlatIndexSnapshot {
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let tmp = path.with_extension("idx.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;

            f.write_all(MAGIC)?;
            f.write_all(&[VERSION])?;
            f.write_all(&(self.dim as u32).to_le_bytes())?;
            f.write_all(&(self.ids.len() as u64).to_le_bytes())?;

            for (i, &id) in self.ids.iter().enumerate() {
                f.write_all(&id.to_le_bytes())?;
                let start = i * self.dim;
                for &v in &self.vecs[start..start + self.dim] {
                    f.write_all(&half::f16::from_f32(v).to_le_bytes())?;
                }
            }
        }
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

impl VectorIndex for FlatIndex {
    fn add(&mut self, id: i64, embedding: &[f32]) {
        if let Some(normalized) = normalize(embedding) {
            self.add_raw(id, &normalized);
        }
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
        let query_norm = match normalize(query) {
            Some(n) => n,
            None => return vec![],
        };
        let dim = self.dim;
        let mut scored: Vec<(i64, f32)> = self
            .ids
            .par_iter()
            .enumerate()
            .filter(|(_, &id)| id != -1)
            .map(|(i, &id)| {
                let start = i * dim;
                let dot: f32 = query_norm
                    .iter()
                    .zip(self.vecs[start..start + dim].iter())
                    .map(|(a, b)| a * b)
                    .sum();
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
        self.snapshot().save(path)
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
        assert_eq!(results.len(), 2, "expected 2 results");
        assert_eq!(results[0].0, 1);
        assert!(results[0].1 > 0.99, "expected cosine ~1.0, got {}", results[0].1);
        assert_eq!(results[1].0, 2);
        assert!((results[1].1).abs() < 0.01, "expected cosine ~0.0, got {}", results[1].1);
    }

    #[test]
    fn search_respects_limit() {
        let mut idx = make_index();
        idx.add(1, &[1.0, 0.0, 0.0]);
        idx.add(2, &[0.9, 0.1, 0.0]);
        idx.add(3, &[0.8, 0.2, 0.0]);
        idx.add(4, &[0.0, 1.0, 0.0]);
        idx.add(5, &[0.0, 0.0, 1.0]);
        let results = idx.search(&[1.0, 0.0, 0.0], 3);
        assert_eq!(results.len(), 3);
        let ids: Vec<i64> = results.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        assert!(ids.contains(&3));
    }

    #[test]
    fn remove_tombstones_entry() {
        let mut idx = make_index();
        idx.add(1, &[1.0, 0.0, 0.0]);
        idx.add(2, &[0.8, 0.6, 0.0]);
        idx.remove(1);
        let results = idx.search(&[1.0, 0.0, 0.0], 10);
        assert!(!results.iter().any(|(id, _)| *id == 1));
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

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("nebula_test_{}.idx", std::process::id()));
        let mut idx = FlatIndex::new(3);
        idx.add(10, &[1.0, 0.0, 0.0]);
        idx.add(20, &[0.0, 1.0, 0.0]);
        idx.save(&tmp).unwrap();

        let loaded = FlatIndex::load(&tmp).unwrap();
        assert_eq!(loaded.len(), 2);

        let results = loaded.search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results[0].0, 10);
        assert!(results[0].1 > 0.99);

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn load_wrong_magic_returns_error() {
        let tmp = std::env::temp_dir().join(format!("nebula_bad_{}.idx", std::process::id()));
        std::fs::write(&tmp, b"BADMAGIC12345").unwrap();
        assert!(FlatIndex::load(&tmp).is_err());
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn compact_removes_tombstones() {
        let mut idx = FlatIndex::new(3);
        idx.add(1, &[1.0, 0.0, 0.0]);
        idx.add(2, &[0.0, 1.0, 0.0]);
        idx.add(3, &[0.0, 0.0, 1.0]);
        idx.remove(2);
        let compacted = idx.compact();
        assert_eq!(compacted.len(), 2);
        assert_eq!(compacted.tombstone_count(), 0);
    }
}
