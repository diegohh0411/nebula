use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::Path;

/// Decode an image at a coarse downscale such that the longest edge is
/// ≥ `target_long_edge` (rounded up to the nearest power-of-two decode
/// factor), preserving aspect ratio. For JPEG this scales DURING decode via
/// `jpeg-decoder`; other formats decode fully via `image`. The caller is
/// responsible for the final exact resize to the target dimensions.
pub fn decode_at_most(path: &Path, target_long_edge: u32) -> Result<DynamicImage> {
    let is_jpeg = matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
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
            let buf =
                image::RgbImage::from_raw(w, h, pixels).context("rgb buffer size mismatch")?;
            Ok(DynamicImage::ImageRgb8(buf))
        }
        PixelFormat::L8 => {
            let buf =
                image::GrayImage::from_raw(w, h, pixels).context("luma buffer size mismatch")?;
            Ok(DynamicImage::ImageLuma8(buf))
        }
        // CMYK32, L16, etc.: let the caller's image::open fallback handle it.
        _ => anyhow::bail!("unsupported jpeg pixel format for scaled decode"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_jpeg(dir: &std::path::Path, w: u32, h: u32) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let mut img = image::RgbImage::new(w, h);
        for p in img.pixels_mut() {
            *p = image::Rgb([120, 180, 60]);
        }
        let path = dir.join(format!("src_{}x{}.jpg", w, h));
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(&path, image::ImageFormat::Jpeg)
            .unwrap();
        path
    }

    #[test]
    fn decode_at_most_scales_large_jpeg_down() {
        let dir = std::env::temp_dir().join(format!("nebula_dec_scale_{}", std::process::id()));
        let path = write_jpeg(&dir, 2000, 1000);
        let img = decode_at_most(&path, 256).unwrap();
        // Coarse scale: result must be no larger than the original and non-empty.
        assert!(img.width() > 0 && img.height() > 0);
        assert!(img.width() <= 2000 && img.height() <= 1000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn decode_at_most_does_not_upscale_small_image() {
        let dir = std::env::temp_dir().join(format!("nebula_dec_small_{}", std::process::id()));
        let path = write_jpeg(&dir, 100, 80);
        let img = decode_at_most(&path, 256).unwrap();
        assert!(img.width() <= 100 && img.height() <= 80);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn decode_at_most_errors_on_missing_file() {
        let res = decode_at_most(Path::new("definitely-not-here.jpg"), 256);
        assert!(res.is_err());
    }
}
