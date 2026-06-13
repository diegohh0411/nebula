use image::DynamicImage;
use ndarray::Array4;

/// Resize `img` to `size`x`size` and write it into `dst` at batch index `b`,
/// normalized to [-1, 1] in CHW order. `dst` must have shape (B, 3, size, size).
pub fn fill_pixel_values(dst: &mut Array4<f32>, b: usize, img: &DynamicImage, size: usize) {
    // Triangle is markedly faster than Lanczos3 with negligible effect on
    // embeddings at 224-256px inputs.
    let resized = img.resize_exact(
        size as u32,
        size as u32,
        image::imageops::FilterType::Triangle,
    );
    let rgb = resized.to_rgb8();
    let raw = rgb.as_raw(); // tightly packed RGBRGB..., row-major
    let plane = size * size;
    let base = b * 3 * plane;
    let data = dst.as_slice_mut().expect("contiguous Array4");
    for i in 0..plane {
        let r = raw[i * 3] as f32;
        let g = raw[i * 3 + 1] as f32;
        let bl = raw[i * 3 + 2] as f32;
        data[base + i] = (r / 255.0 - 0.5) / 0.5;
        data[base + plane + i] = (g / 255.0 - 0.5) / 0.5;
        data[base + 2 * plane + i] = (bl / 255.0 - 0.5) / 0.5;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_matches_manual_normalization_for_solid_color() {
        // Solid mid-gray image → known normalized value.
        let mut img = image::RgbImage::new(8, 8);
        for p in img.pixels_mut() {
            *p = image::Rgb([128, 128, 128]);
        }
        let dimg = DynamicImage::ImageRgb8(img);

        let size = 4;
        let mut dst = Array4::<f32>::zeros((1, 3, size, size));
        fill_pixel_values(&mut dst, 0, &dimg, size);

        let expected = (128.0f32 / 255.0 - 0.5) / 0.5;
        for c in 0..3 {
            for y in 0..size {
                for x in 0..size {
                    assert!((dst[[0, c, y, x]] - expected).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn fill_writes_into_correct_batch_slot() {
        let mut img = image::RgbImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = image::Rgb([255, 0, 0]);
        }
        let dimg = DynamicImage::ImageRgb8(img);

        let size = 2;
        let mut dst = Array4::<f32>::zeros((2, 3, size, size));
        fill_pixel_values(&mut dst, 1, &dimg, size);

        // batch 0 untouched (zeros), batch 1 has red channel = +1.0
        assert_eq!(dst[[0, 0, 0, 0]], 0.0);
        assert!((dst[[1, 0, 0, 0]] - 1.0).abs() < 1e-6); // (255/255-0.5)/0.5 = 1.0
        assert!((dst[[1, 2, 0, 0]] - (-1.0)).abs() < 1e-6); // blue = 0 → -1.0
    }
}
