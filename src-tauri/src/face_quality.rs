//! Pure scoring helpers for choosing the best face crop.
//! All outputs are normalized to 0.0..=1.0 so they can be linearly combined.

use image::DynamicImage;

/// Detector landmark order (SCRFD/buffalo): left_eye, right_eye, nose, left_mouth, right_mouth.
/// Coordinates are relative to the full image (0.0..1.0), matching `DetectedFace.landmarks`.
/// Returns 0.0..1.0; higher = more level and front-facing. Missing/!=5 landmarks -> neutral 0.5.
pub fn frontality(landmarks: Option<&[(f32, f32)]>) -> f32 {
    let lms = match landmarks {
        Some(l) if l.len() == 5 => l,
        _ => return 0.5,
    };
    let (lex, ley) = lms[0];
    let (rex, rey) = lms[1];
    let (nx, _) = lms[2];

    let dx = rex - lex;
    let dy = rey - ley;
    let eye_dist = (dx * dx + dy * dy).sqrt().max(1e-6);

    // Roll: eyes should be level. cos(angle) -> 1.0 when horizontal.
    let roll_score = (dx / eye_dist).abs().clamp(0.0, 1.0);

    // Yaw proxy: nose centered between the eyes.
    let eye_mid_x = (lex + rex) / 2.0;
    // Normalize the nose's horizontal offset by half the eye distance: for a frontal
    // face the nose sits at the eye midpoint (offset 0 -> score 1), and a deviation of
    // half the eye distance is treated as fully turned (offset 1 -> score 0).
    let yaw_offset = (nx - eye_mid_x).abs() / (0.5 * eye_dist);
    let yaw_score = (1.0 - yaw_offset).clamp(0.0, 1.0);

    (0.5 * roll_score + 0.5 * yaw_score).clamp(0.0, 1.0)
}

/// Variance-of-Laplacian sharpness over the whole supplied (already-cropped) region,
/// soft-normalized to 0.0..1.0 via a knee constant. Higher = sharper.
pub fn sharpness(region: &DynamicImage) -> f32 {
    const KNEE: f32 = 500.0; // var-of-Laplacian knee; var==KNEE -> 0.5
    let gray = region.to_luma8();
    let (w, h) = (gray.width() as i32, gray.height() as i32);
    if w < 3 || h < 3 {
        return 0.0;
    }
    let at = |x: i32, y: i32| gray.get_pixel(x as u32, y as u32)[0] as f32;
    let mut sum = 0.0f32;
    let mut sum_sq = 0.0f32;
    let mut n = 0.0f32;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            // 4-neighbour Laplacian
            let lap = at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1) - 4.0 * at(x, y);
            sum += lap;
            sum_sq += lap * lap;
            n += 1.0;
        }
    }
    if n == 0.0 {
        return 0.0;
    }
    let mean = sum / n;
    let var = (sum_sq / n) - mean * mean;
    (var / (var + KNEE)).clamp(0.0, 1.0)
}

/// Weighted composite of detector confidence, frontality and sharpness.
/// Inputs are each 0.0..1.0; output is 0.0..1.0.
pub fn composite(det_score: f32, frontality: f32, sharpness: f32) -> f32 {
    const W_DET: f32 = 0.40;
    const W_FRONT: f32 = 0.35;
    const W_SHARP: f32 = 0.25;
    (W_DET * det_score.clamp(0.0, 1.0)
        + W_FRONT * frontality.clamp(0.0, 1.0)
        + W_SHARP * sharpness.clamp(0.0, 1.0))
    .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, GrayImage, Luma};

    #[test]
    fn frontality_neutral_when_landmarks_missing() {
        assert_eq!(frontality(None), 0.5);
        assert_eq!(frontality(Some(&vec![(0.0, 0.0)])), 0.5); // wrong count
    }

    #[test]
    fn frontality_high_for_level_centered_face() {
        // eyes level (same y), nose centered between them
        let lms = vec![(0.4, 0.5), (0.6, 0.5), (0.5, 0.6), (0.43, 0.7), (0.57, 0.7)];
        let f = frontality(Some(&lms));
        assert!(f > 0.95, "level centered face should score high, got {f}");
    }

    #[test]
    fn frontality_low_for_turned_face() {
        // nose far off-center (turned head), eyes still level
        let lms = vec![(0.4, 0.5), (0.6, 0.5), (0.40, 0.6), (0.4, 0.7), (0.5, 0.7)];
        let turned = frontality(Some(&lms));
        let centered = frontality(Some(&vec![
            (0.4, 0.5), (0.6, 0.5), (0.5, 0.6), (0.43, 0.7), (0.57, 0.7),
        ]));
        assert!(turned < centered, "turned ({turned}) must score below centered ({centered})");
    }

    #[test]
    fn sharpness_higher_for_high_contrast_than_flat() {
        // Flat gray image -> ~0 sharpness
        let flat = DynamicImage::ImageLuma8(GrayImage::from_pixel(32, 32, Luma([128])));
        // Checkerboard -> high Laplacian variance
        let mut checker = GrayImage::new(32, 32);
        for (x, y, p) in checker.enumerate_pixels_mut() {
            *p = Luma([if (x + y) % 2 == 0 { 0 } else { 255 }]);
        }
        let checker = DynamicImage::ImageLuma8(checker);
        assert!(sharpness(&checker) > sharpness(&flat));
        assert!(sharpness(&flat) < 0.05, "flat image should be near zero");
    }

    #[test]
    fn composite_monotonic_and_bounded() {
        let low = composite(0.1, 0.1, 0.1);
        let high = composite(0.9, 0.9, 0.9);
        assert!(high > low);
        assert!((0.0..=1.0).contains(&composite(2.0, -1.0, 0.5)));
    }
}
