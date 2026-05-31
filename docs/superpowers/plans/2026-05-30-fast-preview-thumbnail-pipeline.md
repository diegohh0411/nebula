# Fast Preview / Thumbnail Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make grid previews appear almost instantly by moving thumbnail generation into a dedicated, decoupled subsystem with two-tier scaled decode, viewport prioritization, and a burst-then-yield CPU governor.

**Architecture:** A new `src-tauri/src/preview.rs` module owns thumbnailing end-to-end, fully outside the inference pipeline (which loses thumbnail duties entirely). A `PreviewService` runs a worker pool fed by a high/low priority queue: low priority = backlog + newly-indexed images, high priority = the frontend's current viewport. Each worker does a cheap scaled decode and writes two tiers — a tiny instant preview then the 800px thumbnail. A governor bursts to all cores briefly on demand, then trickles down so inference reclaims the CPU.

**Tech Stack:** Rust, Tokio, SQLx (SQLite), Tauri 2, `image` 0.25, `jpeg-decoder` (added), Angular frontend.

**Spec:** `docs/superpowers/specs/2026-05-30-fast-preview-thumbnail-pipeline-design.md`

**Note on DB:** The app is alpha. Schema changes go inline into `BASE_SCHEMA`; there is **no migration**. After implementation, wipe `APP_DATA` and reboot to get a fresh DB.

**Test command:** All Rust tests run with `cd src-tauri && cargo test`. Run individual tests with `cargo test <name>`.

---

## File Structure

- **Create** `src-tauri/src/preview.rs` — the entire preview subsystem: `decode_at_most`, `PreviewQueue`, `Governor`, `PreviewHandle`, `PreviewService`, worker logic.
- **Modify** `src-tauri/Cargo.toml` — add `jpeg-decoder = "0.3"`.
- **Modify** `src-tauri/src/db.rs` — add `preview_path` to schema + queries + `row_to_image`; add `update_preview_path`, `images_needing_preview`; clear preview/thumbnail paths on hash change.
- **Modify** `src-tauri/src/models/entities.rs` — add `preview_path` to `Image` and `SearchResult`.
- **Modify** `src-tauri/src/search.rs` — populate `preview_path` in the `SearchResult` it builds; and `commands.rs` `search` builder.
- **Modify** `src-tauri/src/pipeline/mod.rs` — delete the Stage-1 thumbnail block + `thumb_sem`.
- **Modify** `src-tauri/src/indexer.rs` — accept a `PreviewHandle`, enqueue images after insert / hash-change.
- **Modify** `src-tauri/src/commands.rs` — add `prioritize_previews` command.
- **Modify** `src-tauri/src/lib.rs` — declare `mod preview`, build `PreviewService`, store `PreviewHandle` in `AppState`, register command.
- **Modify** `src/app/models/models.ts` — add `preview_path`.
- **Modify** `src/app/components/photo-grid/photo-grid.component.ts` + `.html` — `thumbUrl` fallback to `preview_path`; IntersectionObserver reporting visible IDs.
- **Modify** `src/app/services/photo.service.ts` — `prioritizePreviews(ids)` invoking the command.

---

## Task 1: Add `jpeg-decoder` dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `src-tauri/Cargo.toml`, under `[dependencies]`, after the `image = ...` line (line 31), add:

```toml
jpeg-decoder = "0.3"
```

- [ ] **Step 2: Verify it resolves and compiles**

Run: `cd src-tauri && cargo build`
Expected: builds successfully (downloads `jpeg-decoder`).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "build: add jpeg-decoder for scaled JPEG decode (TT-15)"
```

---

## Task 2: `decode_at_most` — scaled decode helper

Decodes an image at a coarse downscale ≤ a target longest edge. JPEG uses `jpeg-decoder`'s power-of-two scaled decode; everything else uses `image::open`. The caller resizes to the exact size afterward.

**Files:**
- Create: `src-tauri/src/preview.rs`
- Modify: `src-tauri/src/lib.rs` (declare module)

- [ ] **Step 1: Declare the module**

In `src-tauri/src/lib.rs`, add after line 7 (`mod preprocess;`):

```rust
mod preview;
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/preview.rs` with:

```rust
use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::Path;

/// Decode an image at a coarse downscale no larger than `target_long_edge`
/// on its longest side. For JPEG this scales DURING decode (power-of-two)
/// via `jpeg-decoder`; other formats decode fully via `image`. The caller
/// is responsible for the final exact resize.
pub fn decode_at_most(path: &Path, target_long_edge: u32) -> Result<DynamicImage> {
    let is_jpeg = matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
        Some("jpg" | "jpeg")
    );
    if is_jpeg {
        if let Ok(img) = decode_jpeg_scaled(path, target_long_edge) {
            return Ok(img);
        }
        // fall through to full decode on any jpeg-decoder failure / unsupported format
    }
    image::open(path).with_context(|| format!("failed to decode {}", path.display()))
}

