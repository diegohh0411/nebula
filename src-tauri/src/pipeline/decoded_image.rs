use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::Path;
use std::sync::Arc;

/// An image decoded exactly once, shared (read-only) across all pipeline stages.
///
/// `full` is the originally-decoded image, reused for thumbnail and face crops.
/// Embedding and face detection both read from `full` — the file is never
/// re-opened after Stage 1.
#[derive(Clone)]
pub struct DecodedImage {
    #[allow(dead_code)]
    pub image_id: i64,
    pub full: Arc<DynamicImage>,
}

/// Decode an image from disk once. CPU/IO bound — call inside `spawn_blocking`
/// or a rayon task, never on the async runtime.
pub fn load_decoded(image_id: i64, path: &Path) -> Result<DecodedImage> {
    let full = image::open(path)
        .with_context(|| format!("failed to decode image at {}", path.display()))?;
    Ok(DecodedImage {
        image_id,
        full: Arc::new(full),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_decoded_decodes_once_and_keeps_dimensions() {
        // 2x2 red PNG written to a temp file.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nebula_decode_{}.png", std::process::id()));
        let mut img = image::RgbImage::new(2, 2);
        for p in img.pixels_mut() {
            *p = image::Rgb([255, 0, 0]);
        }
        image::DynamicImage::ImageRgb8(img).save(&path).unwrap();

        let decoded = load_decoded(42, &path).unwrap();
        assert_eq!(decoded.image_id, 42);
        assert_eq!(decoded.full.width(), 2);
        assert_eq!(decoded.full.height(), 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_decoded_errors_on_missing_file() {
        let res = load_decoded(1, Path::new("definitely-not-here.jpg"));
        assert!(res.is_err());
    }
}
