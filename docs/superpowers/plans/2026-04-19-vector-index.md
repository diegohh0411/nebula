# Vector Index Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the per-query SQLite full-scan with a persistent in-memory flat vector index, making semantic image search O(n·rayon) instead of O(n·SQLite-IO).

**Architecture:** A `VectorIndex` trait with a `FlatIndex` implementation stores pre-normalized f32 vectors in memory, persisted as f16 on disk in a `nebula.idx` file alongside the SQLite DB. The index is loaded (or rebuilt from SQLite) once at startup and updated incrementally by the embedder worker. `search_images` reads from the index instead of the DB.

**Tech Stack:** Rust, `half` crate (f16), `rayon` (already present), `sqlx` (already present), Tauri managed state.

---

## File Map

| File | Action |
|------|--------|
| `src-tauri/Cargo.toml` | Add `half = "2"` |
| `src-tauri/src/vector_index.rs` | **Create**: `VectorIndex` trait, `IndexStore` type alias, `FlatIndex` struct with all operations |
| `src-tauri/src/lib.rs` | Add `mod vector_index`, add `index: IndexStore` to `AppState`, init at startup, thread workers |
| `src-tauri/src/embedder.rs` | `run_semantic_worker` and `process_semantic_one` accept `IndexStore` + `data_dir`; update index after each image |
| `src-tauri/src/search.rs` | `search_images` takes `&IndexStore` instead of pool; `build_search_results` guards `deleted_at` |
| `src-tauri/src/commands.rs` | Pass `&state.index` to `search_images` at both call sites |

---

### Task 1: `VectorIndex` Trait + `FlatIndex` Core (add / remove / search)

**Files:**
- Create: `src-tauri/src/vector_index.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add `half` to Cargo.toml**

In `src-tauri/Cargo.toml`, after the `ndarray` line:

```toml
half = "2"
```

- [ ] **Step 2: Create `src-tauri/src/vector_index.rs` with failing tests**

Create the file with just the tests (they will fail to compile until the types exist — that's the TDD signal):

```rust
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
```

- [ ] **Step 3: Run tests to verify compilation failure**

```bash
cd src-tauri && cargo test vector_index 2>&1 | head -20
```

Expected: compile error — `FlatIndex` is not defined. This confirms the tests are in place before the implementation.

- [ ] **Step 4: Implement `FlatIndex` — add this block before the `#[cfg(test)]` section**

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd src-tauri && cargo test vector_index 2>&1
```

Expected output: all 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/vector_index.rs
git commit -m "feat: add VectorIndex trait and FlatIndex core (add/remove/search)"
```

---

### Task 2: `FlatIndex` Persistence (save / load / load_or_rebuild)

**Files:**
- Modify: `src-tauri/src/vector_index.rs`

- [ ] **Step 1: Add persistence tests inside the existing `#[cfg(test)]` block**

Add these tests after the existing ones in the `tests` module:

```rust
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
```

- [ ] **Step 2: Run tests — persistence ones should fail (save is a stub)**

```bash
cd src-tauri && cargo test vector_index 2>&1
```

Expected: `save_and_load_roundtrip` fails because `save` is currently a no-op. `compact_removes_tombstones` should pass already.

- [ ] **Step 3: Replace the stub `save` and add `load` + `load_or_rebuild` to `FlatIndex`**

Replace the stub `save` implementation inside `impl VectorIndex for FlatIndex` and add the new associated functions to `impl FlatIndex`. Add `use std::io::{Read, Write};` at the top of the file.

Full updated `vector_index.rs` — replace the entire file content:

```rust
use std::io::{Read, Write};
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
    pub(crate) ids: Vec<i64>,
    pub(crate) vecs: Vec<f32>,
}

impl FlatIndex {
    pub fn new(dim: usize) -> Self {
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
            ids.push(i64::from_le_bytes(id_bytes));
            for _ in 0..dim {
                f.read_exact(&mut f16_bytes)?;
                vecs.push(half::f16::from_le_bytes(f16_bytes).to_f32());
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
                        eprintln!("[vector-index] Compacting {tomb} tombstones out of {total}");
                        let compacted = index.compact();
                        let path2 = idx_path.clone();
                        let snap = compacted.snapshot();
                        tokio::task::spawn_blocking(move || snap.save(&path2)).await??;
                        return Ok(compacted);
                    }
                    eprintln!("[vector-index] Loaded {} entries from disk", index.len());
                    return Ok(index);
                }
                Err(e) => eprintln!("[vector-index] Failed to load .idx (rebuilding): {e}"),
            }
        }

        eprintln!("[vector-index] Rebuilding index from SQLite…");
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
            eprintln!("[vector-index] Failed to save .idx: {e}");
        }

        eprintln!("[vector-index] Built index with {} entries", index.len());
        Ok(index)
    }

    /// Returns a saveable snapshot — a compacted clone used for serialization in spawn_blocking.
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

            // Header
            f.write_all(MAGIC)?;
            f.write_all(&[VERSION])?;
            f.write_all(&(self.dim as u32).to_le_bytes())?;
            f.write_all(&(self.ids.len() as u64).to_le_bytes())?;

            // Entries: i64 id + dim × f16
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
```

