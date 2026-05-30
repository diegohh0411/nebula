use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::{Path, PathBuf};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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

    /// Returns true if there are high-priority items pending.
    pub fn high_nonempty(&self) -> bool {
        !self.high.is_empty()
    }
}

impl Default for PreviewQueue {
    fn default() -> Self { Self::new() }
}

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

/// Decode an image at a coarse downscale such that the longest edge is
/// ≤ `target_long_edge`, preserving aspect ratio. For JPEG this scales
/// DURING decode (power-of-two factor) via `jpeg-decoder`; other formats
/// decode fully via `image`. The caller is responsible for the final exact
/// resize to the target dimensions.
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

/// Tier 1: decode coarsely, resize to <=256px longest edge, write WebP.
pub fn write_preview(src: &Path, image_id: i64, data_dir: &Path) -> Result<PathBuf> {
    let img = decode_at_most(src, 256)?;
    let small = if img.width() > 256 || img.height() > 256 {
        img.resize(256, 256, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let dest = crate::thumbnail::preview_path_for(data_dir, image_id);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
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

    #[test]
    fn enqueue_high_then_low_ignores_low() {
        let mut q = PreviewQueue::new();
        q.enqueue_high(1);
        assert!(!q.enqueue_low(1)); // already seen, returns false
        assert_eq!(q.next(), Some(1)); // only one copy
        assert_eq!(q.next(), None);
    }

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
}
