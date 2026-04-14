use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn thumbnail_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("thumbnails")
}

pub fn thumbnail_path_for(data_dir: &Path, image_id: i64) -> PathBuf {
    thumbnail_cache_dir(data_dir).join(format!("{}.webp", image_id))
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
