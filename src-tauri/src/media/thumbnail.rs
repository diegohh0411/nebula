use anyhow::Result;
use std::path::{Path, PathBuf};
use image::DynamicImage;

pub use crate::platform::paths::{thumbnail_cache_dir, face_crop_cache_dir};

pub fn thumbnail_path_for(data_dir: &Path, image_id: i64) -> PathBuf {
    thumbnail_cache_dir(data_dir).join(format!("{}.webp", image_id))
}

pub fn preview_path_for(data_dir: &Path, image_id: i64) -> PathBuf {
    thumbnail_cache_dir(data_dir).join(format!("{}_p.webp", image_id))
}

pub fn face_crop_path_for(data_dir: &Path, face_id: i64) -> PathBuf {
    face_crop_cache_dir(data_dir).join(format!("{}.webp", face_id))
}

/// Generate a 320x320 square WebP face crop: bbox expanded by a margin,
/// squared and centered on the face, clamped to image bounds, no aspect distortion.
pub async fn generate_face_crop(
    src_path: PathBuf,
    dest_path: PathBuf,
    bbox: (f64, f64, f64, f64),
) -> Result<()> {
    const OUT: u32 = 320;
    const MARGIN: f64 = 0.4; // 40% padding around the face

    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    tokio::task::spawn_blocking(move || -> Result<()> {
        let img = image::open(&src_path)?;
        let (iw, ih) = (img.width() as f64, img.height() as f64);

        // bbox is relative (x, y, w, h). Compute absolute center + a padded square side.
        let cx = (bbox.0 + bbox.2 / 2.0) * iw;
        let cy = (bbox.1 + bbox.3 / 2.0) * ih;
        let face_px = (bbox.2 * iw).max(bbox.3 * ih);
        // Side cannot exceed the image's smaller dimension.
        let side = (face_px * (1.0 + 2.0 * MARGIN)).min(iw).min(ih).max(1.0);

        // Top-left, clamped so the square stays inside the image.
        let x = (cx - side / 2.0).clamp(0.0, iw - side);
        let y = (cy - side / 2.0).clamp(0.0, ih - side);

        let square = img.crop_imm(x.round() as u32, y.round() as u32, side.round() as u32, side.round() as u32);
        // Square -> square keeps aspect ratio (no squish).
        let resized = square.resize_exact(OUT, OUT, image::imageops::FilterType::CatmullRom);
        resized.save_with_format(&dest_path, image::ImageFormat::WebP)?;
        Ok(())
    })
    .await??;

    Ok(())
}

/// Write an 800px-longest-side WebP thumbnail from an already-decoded image.
pub fn write_thumbnail_from_image(img: &DynamicImage, dest_path: &std::path::Path) -> Result<()> {
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let thumb = if img.width() > 800 || img.height() > 800 {
        img.resize(800, 800, image::imageops::FilterType::CatmullRom)
    } else {
        img.clone()
    };
    thumb.save_with_format(dest_path, image::ImageFormat::WebP)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(w: u32, h: u32) -> image::DynamicImage {
        let mut img = image::RgbImage::new(w, h);
        for p in img.pixels_mut() {
            *p = image::Rgb([200, 50, 50]);
        }
        image::DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn thumbnail_from_image_fits_within_box_and_writes_file() {
        let img = red(1600, 1200);
        let dest = std::env::temp_dir().join(format!("nebula_thumb_{}.webp", std::process::id()));
        write_thumbnail_from_image(&img, &dest).unwrap();
        let loaded = image::open(&dest).unwrap();
        assert!(loaded.width() <= 800 && loaded.height() <= 800);
        assert!(loaded.width() == 800 || loaded.height() == 800);
        std::fs::remove_file(&dest).ok();
    }

    /// thumbnail_path_for must return a path nested under data_dir/thumbnails,
    /// named after the image id, and be deterministic for the same inputs.
    #[test]
    fn thumbnail_path_for_is_under_data_dir_and_deterministic() {
        let data_dir = std::env::temp_dir().join(format!("nebula_test_data_{}", std::process::id()));
        let image_id: i64 = 42;

        let path = thumbnail_path_for(&data_dir, image_id);

        // Must be located under data_dir
        assert!(
            path.starts_with(&data_dir),
            "thumbnail path {:?} should be under data_dir {:?}",
            path,
            data_dir
        );
        // Must carry the image id in the filename
        let file_name = path.file_name().unwrap().to_string_lossy();
        assert!(
            file_name.contains("42"),
            "filename {:?} should contain the image id 42",
            file_name
        );
        // Must be deterministic: same inputs → same output
        assert_eq!(path, thumbnail_path_for(&data_dir, image_id));
        // Different ids must not collide
        assert_ne!(path, thumbnail_path_for(&data_dir, 99));
    }

    #[tokio::test]
    async fn face_crop_is_square_320_and_within_bounds() {
        // 400x300 image, a non-square bbox in the middle.
        let img = red(400, 300);
        let src = std::env::temp_dir().join(format!("nebula_src_{}.png", std::process::id()));
        img.save(&src).unwrap();
        let dest = std::env::temp_dir().join(format!("nebula_crop_{}.webp", std::process::id()));

        // bbox: x=0.4,y=0.4,w=0.2,h=0.3 (taller than wide) — must NOT be squished.
        generate_face_crop(src.clone(), dest.clone(), (0.4, 0.4, 0.2, 0.3)).await.unwrap();

        let out = image::open(&dest).unwrap();
        assert_eq!(out.width(), 320, "crop width must be 320");
        assert_eq!(out.height(), 320, "crop must be square 320");
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&dest).ok();
    }

    /// write_thumbnail_from_image must actually create the file on disk
    /// at the path computed by thumbnail_path_for, mirroring Stage-1 of the
    /// early-preview pipeline.
    #[test]
    fn early_preview_writes_thumbnail_file() {
        let data_dir = std::env::temp_dir().join(format!(
            "nebula_test_early_preview_{}_{}", std::process::id(), 1u64
        ));
        let image_id: i64 = 7;
        let img = red(400, 300);

        let thumb_path = thumbnail_path_for(&data_dir, image_id);
        write_thumbnail_from_image(&img, &thumb_path).unwrap();

        assert!(
            thumb_path.exists(),
            "thumbnail file should exist at {:?} after write_thumbnail_from_image",
            thumb_path
        );

        // Clean up
        std::fs::remove_file(&thumb_path).ok();
        std::fs::remove_dir_all(&data_dir).ok();
    }
}
