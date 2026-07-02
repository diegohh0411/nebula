//! Pure geometry helpers for matching face detections across a model switch.

/// Intersection-over-union of two axis-aligned boxes in relative `(x, y, w, h)`
/// form (the same convention as `faces.bbox_x/y/w/h`). Returns `0.0` for
/// non-overlapping, merely-touching, or degenerate (zero-area) boxes.
pub fn iou(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    if aw <= 0.0 || ah <= 0.0 || bw <= 0.0 || bh <= 0.0 {
        return 0.0;
    }
    let (ax2, ay2) = (ax + aw, ay + ah);
    let (bx2, by2) = (bx + bw, by + bh);

    let ix1 = ax.max(bx);
    let iy1 = ay.max(by);
    let ix2 = ax2.min(bx2);
    let iy2 = ay2.min(by2);

    let iw = (ix2 - ix1).max(0.0);
    let ih = (iy2 - iy1).max(0.0);
    let inter = iw * ih;
    if inter <= 0.0 {
        return 0.0;
    }

    let union = aw * ah + bw * bh - inter;
    if union <= 0.0 {
        return 0.0;
    }
    inter / union
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_boxes_have_iou_one() {
        let a = (0.1, 0.1, 0.2, 0.2);
        assert!((iou(a, a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disjoint_boxes_have_iou_zero() {
        let a = (0.0, 0.0, 0.1, 0.1);
        let b = (0.5, 0.5, 0.1, 0.1);
        assert_eq!(iou(a, b), 0.0);
    }

    #[test]
    fn half_overlap_matches_known_value() {
        // a: [0,1]x[0,1] area=1; b: [0.5,1.5]x[0,1] area=1; intersection: [0.5,1]x[0,1] area=0.5
        // union = 1 + 1 - 0.5 = 1.5; iou = 0.5/1.5 = 1/3
        let a = (0.0, 0.0, 1.0, 1.0);
        let b = (0.5, 0.0, 1.0, 1.0);
        assert!((iou(a, b) - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn zero_area_box_has_iou_zero() {
        let a = (0.1, 0.1, 0.0, 0.2);
        let b = (0.1, 0.1, 0.2, 0.2);
        assert_eq!(iou(a, b), 0.0);
    }

    #[test]
    fn touching_edges_have_iou_zero() {
        // Boxes share only a boundary line -> zero-area intersection.
        let a = (0.0, 0.0, 0.5, 0.5);
        let b = (0.5, 0.0, 0.5, 0.5);
        assert_eq!(iou(a, b), 0.0);
    }
}