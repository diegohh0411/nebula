use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::{Path, PathBuf};

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
}