- [ ] **Step 4: Run all tests**

```bash
cd src-tauri && cargo test vector_index 2>&1
```

Expected: all 9 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vector_index.rs
git commit -m "feat: add FlatIndex persistence (save/load/load_or_rebuild)"
```

---

### Task 3: Wire `FlatIndex` into `AppState` and Startup

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `mod vector_index` and update `AppState`**

In `src-tauri/src/lib.rs`, add the module declaration after the existing `mod` block and add the field to `AppState`:

```rust
mod vector_index;  // ← add after existing mods
```

Replace the `AppState` struct:

```rust
pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub api_key: Arc<Mutex<Option<String>>>,
    pub indexer: Arc<indexer::Indexer>,
    pub vision_engine: Arc<vision_engine::VisionEngine>,
    pub index: vector_index::IndexStore,
}
```

- [ ] **Step 2: Initialize the index in `setup` and register it in `AppState`**

In the `setup` closure, after the `let pool = ...` line and before `let api_key = ...`, add:

```rust
let flat_index = tauri::async_runtime::block_on(
    vector_index::FlatIndex::load_or_rebuild(&data_dir, &pool)
)?;
let index: vector_index::IndexStore = Arc::new(std::sync::RwLock::new(Box::new(flat_index)));
```

Then add `index: index.clone()` to the `AppState { ... }` struct literal.

- [ ] **Step 3: Verify the project compiles (tests will be fixed in later tasks)**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: errors only in `embedder.rs`, `search.rs`, `commands.rs` about missing `index` argument — that's expected and will be fixed in Tasks 4 and 5.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: add IndexStore to AppState and initialize at startup"
```

---

### Task 4: Update `embedder.rs` — Incremental Index Writes

**Files:**
- Modify: `src-tauri/src/embedder.rs`

- [ ] **Step 1: Add `index` and `data_dir` parameters to `process_semantic_one`**