fn decode_jpeg_scaled(path: &Path, target_long_edge: u32) -> Result<DynamicImage> {
    use jpeg_decoder::{Decoder, PixelFormat};
    let file = std::fs::File::open(path)?;
    let mut decoder = Decoder::new(std::io::BufReader::new(file));
    let t = target_long_edge as u16;
    // scale() requests an output size; jpeg-decoder rounds to a power-of-two
    // downscale (1, 1/2, 1/4, 1/8) and returns the actual chosen dimensions.
    let (w, h) = decoder.scale(t, t)?;
    let pixels = decoder.decode()?;
    let info = decoder.info().context("jpeg info missing after decode")?;
    let (w, h) = (w as u32, h as u32);
    match info.pixel_format {
        PixelFormat::RGB24 => {
            let buf = image::RgbImage::from_raw(w, h, pixels)
                .context("rgb buffer size mismatch")?;
            Ok(DynamicImage::ImageRgb8(buf))
        }
        PixelFormat::L8 => {
            let buf = image::GrayImage::from_raw(w, h, pixels)
                .context("luma buffer size mismatch")?;
            Ok(DynamicImage::ImageLuma8(buf))
        }
        // CMYK32, L16, etc.: let the caller's image::open fallback handle it.
        _ => anyhow::bail!("unsupported jpeg pixel format for scaled decode"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_jpeg(w: u32, h: u32) -> std::path::PathBuf {
        let mut img = image::RgbImage::new(w, h);
        for p in img.pixels_mut() { *p = image::Rgb([120, 180, 60]); }
        let path = std::env::temp_dir()
            .join(format!("nebula_dec_{}_{}x{}.jpg", std::process::id(), w, h));
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(&path, image::ImageFormat::Jpeg).unwrap();
        path
    }

    #[test]
    fn decode_at_most_scales_large_jpeg_down() {
        let path = write_jpeg(2000, 1000);
        let img = decode_at_most(&path, 256).unwrap();
        // Coarse scale: result must be no larger than the original and non-empty.
        assert!(img.width() > 0 && img.height() > 0);
        assert!(img.width() <= 2000 && img.height() <= 1000);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn decode_at_most_does_not_upscale_small_image() {
        let path = write_jpeg(100, 80);
        let img = decode_at_most(&path, 256).unwrap();
        assert!(img.width() <= 100 && img.height() <= 80);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn decode_at_most_errors_on_missing_file() {
        let res = decode_at_most(Path::new("definitely-not-here.jpg"), 256);
        assert!(res.is_err());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail to compile / fail**

Run: `cd src-tauri && cargo test preview::tests`
Expected: compiles (module is wired) and tests PASS — this helper is self-contained. If `jpeg-decoder`'s `scale`/`PixelFormat` API names differ for the resolved version, fix the call sites until it compiles. (`Decoder::scale(&mut self, w: u16, h: u16) -> Result<(u16,u16)>` and `info().pixel_format` are correct for `jpeg-decoder` 0.3.)

- [ ] **Step 4: Confirm pass**

Run: `cd src-tauri && cargo test preview::tests`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/preview.rs src-tauri/src/lib.rs
git commit -m "feat(preview): scaled decode helper decode_at_most (TT-15)"
```

---

## Task 3: Tier writers — `write_preview` and `write_thumbnail`

Two functions that take an image path + id + data_dir and produce each tier's WebP, returning the written path.

**Files:**
- Modify: `src-tauri/src/preview.rs`
- Uses existing: `src-tauri/src/thumbnail.rs` (`thumbnail_path_for`, `write_thumbnail_from_image`)

- [ ] **Step 1: Add a tiny-preview path helper to `thumbnail.rs`**

In `src-tauri/src/thumbnail.rs`, after `thumbnail_path_for` (line 15), add:

```rust
pub fn preview_path_for(data_dir: &Path, image_id: i64) -> PathBuf {
    thumbnail_cache_dir(data_dir).join(format!("{}_p.webp", image_id))
}
```

- [ ] **Step 2: Write the failing tests in `preview.rs`**

Append to the `tests` module in `src-tauri/src/preview.rs`:

```rust
    #[test]
    fn write_preview_creates_small_webp() {
        let data_dir = std::env::temp_dir()
            .join(format!("nebula_prev_{}", std::process::id()));
        let src = write_jpeg(1600, 1200);
        let out = write_preview(&src, 7, &data_dir).unwrap();
        assert!(out.exists());
        let loaded = image::open(&out).unwrap();
        assert!(loaded.width() <= 256 && loaded.height() <= 256);
        std::fs::remove_dir_all(&data_dir).ok();
        std::fs::remove_file(&src).ok();
    }

    #[test]
    fn write_thumbnail_creates_800px_webp() {
        let data_dir = std::env::temp_dir()
            .join(format!("nebula_thumb_{}", std::process::id()));
        let src = write_jpeg(1600, 1200);
        let out = write_thumbnail(&src, 7, &data_dir).unwrap();
        assert!(out.exists());
        let loaded = image::open(&out).unwrap();
        assert!(loaded.width() <= 800 && loaded.height() <= 800);
        assert!(loaded.width() == 800 || loaded.height() == 800);
        std::fs::remove_dir_all(&data_dir).ok();
        std::fs::remove_file(&src).ok();
    }
```

- [ ] **Step 3: Implement the writers**

In `src-tauri/src/preview.rs`, add (after `decode_jpeg_scaled`):

```rust
use std::path::PathBuf;

/// Tier 1: decode coarsely, resize to <=256px longest edge, write WebP.
pub fn write_preview(src: &Path, image_id: i64, data_dir: &Path) -> Result<PathBuf> {
    let img = decode_at_most(src, 256)?;
    let small = if img.width() > 256 || img.height() > 256 {
        img.resize(256, 256, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let dest = crate::thumbnail::preview_path_for(data_dir, image_id);
    if let Some(parent) = dest.parent() { std::fs::create_dir_all(parent)?; }
    small.save_with_format(&dest, image::ImageFormat::WebP)?;
    Ok(dest)
}

/// Tier 2: decode with headroom, resize to 800px longest edge, write WebP.
pub fn write_thumbnail(src: &Path, image_id: i64, data_dir: &Path) -> Result<PathBuf> {
    let img = decode_at_most(src, 1600)?;
    let dest = crate::thumbnail::thumbnail_path_for(data_dir, image_id);
    crate::thumbnail::write_thumbnail_from_image(&img, &dest)?;
    Ok(dest)
}
```

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test preview::tests`
Expected: all preview tests pass (5 total).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/preview.rs src-tauri/src/thumbnail.rs
git commit -m "feat(preview): two-tier WebP writers (TT-15)"
```

---

## Task 4: `PreviewQueue` — priority + dedup (pure logic)

**Files:**
- Modify: `src-tauri/src/preview.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:

```rust
    #[test]
    fn queue_drains_high_before_low() {
        let mut q = PreviewQueue::new();
        assert!(q.enqueue_low(1));
        assert!(q.enqueue_low(2));
        q.enqueue_high(2);
        assert_eq!(q.next(), Some(2)); // promoted
        assert_eq!(q.next(), Some(1));
        assert_eq!(q.next(), None);
    }

    #[test]
    fn promoting_low_id_does_not_double_process() {
        let mut q = PreviewQueue::new();
        q.enqueue_low(5);
        q.enqueue_high(5);
        assert_eq!(q.next(), Some(5));
        assert_eq!(q.next(), None); // not still sitting in low
    }

    #[test]
    fn enqueue_is_deduped() {
        let mut q = PreviewQueue::new();
        assert!(q.enqueue_low(1));
        assert!(!q.enqueue_low(1)); // already seen
        assert_eq!(q.next(), Some(1));
        assert_eq!(q.next(), None);
    }

    #[test]
    fn high_nonempty_reflects_state() {
        let mut q = PreviewQueue::new();
        assert!(!q.high_nonempty());
        q.enqueue_high(9);
        assert!(q.high_nonempty());
        q.next();
        assert!(!q.high_nonempty());
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cd src-tauri && cargo test preview::tests::queue`
Expected: FAIL — `PreviewQueue` not defined.

- [ ] **Step 3: Implement `PreviewQueue`**

In `src-tauri/src/preview.rs` (top-level, after imports), add:

```rust
use std::collections::{HashSet, VecDeque};

/// High/low priority work queue for preview generation, with dedup.
pub struct PreviewQueue {
    high: VecDeque<i64>,
    low: VecDeque<i64>,
    seen: HashSet<i64>,
}

impl PreviewQueue {
    pub fn new() -> Self {
        Self { high: VecDeque::new(), low: VecDeque::new(), seen: HashSet::new() }
    }

    /// Enqueue at low priority. Returns true if newly added (not seen before).
    pub fn enqueue_low(&mut self, id: i64) -> bool {
        if !self.seen.insert(id) {
            return false;
        }
        self.low.push_back(id);
        true
    }

    /// Enqueue/promote at high priority. If already queued in low, move it to
    /// high; if unseen, add to high; if already in high or done, no-op.
    pub fn enqueue_high(&mut self, id: i64) {
        if let Some(pos) = self.low.iter().position(|&x| x == id) {
            self.low.remove(pos);
            self.high.push_back(id);
            return;
        }
        if self.seen.insert(id) {
            self.high.push_back(id);
        }
    }

    /// Pop the next id: high priority first, then low.
    pub fn next(&mut self) -> Option<i64> {
        self.high.pop_front().or_else(|| self.low.pop_front())
    }

    pub fn high_nonempty(&self) -> bool {
        !self.high.is_empty()
    }
}

impl Default for PreviewQueue {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test preview::tests::`
Expected: queue tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/preview.rs
git commit -m "feat(preview): high/low priority queue with dedup (TT-15)"
```

---

## Task 5: `Governor` — burst→trickle parallelism (pure logic)

**Files:**
- Modify: `src-tauri/src/preview.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:

```rust
    #[test]
    fn parallelism_bursts_within_window() {
        // 1s since last high demand, window 5s, high empty -> burst
        let p = compute_parallelism(1.0, false, 8, 2, 5.0);
        assert_eq!(p, 8);
    }

    #[test]
    fn parallelism_trickles_after_window() {
        // 6s since last high demand, window 5s, high empty -> trickle
        let p = compute_parallelism(6.0, false, 8, 2, 5.0);
        assert_eq!(p, 2);
    }

    #[test]
    fn parallelism_bursts_when_high_pending_even_after_window() {
        let p = compute_parallelism(60.0, true, 8, 2, 5.0);
        assert_eq!(p, 8);
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cd src-tauri && cargo test preview::tests::parallelism`
Expected: FAIL — `compute_parallelism` not defined.

- [ ] **Step 3: Implement the governor**

In `src-tauri/src/preview.rs`, add:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Pure parallelism decision: burst (all cores) while there is high-priority
/// work OR we are still inside the burst window after the last high demand;
/// otherwise trickle.
pub fn compute_parallelism(
    secs_since_high_demand: f64,
    high_pending: bool,
    burst: usize,
    trickle: usize,
    window_secs: f64,
) -> usize {
    if high_pending || secs_since_high_demand < window_secs {
        burst
    } else {
        trickle
    }
}

/// Tracks the last "high demand" moment and converts it into a live
/// parallelism target. `last_high_demand_ms` is millis since an arbitrary
/// epoch (the service's start Instant), stored atomically for cheap sharing.
pub struct Governor {
    start: Instant,
    last_high_demand_ms: AtomicU64,
    burst: usize,
    trickle: usize,
    window: Duration,
}

impl Governor {
    pub fn new(burst: usize, trickle: usize, window: Duration) -> Self {
        Self {
            start: Instant::now(),
            last_high_demand_ms: AtomicU64::new(0),
            burst,
            trickle,
            window,
        }
    }

    /// Record that high-priority demand just arrived (viewport request or a
    /// fresh folder scan), re-entering the burst window.
    pub fn signal_high_demand(&self) {
        let ms = self.start.elapsed().as_millis() as u64;
        self.last_high_demand_ms.store(ms, Ordering::Relaxed);
    }

    /// Current parallelism target given whether high-priority work is pending.
    pub fn parallelism(&self, high_pending: bool) -> usize {
        let now_ms = self.start.elapsed().as_millis() as u64;
        let last = self.last_high_demand_ms.load(Ordering::Relaxed);
        let secs = (now_ms.saturating_sub(last)) as f64 / 1000.0;
        compute_parallelism(secs, high_pending, self.burst, self.trickle, self.window.as_secs_f64())
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test preview::tests::parallelism`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/preview.rs
git commit -m "feat(preview): burst-then-trickle parallelism governor (TT-15)"
```

---

## Task 6: DB layer — `preview_path` column, queries, hash-change clearing

**Files:**
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/models/entities.rs`

- [ ] **Step 1: Add `preview_path` to the `Image` struct**

In `src-tauri/src/models/entities.rs`, in `struct Image` after line 19 (`pub thumbnail_path: Option<String>,`) add:

```rust
    pub preview_path: Option<String>,
```

- [ ] **Step 2: Add `preview_path` to schema and all image SELECTs + mapping**

In `src-tauri/src/db.rs`:

(a) In `BASE_SCHEMA` `CREATE TABLE images` (after `thumbnail_path TEXT,`, line 31) add:
```sql
    preview_path           TEXT,
```

(b) In `row_to_image` (after `thumbnail_path: r.get("thumbnail_path"),`, line 258) add:
```rust
        preview_path: r.get("preview_path"),
```

(c) In **every** image `SELECT` column list, add `preview_path` next to `thumbnail_path`. These are at lines ~424, ~434, ~448, and ~765 (the search query `i.thumbnail_path` → also add `i.preview_path`). Each list currently reads:
```
... mtime, thumbnail_path, semantic_analysis_done ...
```
change to:
```
... mtime, thumbnail_path, preview_path, semantic_analysis_done ...
```
(for the `i.`-prefixed query use `i.thumbnail_path, i.preview_path,`).

- [ ] **Step 3: Write failing tests for the new DB functions**

In `src-tauri/src/db.rs` tests module (near the existing `update_thumbnail_path_persists_and_is_readable` test ~line 1273), add:

```rust
    #[tokio::test]
    async fn update_preview_path_persists_and_is_readable() {
        let dir = std::env::temp_dir().join(format!("nebula_prevdb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = init_db(&dir).await.unwrap();
        let folder_id = insert_folder(&pool, "/tmp/f").await.unwrap();
        let image_id = insert_image(&pool, folder_id, "/tmp/f/a.jpg", "h", 1, 1).await.unwrap();

        update_preview_path(&pool, image_id, "/tmp/p_7.webp").await.unwrap();

        let img = get_image_by_id(&pool, image_id).await.unwrap().unwrap();
        assert_eq!(img.preview_path.as_deref(), Some("/tmp/p_7.webp"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn images_needing_preview_excludes_thumbnailed_and_deleted() {
        let dir = std::env::temp_dir().join(format!("nebula_needprev_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = init_db(&dir).await.unwrap();
        let fid = insert_folder(&pool, "/tmp/f").await.unwrap();
        let a = insert_image(&pool, fid, "/tmp/f/a.jpg", "h1", 1, 1).await.unwrap();
        let b = insert_image(&pool, fid, "/tmp/f/b.jpg", "h2", 1, 1).await.unwrap();
        // a already has a thumbnail -> excluded
        update_thumbnail_path(&pool, a, "/tmp/a.webp").await.unwrap();

        let need = images_needing_preview(&pool).await.unwrap();
        assert!(need.contains(&b));
        assert!(!need.contains(&a));
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 4: Run to confirm failure**

Run: `cd src-tauri && cargo test db::tests::update_preview_path db::tests::images_needing_preview`
Expected: FAIL — functions not defined.

- [ ] **Step 5: Implement the DB functions**

In `src-tauri/src/db.rs`, after `update_thumbnail_path` (line 419) add:

```rust
pub async fn update_preview_path(pool: &SqlitePool, image_id: i64, preview_path: &str) -> Result<()> {
    sqlx::query("UPDATE images SET preview_path = ? WHERE id = ?")
        .bind(preview_path)
        .bind(image_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Ids of non-deleted images that still lack an 800px thumbnail.
pub async fn images_needing_preview(pool: &SqlitePool) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT id FROM images
         WHERE thumbnail_path IS NULL AND deleted_at IS NULL
         ORDER BY COALESCE(date_taken, mtime) DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>("id")).collect())
}
```

- [ ] **Step 6: Clear both paths on hash change**

In `update_image_hash_changed` (line ~300), change the UPDATE SET clause to also null the cached previews so they regenerate. Change:
```rust
        "UPDATE images SET file_hash = ?, file_size = ?, mtime = ?,
         semantic_analysis_done = 0, subject_analysis_done = 0, embedding = NULL,
         updated_at = ?, deleted_at = NULL WHERE id = ?",
```
to:
```rust
        "UPDATE images SET file_hash = ?, file_size = ?, mtime = ?,
         semantic_analysis_done = 0, subject_analysis_done = 0, embedding = NULL,
         thumbnail_path = NULL, preview_path = NULL,
         updated_at = ?, deleted_at = NULL WHERE id = ?",
```

- [ ] **Step 7: Run tests**

Run: `cd src-tauri && cargo test db::tests::update_preview_path db::tests::images_needing_preview`
Expected: 2 passed. Also run `cargo test db::tests` to confirm no existing DB test regressed.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/models/entities.rs
git commit -m "feat(db): preview_path column, queries, hash-change clearing (TT-15)"
```

---

## Task 7: `PreviewHandle` and `PreviewService` (worker pool + backlog feeder)

Ties the queue, governor, decode/writers, DB, and Tauri emit together. `PreviewHandle` is the cloneable façade other modules use to enqueue; `PreviewService::start` spawns the dispatcher and backlog feeder and returns a handle.

**Files:**
- Modify: `src-tauri/src/preview.rs`

- [ ] **Step 1: Implement `PreviewHandle` and `PreviewService`**

In `src-tauri/src/preview.rs`, add:

```rust
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Cloneable façade used by the indexer and commands to feed the queue.
#[derive(Clone)]
pub struct PreviewHandle {
    queue: Arc<Mutex<PreviewQueue>>,
    governor: Arc<Governor>,
    notify: Arc<Notify>,
}

impl PreviewHandle {
    /// Enqueue a newly-indexed image at low priority and wake the dispatcher.
    pub fn enqueue_low(&self, id: i64) {
        let added = self.queue.lock().unwrap().enqueue_low(id);
        if added {
            self.notify.notify_one();
        }
    }

    /// Promote viewport ids to high priority and re-enter the burst window.
    pub fn prioritize(&self, ids: Vec<i64>) {
        {
            let mut q = self.queue.lock().unwrap();
            for id in ids {
                q.enqueue_high(id);
            }
        }
        self.governor.signal_high_demand();
        self.notify.notify_one();
    }
}

/// Owns the worker pool. Spawned once at startup.
pub struct PreviewService;

impl PreviewService {
    /// Start the dispatcher + backlog feeder. Returns a handle for enqueuing.
    pub fn start(
        pool: sqlx::SqlitePool,
        app: tauri::AppHandle,
        data_dir: std::path::PathBuf,
    ) -> PreviewHandle {
        let cores = num_cpus::get().max(2);
        let queue = Arc::new(Mutex::new(PreviewQueue::new()));
        let governor = Arc::new(Governor::new(cores, 2, Duration::from_secs(5)));
        let notify = Arc::new(Notify::new());
        let handle = PreviewHandle {
            queue: queue.clone(),
            governor: governor.clone(),
            notify: notify.clone(),
        };

        // Backlog feeder: enqueue everything that still needs a thumbnail.
        {
            let pool = pool.clone();
            let h = handle.clone();
            tokio::spawn(async move {
                match crate::db::images_needing_preview(&pool).await {
                    Ok(ids) => {
                        for id in ids {
                            h.enqueue_low(id);
                        }
                    }
                    Err(e) => eprintln!("[preview] backlog query failed: {e}"),
                }
            });
        }

        // Dispatcher: keep up to `governor.parallelism()` workers in flight.
        let in_flight = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                // Wait for work signals; also wake periodically so the trickle
                // tier keeps draining a large backlog without new signals.
                tokio::select! {
                    _ = notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                }

                loop {
                    let high_pending = queue.lock().unwrap().high_nonempty();
                    let par = governor.parallelism(high_pending);
                    if in_flight.load(Ordering::Relaxed) >= par {
                        break;
                    }
                    let id = match queue.lock().unwrap().next() {
                        Some(id) => id,
                        None => break,
                    };
                    in_flight.fetch_add(1, Ordering::Relaxed);

                    let pool = pool.clone();
                    let app = app.clone();
                    let data_dir = data_dir.clone();
                    let in_flight = in_flight.clone();
                    let notify = notify.clone();
                    tokio::spawn(async move {
                        process_image(&pool, &app, &data_dir, id).await;
                        in_flight.fetch_sub(1, Ordering::Relaxed);
                        // wake dispatcher to top up the pool
                        notify.notify_one();
                    });
                }
            }
        });

        handle
    }
}

/// Generate both tiers for one image (skips if deleted or gone), emitting
/// `image_updated` after each tier so the grid paints progressively.
async fn process_image(
    pool: &sqlx::SqlitePool,
    app: &tauri::AppHandle,
    data_dir: &std::path::Path,
    image_id: i64,
) {
    use tauri::Emitter;

    let image = match crate::db::get_image_by_id(pool, image_id).await {
        Ok(Some(i)) => i,
        Ok(None) => return,
        Err(e) => {
            eprintln!("[preview] lookup failed for {image_id}: {e}");
            return;
        }
    };
    if image.deleted_at.is_some() {
        return;
    }
    let src = std::path::PathBuf::from(&image.path);
    let data_dir = data_dir.to_path_buf();

    // Tier 1 — tiny instant preview.
    {
        let src = src.clone();
        let dd = data_dir.clone();
        let res = tokio::task::spawn_blocking(move || write_preview(&src, image_id, &dd)).await;
        match res {
            Ok(Ok(path)) => {
                if crate::db::update_preview_path(pool, image_id, &path.to_string_lossy())
                    .await
                    .is_ok()
                {
                    let _ = app.emit(
                        "image_updated",
                        crate::models::ImageUpdatedPayload { image_id },
                    );
                }
            }
            Ok(Err(e)) => eprintln!("[preview] tier1 failed for {image_id}: {e}"),
            Err(e) => eprintln!("[preview] tier1 panicked for {image_id}: {e}"),
        }
    }

    // Tier 2 — 800px thumbnail.
    {
        let res = tokio::task::spawn_blocking(move || write_thumbnail(&src, image_id, &data_dir)).await;
        match res {
            Ok(Ok(path)) => {
                if crate::db::update_thumbnail_path(pool, image_id, &path.to_string_lossy())
                    .await
                    .is_ok()
                {
                    let _ = app.emit(
                        "image_updated",
                        crate::models::ImageUpdatedPayload { image_id },
                    );
                }
            }
            Ok(Err(e)) => eprintln!("[preview] tier2 failed for {image_id}: {e}"),
            Err(e) => eprintln!("[preview] tier2 panicked for {image_id}: {e}"),
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src-tauri && cargo build`
Expected: builds. (`PreviewService`/`PreviewHandle` are not yet wired into `lib.rs`; that's Task 9.) If the compiler warns they are unused, that is expected until Task 9 — do not delete them.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/preview.rs
git commit -m "feat(preview): PreviewService worker pool + backlog feeder (TT-15)"
```

---

## Task 8: Remove thumbnail generation from the inference pipeline

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`

- [ ] **Step 1: Delete the `thumb_sem` and Stage-1 thumbnail block**

In `src-tauri/src/pipeline/mod.rs`:

(a) Delete line 120:
```rust
    let thumb_sem = Arc::new(tokio::sync::Semaphore::new(config.load_channel_depth));
```

(b) In the `Ok(Ok(x)) => {` arm of the decode-join loop (lines ~180–224), remove the entire early-thumbnail block so the arm becomes simply:
```rust
                Ok(Ok(x)) => {
                    decoded.push(x);
                }
```
(Delete everything from the `// Early Thumbnail Generation (Stage 1)` comment through the closing of the inner `tokio::spawn(async move { ... });`, i.e. the block that acquires `thumb_permit`, clones `d`, and writes the thumbnail. Keep `decoded.push(x);`.)

- [ ] **Step 2: Verify it compiles (no unused warnings for removed items)**

Run: `cd src-tauri && cargo build`
Expected: builds. `tauri::Emitter` is still used by the post-inference `app.emit` at line ~376, so the `use tauri::Emitter;` at the top of `run_pipeline` stays. If the compiler flags an unused import, remove only what it names.

- [ ] **Step 3: Run pipeline tests**

Run: `cd src-tauri && cargo test pipeline`
Expected: existing EMA tests still pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "refactor(pipeline): remove thumbnail gen; owned by preview subsystem (TT-15)"
```

---

## Task 9: Wire `PreviewService` into startup, indexer, and a command

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/indexer.rs`
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Give `Indexer` a `PreviewHandle`**

In `src-tauri/src/indexer.rs`:

(a) Add a field to `struct Indexer` (after `scan_mutex`, line 25):
```rust
    preview: crate::preview::PreviewHandle,
```

(b) Change `Indexer::init` signature (line 96) to accept the handle:
```rust
    pub async fn init(pool: SqlitePool, data_dir: PathBuf, app: AppHandle, preview: crate::preview::PreviewHandle) -> Result<Arc<Self>> {
```

(c) In the `Arc::new(Self { ... })` construction (line ~116), add `preview,` to the field list.

(d) In `process_file`, enqueue after a new image is inserted+enqueued. After the `db::enqueue_image(...)` block in the `None =>` arm (right after line 208, before the `image_added` emit), add:
```rust
                self.preview.enqueue_low(image_id);
```
And in the hash-changed branch (the `else` at line ~260, after `db::enqueue_image(&self.pool, existing.id)`), add:
```rust
                    self.preview.enqueue_low(existing.id);
```

- [ ] **Step 2: Build `PreviewService` and store the handle in `AppState`**

In `src-tauri/src/lib.rs`:

(a) Add a field to `struct AppState` (after `index`, line 26):
```rust
    pub preview: preview::PreviewHandle,
```

(b) In `setup`, after the `pool`/`index` are ready and **before** `indexer::init` (line 54), construct the service:
```rust
            let preview_handle = preview::PreviewService::start(
                pool.clone(),
                app.handle().clone(),
                data_dir.clone(),
            );
```

(c) Pass it into `indexer::init`:
```rust
            let indexer = tauri::async_runtime::block_on(
                indexer::Indexer::init(pool.clone(), data_dir.clone(), app.handle().clone(), preview_handle.clone())
            )?;
```

(d) Add `preview: preview_handle.clone(),` to the `app.manage(AppState { ... })` block.

- [ ] **Step 3: Add the `prioritize_previews` command**

In `src-tauri/src/commands.rs`, add (near `list_images`, after line 73):
```rust
#[tauri::command]
pub async fn prioritize_previews(
    image_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.preview.prioritize(image_ids);
    Ok(())
}
```

- [ ] **Step 4: Register the command**

In `src-tauri/src/lib.rs` `generate_handler!` (after `commands::list_images,`, line 104) add:
```rust
            commands::prioritize_previews,
```

- [ ] **Step 5: Build and run the full backend test suite**

Run: `cd src-tauri && cargo build && cargo test`
Expected: builds; all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/indexer.rs src-tauri/src/commands.rs
git commit -m "feat(preview): wire PreviewService into startup, indexer, command (TT-15)"
```

---

## Task 10: Integration test — decoupled pass completes without inference

**Files:**
- Modify: `src-tauri/src/preview.rs`

This verifies the core claim: previews are generated independently of `run_pipeline`. We exercise `process_image` directly against a temp DB + real files (it does not require the Tauri app, only an `AppHandle` for the emit — so we test the DB-visible result via the writers + DB functions rather than the full service loop, which needs a runtime app handle).

- [ ] **Step 1: Write the integration test**

Append to the `tests` module in `src-tauri/src/preview.rs`:

```rust
    #[tokio::test]
    async fn writers_plus_db_make_image_previewable_end_to_end() {
        let dir = std::env::temp_dir().join(format!("nebula_e2e_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = crate::db::init_db(&dir).await.unwrap();
        let fid = crate::db::insert_folder(&pool, "/tmp/f").await.unwrap();

        // Create a real source image on disk.
        let src = write_jpeg(1600, 1200);
        let id = crate::db::insert_image(
            &pool, fid, src.to_str().unwrap(), "h", 1, 1,
        ).await.unwrap();

        // Before: needs preview.
        assert!(crate::db::images_needing_preview(&pool).await.unwrap().contains(&id));

        // Tier 1 then tier 2, persisting paths as process_image would.
        let p = write_preview(&src, id, &dir).unwrap();
        crate::db::update_preview_path(&pool, id, p.to_str().unwrap()).await.unwrap();
        let t = write_thumbnail(&src, id, &dir).unwrap();
        crate::db::update_thumbnail_path(&pool, id, t.to_str().unwrap()).await.unwrap();

        // After: both paths set, no longer in the needs-preview set.
        let img = crate::db::get_image_by_id(&pool, id).await.unwrap().unwrap();
        assert!(img.preview_path.is_some());
        assert!(img.thumbnail_path.is_some());
        assert!(!crate::db::images_needing_preview(&pool).await.unwrap().contains(&id));

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_file(&src).ok();
    }
```

- [ ] **Step 2: Run it**

Run: `cd src-tauri && cargo test preview::tests::writers_plus_db`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/preview.rs
git commit -m "test(preview): end-to-end preview generation without inference (TT-15)"
```

---

## Task 11: Frontend — `preview_path`, grid fallback, viewport prioritization

**Files:**
- Modify: `src/app/models/models.ts`
- Modify: `src/app/services/photo.service.ts`
- Modify: `src/app/components/photo-grid/photo-grid.component.ts`
- Modify: `src/app/components/photo-grid/photo-grid.component.html`

- [ ] **Step 1: Add `preview_path` to the models**

In `src/app/models/models.ts`, add to both interfaces after their `thumbnail_path` lines (15 and 26):
```ts
  preview_path: string | null;
```

- [ ] **Step 2: Grid prefers thumbnail, falls back to preview**

In `src/app/components/photo-grid/photo-grid.component.ts`, change `thumbUrl` (line 48):
```ts
  protected thumbUrl(img: Image | SearchResult): string | null {
    return this.photos.thumbnailUrl(img.thumbnail_path ?? img.preview_path);
  }
```

- [ ] **Step 3: Add `prioritizePreviews` to the service**

In `src/app/services/photo.service.ts`, add a method (anywhere in the class, e.g. near the other `invoke` calls):
```ts
  /** Tell the backend to prioritize previews for the given image ids. */
  async prioritizePreviews(imageIds: number[]): Promise<void> {
    if (imageIds.length === 0) return;
    try {
      await invoke('prioritize_previews', { imageIds });
    } catch {
      // best-effort; previews still arrive via the background pass
    }
  }
```

- [ ] **Step 4: Report visible image ids via IntersectionObserver**

In `src/app/components/photo-grid/photo-grid.component.ts`, add an observer that watches the `.photo-cell[data-id]` elements (which already carry `data-id`) and debounces visible ids to the service. Replace the class body opening to add lifecycle + observer:

```ts
import {
  Component,
  Input,
  ChangeDetectionStrategy,
  inject,
  ElementRef,
  AfterViewInit,
  OnDestroy,
} from '@angular/core';
```

Add `implements AfterViewInit, OnDestroy` to the class declaration, and inside the class:

```ts
  private host = inject(ElementRef<HTMLElement>);
  private observer?: IntersectionObserver;
  private visible = new Set<number>();
  private flushTimer: ReturnType<typeof setTimeout> | null = null;

  ngAfterViewInit(): void {
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const id = Number((e.target as HTMLElement).dataset['id']);
          if (Number.isNaN(id)) continue;
          if (e.isIntersecting) this.visible.add(id);
          else this.visible.delete(id);
        }
        this.scheduleFlush();
      },
      { root: null, rootMargin: '400px', threshold: 0.01 }
    );
    this.observeCells();
  }

  private observeCells(): void {
    if (!this.observer) return;
    this.observer.disconnect();
    this.host.nativeElement
      .querySelectorAll<HTMLElement>('.photo-cell[data-id]')
      .forEach((el) => this.observer!.observe(el));
  }

  private scheduleFlush(): void {
    if (this.flushTimer) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      this.photos.prioritizePreviews([...this.visible]);
    }, 100);
  }

  ngOnDestroy(): void {
    this.observer?.disconnect();
    if (this.flushTimer) clearTimeout(this.flushTimer);
  }
```

Note: because the grid uses `@for` with `track`, re-observe when the image list changes. Add an `Input` setter wrapper — change the `@Input() images` line to:

```ts
  private _images: (Image | SearchResult)[] = [];
  @Input() set images(value: (Image | SearchResult)[]) {
    this._images = value;
    queueMicrotask(() => this.observeCells());
  }
  get images() { return this._images; }
```

- [ ] **Step 5: Build the frontend**

Run: `cd /home/pi/nebula && npm run build` (or the project's configured build, e.g. `ng build`).
Expected: TypeScript compiles with no errors.

- [ ] **Step 6: Commit**

```bash
git add src/app/models/models.ts src/app/services/photo.service.ts \
        src/app/components/photo-grid/photo-grid.component.ts \
        src/app/components/photo-grid/photo-grid.component.html
git commit -m "feat(ui): preview_path fallback + viewport-driven preview prioritization (TT-15)"
```

---

## Task 12: Manual verification

**Files:** none (runtime check)

- [ ] **Step 1: Wipe APP_DATA for a clean DB (alpha — no migration)**

Delete the app data dir (Tauri `app_data_dir`), e.g. on Linux `~/.local/share/<app-id>/` — specifically remove `nebula.db*` and the `thumbnails/` cache so the schema is recreated with `preview_path`.

- [ ] **Step 2: Run the app**

Use the `run` skill (or `cd /home/pi/nebula && npm run tauri dev`).

- [ ] **Step 3: Add a folder of ~400 images and observe**

Expected:
- Tiny previews paint across the grid within a few **seconds** (not minutes).
- Each cell sharpens to the 800px thumbnail shortly after.
- Scrolling to a new region pulls those images' previews forward (viewport priority).
- The inference badges (analysis dots) continue to resolve afterward — inference was not blocked.

- [ ] **Step 4: Update the Notion task status**

Per `nebula-notion-workflow`, set TT-15 to **Ready for review** and record the PR number once the PR is open. (Do not merge.)

---

## Self-Review Notes

- **Spec coverage:** decoupled subsystem (Tasks 7–9), two-tier decode (Tasks 2–3, 7), `preview_path` schema/no-migration (Task 6), priority queue + viewport command (Tasks 4, 9, 11), burst→trickle governor (Task 5, 7), event reuse `image_updated` (Task 7), grid fallback (Task 11), edge cases — deleted check + hash-change clearing (Tasks 6, 7), failure fall-through (Task 7 `process_image`), testing incl. integration (Task 10) and manual (Task 12). Pipeline thumbnail removal (Task 8).
- **Type consistency:** `PreviewHandle.enqueue_low`/`prioritize`, `PreviewQueue.enqueue_low`/`enqueue_high`/`next`/`high_nonempty`, `Governor.signal_high_demand`/`parallelism`, `compute_parallelism`, `decode_at_most`, `write_preview`, `write_thumbnail`, `preview_path_for`, `update_preview_path`, `images_needing_preview` are used consistently across tasks.
- **Lightbox→original** is intentionally out of scope (separate Notion task).
