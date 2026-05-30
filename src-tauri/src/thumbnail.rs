use anyhow::Result;
use std::path::{Path, PathBuf};
use image::DynamicImage;

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
        let (img_w, img_h) = (img.width() as f64, img.height() as f64);

        let x = (bbox.0 * img_w).max(0.0).min(img_w - 1.0) as u32;
        let y = (bbox.1 * img_h).max(0.0).min(img_h - 1.0) as u32;
        let max_w = img_w - x as f64;
        let max_h = img_h - y as f64;
        let w = (bbox.2 * img_w).min(max_w).max(1.0) as u32;
        let h = (bbox.3 * img_h).min(max_h).max(1.0) as u32;

        let face = img.crop(x, y, w, h);
        let face_resized = face.thumbnail_exact(200, 200);
        face_resized.save_with_format(&dest_path, image::ImageFormat::WebP)?;
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
