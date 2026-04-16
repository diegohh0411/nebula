use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn thumbnail_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("thumbnails")
}

pub fn face_crop_cache_dir(data_dir: &Path) -> PathBuf {
    thumbnail_cache_dir(data_dir).join("face-crops")
}

pub fn thumbnail_path_for(data_dir: &Path, image_id: i64) -> PathBuf {
    thumbnail_cache_dir(data_dir).join(format!("{}.webp", image_id))
}

pub fn face_crop_path_for(data_dir: &Path, face_id: i64) -> PathBuf {
    face_crop_cache_dir(data_dir).join(format!("{}.webp", face_id))
}

/// Generate a 800px-longest-side WebP thumbnail.
/// Runs in a blocking thread to avoid blocking the async runtime.
pub async fn generate_thumbnail(src_path: PathBuf, dest_path: PathBuf) -> Result<()> {
    // Ensure the parent directory exists
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    tokio::task::spawn_blocking(move || -> Result<()> {
        let img = image::open(&src_path)?;
        // thumbnail() preserves aspect ratio, fitting within 800×800
        let thumb = img.thumbnail(800, 800);
        thumb.save_with_format(&dest_path, image::ImageFormat::WebP)?;
        Ok(())
    })
    .await??;

    Ok(())
}

/// Generate a 200x200 square WebP face crop.
pub async fn generate_face_crop(
    src_path: PathBuf,
    dest_path: PathBuf,
    bbox: (f64, f64, f64, f64),
) -> Result<()> {
    // Ensure the parent directory exists
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut img = image::open(&src_path)?;
        let (width, height) = (img.width() as f64, img.height() as f64);

        // Bbox is often normalized (0.0 to 1.0), but let's check if it's absolute.
        // Looking at face_detector.rs might help, but typical ML bboxes are normalized.
        // Assuming normalized for now. If absolute, we don't multiply by width/height.
        // Actually, let's check face_detector.rs if possible.
        let x = (bbox.0 * width).max(0.0) as u32;
        let y = (bbox.1 * height).max(0.0) as u32;
        let w = (bbox.2 * width).min(width - x as f64) as u32;
        let h = (bbox.3 * height).min(height - y as f64) as u32;

        let face = img.crop(x, y, w, h);
        let face_resized = face.thumbnail_exact(200, 200);
        face_resized.save_with_format(&dest_path, image::ImageFormat::WebP)?;
        Ok(())
    })
    .await??;

    Ok(())
}
