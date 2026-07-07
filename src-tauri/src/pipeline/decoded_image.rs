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
    pub image_id: i64,
    pub full: Arc<DynamicImage>,
}

/// Upper bound on the decoded image's long edge. Every downstream consumer
/// (SigLIP embed ~224–256px, face detector ~640px, sharpness crop) works on a
/// far smaller resize, so decoding above this wastes memory and resize cost.
/// Named so it can be tuned. See docs/superpowers/specs/2026-07-07-bounded-decode-inference-pipeline-design.md.
pub const DECODE_MAX_LONG_EDGE: u32 = 2048;

/// Decode an image from disk once, bounded to `DECODE_MAX_LONG_EDGE` on the
/// long edge. CPU/IO bound — call inside `spawn_blocking` or a rayon task,
/// never on the async runtime.
pub fn load_decoded(image_id: i64, path: &Path) -> Result<DecodedImage> {
    // Coarse DCT-scaled decode for JPEG (near-free), full decode for other
    // formats. `decode_at_most` only rounds JPEG to a power-of-two whose long
    // edge is >= the bound, so it may return up to ~2x the bound.
    let img = crate::media::decode::decode_at_most(path, DECODE_MAX_LONG_EDGE)
        .with_context(|| format!("failed to decode image at {}", path.display()))?;
    // Clamp to the exact bound. Images already within it are untouched
    // (no resize pass, and decode_at_most never upscales).
    let full = if img.width().max(img.height()) > DECODE_MAX_LONG_EDGE {
        img.resize(
            DECODE_MAX_LONG_EDGE,
            DECODE_MAX_LONG_EDGE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
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

    #[test]
    fn load_decoded_bounds_oversized_to_2048_long_edge() {
        // 4000x2000 JPEG — long edge well above the 2048 bound, aspect ratio 2:1.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nebula_bound_{}.jpg", std::process::id()));
        let img = image::RgbImage::from_pixel(4000, 2000, image::Rgb([100, 150, 200]));
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(&path, image::ImageFormat::Jpeg)
            .unwrap();

        let decoded = load_decoded(7, &path).unwrap();
        let long = decoded.full.width().max(decoded.full.height());
        assert!(long <= 2048, "long edge {long} exceeds the 2048 bound");
        // Aspect ratio (2:1) preserved through the bound.
        assert_eq!(decoded.full.width(), decoded.full.height() * 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_decoded_passes_small_image_through_untouched() {
        // 800x600 PNG — already within the bound; must be returned unchanged.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nebula_small_{}.png", std::process::id()));
        let img = image::RgbImage::from_pixel(800, 600, image::Rgb([10, 20, 30]));
        image::DynamicImage::ImageRgb8(img).save(&path).unwrap();

        let decoded = load_decoded(9, &path).unwrap();
        assert_eq!(decoded.full.width(), 800);
        assert_eq!(decoded.full.height(), 600);

        std::fs::remove_file(&path).ok();
    }
}