Change the signature of `process_semantic_one` (it's a private `async fn` in `embedder.rs`):

```rust
async fn process_semantic_one(
    pool: &SqlitePool,
    app: &AppHandle,
    vision_engine: &crate::vision_engine::VisionEngine,
    queue_id: i64,
    image_id: i64,
    attempts: i32,
    index: &crate::vector_index::IndexStore,
    data_dir: &std::path::Path,
) {
```

- [ ] **Step 2: Update index after a successful embedding**

In `process_semantic_one`, replace the `Ok(values)` arm:

```rust
Ok(values) => {
    let blob = f32_slice_to_bytes(&values);
    if db::mark_semantic_analysis_done(pool, image_id, &blob).await.is_ok() {
        index.write().unwrap().add(image_id, &values);
        let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
    }
}
```

- [ ] **Step 3: Replace `run_semantic_worker` body with the updated version**

Replace the entire `run_semantic_worker` function with:

```rust
pub async fn run_semantic_worker(
    pool: SqlitePool,
    app: AppHandle,
    vision_engine: Arc<crate::vision_engine::VisionEngine>,
    index: crate::vector_index::IndexStore,
    data_dir: std::path::PathBuf,
) {
    vision_engine.wait_until_ready().await;

    let semaphore = Arc::new(Semaphore::new(CONCURRENT_WORKERS));

    loop {
        let batch = match db::get_queue_batch(&pool, "semantic", (CONCURRENT_WORKERS * 2) as i64).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[semantic-worker] Failed to fetch batch: {}", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        if batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let mut handles = vec![];
        for (queue_id, image_id, attempts) in batch {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let pool_c = pool.clone();
            let app_c = app.clone();
            let ve_c = Arc::clone(&vision_engine);
            let index_c = Arc::clone(&index);
            let data_dir_c = data_dir.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                process_semantic_one(
                    &pool_c, &app_c, ve_c.as_ref(),
                    queue_id, image_id, attempts,
                    &index_c, &data_dir_c,
                ).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        // Persist index snapshot after each batch so new embeddings survive a crash
        let snap_path = data_dir.join("nebula.idx");
        let guard = index.read().unwrap();
        if let Err(e) = guard.save(&snap_path) {
            eprintln!("[semantic-worker] Failed to save index snapshot: {e}");
        }
    }
}
```

- [ ] **Step 5: Update the spawn call in `lib.rs`**

In `src-tauri/src/lib.rs`, update the `run_semantic_worker` spawn:

```rust
let pool_semantic = pool.clone();
let app_handle_semantic = app.handle().clone();
let vision_engine_semantic = Arc::clone(&vision_engine);
let index_semantic = index.clone();
let data_dir_semantic = data_dir.clone();
tauri::async_runtime::spawn(async move {
    embedder::run_semantic_worker(
        pool_semantic,
        app_handle_semantic,
        vision_engine_semantic,
        index_semantic,
        data_dir_semantic,
    ).await;
});
```

- [ ] **Step 6: Verify compilation**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: only `commands.rs` and `search.rs` errors remain.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/embedder.rs src-tauri/src/lib.rs
git commit -m "feat: update embedder to write new embeddings into vector index"
```

---

### Task 5: Update `search.rs` and `commands.rs` — Replace Brute-Force Scan

**Files:**
- Modify: `src-tauri/src/search.rs`
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Rewrite `search_images` to use `IndexStore`**

Replace the entire `search_images` function in `src-tauri/src/search.rs`:

```rust
/// Search the in-memory vector index for the top `limit` images most similar to `query_embedding`.
pub async fn search_images(
    index: &crate::vector_index::IndexStore,
    query_embedding: Vec<f32>,
    limit: usize,
) -> Result<Vec<(i64, f32)>> {
    let index = std::sync::Arc::clone(index);
    let results = tokio::task::spawn_blocking(move || {
        let guard = index.read().unwrap();
        let mut scored = guard.search(&query_embedding, limit);

        // GAP HEURISTIC (Elbow method): cut at the largest similarity drop >= 0.05
        if scored.len() > 1 {
            let mut max_gap = 0.0f32;
            let mut cut_index = scored.len();
            for i in 0..scored.len() - 1 {
                let gap = scored[i].1 - scored[i + 1].1;
                if gap > max_gap && gap >= 0.05 {
                    max_gap = gap;
                    cut_index = i + 1;
                }
            }
            scored.truncate(cut_index);
        }
        scored
    })
    .await?;
    Ok(results)
}
```

The import at the top of `search.rs` no longer needs `db` or `SqlitePool`. Remove the unused `db` import and update the use declaration to only keep what's still needed:

```rust
use anyhow::Result;
```

- [ ] **Step 2: Add `deleted_at` guard in `build_search_results`**

In `build_search_results`, update the `if let` block to skip soft-deleted images:

```rust
pub async fn build_search_results(
    pool: &sqlx::SqlitePool,
    scored: Vec<(i64, f32)>,
) -> Result<Vec<SearchResult>> {
    let mut results = Vec::with_capacity(scored.len());
    for (image_id, score) in scored {
        if let Ok(Some(img)) = db::get_image_by_id(pool, image_id).await {
            if img.deleted_at.is_some() {
                continue; // guard against tombstone/soft-delete race
            }
            results.push(SearchResult {
                image_id,
                path: img.path,
                thumbnail_path: img.thumbnail_path,
                score,
                date_taken: img.date_taken,
                mtime: img.mtime,
                semantic_analysis_done: img.semantic_analysis_done,
                subject_analysis_done: img.subject_analysis_done,
            });
        }
    }
    Ok(results)
}
```

The `build_search_results` function still needs `db` and `SqlitePool`, so add these back to the use declaration in `search.rs`:

```rust
use anyhow::Result;
use sqlx::SqlitePool;

use crate::{db, models::SearchResult};
```

- [ ] **Step 3: Update both call sites in `commands.rs`**

In `src-tauri/src/commands.rs`, find the two `search::search_images(pool, ...)` calls and replace them:

Line ~103 (text search path):
```rust
if let Ok(scored) = search::search_images(&state.index, query_embedding, 50).await {
```

Line ~125 (image-id search path):
```rust
let mut scored = search::search_images(&state.index, embedding_f32, 50)
    .await
    .map_err(map_err)?;
```

Find and update the third call site if it exists (image-bytes path, ~line 150):
```rust
if let Ok(scored) = search::search_images(&state.index, query_embedding, 50).await {
```

- [ ] **Step 4: Full build and all tests pass**

```bash
cd src-tauri && cargo build 2>&1
cd src-tauri && cargo test 2>&1
```

Expected: clean build, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/search.rs src-tauri/src/commands.rs
git commit -m "feat: replace SQLite brute-force scan with in-memory vector index in search"
```

---

## Done

After Task 5, every semantic image search reads from the in-memory `FlatIndex` instead of loading all embeddings from SQLite. The index is built once at startup (from `nebula.idx` if it exists, otherwise from SQLite) and updated incrementally by the embedder worker. All existing search behavior — gap heuristic, result shapes, subject-name matching — is preserved.
