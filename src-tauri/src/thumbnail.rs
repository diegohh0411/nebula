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
    // CatmullRom is a good speed/quality tradeoff for downscaling thumbnails.
    let thumb = img.resize(800, 800, image::imageops::FilterType::CatmullRom);
    thumb.save_with_format(dest_path, image::ImageFormat::WebP)?;
    Ok(())
}

/// Write a 200x200 square WebP face crop from an already-decoded image.
/// `bbox` is relative `(x, y, w, h)` in [0,1].
#[allow(dead_code)]
pub fn write_face_crop_from_image(
    img: &DynamicImage,
    dest_path: &std::path::Path,
    bbox: (f64, f64, f64, f64),
) -> Result<()> {
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let (img_w, img_h) = (img.width() as f64, img.height() as f64);
    let x = (bbox.0 * img_w).max(0.0).min(img_w - 1.0) as u32;
    let y = (bbox.1 * img_h).max(0.0).min(img_h - 1.0) as u32;
    let max_w = img_w - x as f64;
    let max_h = img_h - y as f64;
    let w = (bbox.2 * img_w).min(max_w).max(1.0) as u32;
    let h = (bbox.3 * img_h).min(max_h).max(1.0) as u32;

    let face = img.crop_imm(x, y, w, h);
    let face_resized = face.resize_exact(200, 200, image::imageops::FilterType::CatmullRom);
    face_resized.save_with_format(dest_path, image::ImageFormat::WebP)?;
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

    #[test]
    fn face_crop_from_image_is_square_200() {
        let img = red(1000, 800);
        let dest = std::env::temp_dir().join(format!("nebula_face_{}.webp", std::process::id()));
        // bbox in relative coords: x=0.25, y=0.25, w=0.5, h=0.5
        write_face_crop_from_image(&img, &dest, (0.25, 0.25, 0.5, 0.5)).unwrap();
        let loaded = image::open(&dest).unwrap();
        assert_eq!(loaded.width(), 200);
        assert_eq!(loaded.height(), 200);
        std::fs::remove_file(&dest).ok();
    }
}
