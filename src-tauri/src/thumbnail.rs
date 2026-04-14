use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn thumbnail_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("thumbnails")
}

pub fn thumbnail_path_for(data_dir: &Path, image_id: i64) -> PathBuf {
    thumbnail_cache_dir(data_dir).join(format!("{}.jpg", image_id))
}

/// Generate a 400px-longest-side JPEG thumbnail.
/// Runs in a blocking thread to avoid blocking the async runtime.
pub async fn generate_thumbnail(src_path: PathBuf, dest_path: PathBuf) -> Result<()> {
    // Ensure the parent directory exists
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    tokio::task::spawn_blocking(move || -> Result<()> {
        let img = image::open(&src_path)?;
        // thumbnail() preserves aspect ratio, fitting within 400×400
        let thumb = img.thumbnail(400, 400);
        thumb.save_with_format(&dest_path, image::ImageFormat::Jpeg)?;
        Ok(())
    })
    .await??;

    Ok(())
}
