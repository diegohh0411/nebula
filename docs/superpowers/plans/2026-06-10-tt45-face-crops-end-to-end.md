# TT-45 Face Crops End-to-End Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate well-framed face-crop profile pictures eagerly during import, selected and upgraded by a composite quality score (confidence + frontality + sharpness), served without visible delay.

**Architecture:** The face detector already produces a per-face confidence `score` and 5-point `landmarks` that the pipeline currently discards. We capture them, compute a `quality_score` per face at detection time (sharpness measured on the decoded image inside the face actor; frontality from landmarks; combined with detector confidence), persist it on the `faces` row, select each subject's profile crop by highest `quality_score` with upgrade-but-never-revert semantics, generate the crop eagerly after each clustering pass with corrected framing (margin + square, no aspect distortion, 320px WebP), and lazy-load on the grid.

**Tech Stack:** Rust (Tauri backend, sqlx/SQLite, `image` crate, `face_id` 0.4.1), Angular frontend.

**Spec:** `docs/superpowers/specs/2026-06-10-tt45-face-crops-end-to-end-design.md`

**Note on schema:** App is in alpha; the DB will be wiped. We edit the original `BASE_SCHEMA` `CREATE TABLE faces` directly — **do not add a versioned migration**.

---

## File structure

- `src-tauri/src/db.rs` — add `det_score`/`quality_score` to base `faces` schema; widen `insert_face`; change `auto_assign_missing_thumbnails` to order by quality; add `upgrade_subject_thumbnails`; add `get_face_with_image`.
- `src-tauri/src/face_quality.rs` — **new** pure-function module: `frontality`, `sharpness`, `composite`.
- `src-tauri/src/lib.rs` — register the new `face_quality` module.
- `src-tauri/src/pipeline/face_actor.rs` — stop discarding `score`/`landmarks`; compute sharpness on the decoded image; widen reply type.
- `src-tauri/src/pipeline/mod.rs` — `save_faces` computes frontality + composite and persists; eager crop generation after clustering.
- `src-tauri/src/thumbnail.rs` — framing fix (margin + square, no distortion) and 320px in `generate_face_crop`.
- `src/app/components/people-view/people-view.component.html` — add `loading="lazy"` to crop `<img>`.

---

## Task 1: Add quality columns to the base `faces` schema

**Files:**
- Modify: `src-tauri/src/db.rs` (the `BASE_SCHEMA` `CREATE TABLE IF NOT EXISTS faces`, around line 82)
- Test: `src-tauri/src/db.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `db.rs`:

```rust
#[tokio::test]
async fn faces_table_has_quality_columns() {
    let dir = std::env::temp_dir().join(format!("nebula_q_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    // PRAGMA table_info returns one row per column; assert our columns exist.
    let cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('faces')")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(cols.contains(&"det_score".to_string()), "faces must have det_score; got {cols:?}");
    assert!(cols.contains(&"quality_score".to_string()), "faces must have quality_score; got {cols:?}");
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula faces_table_has_quality_columns`
Expected: FAIL (column not found / assertion fails).

- [ ] **Step 3: Add the columns to the base schema**

In `BASE_SCHEMA`, change the `faces` table definition (db.rs:82-93) to add two nullable REAL columns before the closing paren:

```sql
CREATE TABLE IF NOT EXISTS faces (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id    INTEGER NOT NULL,
    subject_id  INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    bbox_x      REAL NOT NULL,
    bbox_y      REAL NOT NULL,
    bbox_w      REAL NOT NULL,
    bbox_h      REAL NOT NULL,
    embedding   BLOB,
    added_at    INTEGER NOT NULL,
    is_manual   INTEGER NOT NULL DEFAULT 0,
    det_score      REAL,
    quality_score  REAL
);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula faces_table_has_quality_columns`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(TT-45): add det_score/quality_score columns to faces base schema"
```

---

## Task 2: Quality scoring module (`face_quality.rs`)

Pure functions, no I/O. `frontality` from landmarks, `sharpness` from a grayscale image region, `composite` combining the three normalized signals.

**Files:**
- Create: `src-tauri/src/face_quality.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod face_quality;`)
- Test: `src-tauri/src/face_quality.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/face_quality.rs`:

```rust
//! Pure scoring helpers for choosing the best face crop.
//! All outputs are normalized to 0.0..=1.0 so they can be linearly combined.

use image::DynamicImage;

/// Detector landmark order (SCRFD/buffalo): left_eye, right_eye, nose, left_mouth, right_mouth.
/// Coordinates are relative to the full image (0.0..1.0), matching `DetectedFace.landmarks`.
/// Returns 0.0..1.0; higher = more level and front-facing. Missing/!=5 landmarks -> neutral 0.5.
pub fn frontality(landmarks: Option<&Vec<(f32, f32)>>) -> f32 {
    let lms = match landmarks {
        Some(l) if l.len() == 5 => l,
        _ => return 0.5,
    };
    let (lex, ley) = lms[0];
    let (rex, rey) = lms[1];
    let (nx, _ny) = lms[2];

    let dx = rex - lex;
    let dy = rey - ley;
    let eye_dist = (dx * dx + dy * dy).sqrt().max(1e-6);

    // Roll: eyes should be level. cos(angle) -> 1.0 when horizontal.
    let roll_score = (dx / eye_dist).abs().clamp(0.0, 1.0);

    // Yaw proxy: nose centered between the eyes.
    let eye_mid_x = (lex + rex) / 2.0;
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
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/lib.rs`, add alongside the other `mod` declarations:

```rust
mod face_quality;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p nebula face_quality`
Expected: PASS (5 tests). If a compile error references `enumerate_pixels_mut`, ensure `image` is already a dependency (it is — used in `thumbnail.rs`).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/face_quality.rs src-tauri/src/lib.rs
git commit -m "feat(TT-45): add face_quality scoring module (frontality, sharpness, composite)"
```

---

## Task 3: Carry score, landmarks, and sharpness out of the face actor

The actor currently maps each face to `(bbox, embedding)`, discarding `score` and `landmarks`. Widen the reply to `(DetectedFace, embedding, sharpness)`. Sharpness is computed here because the decoded image is available.

**Files:**
- Modify: `src-tauri/src/pipeline/face_actor.rs`

- [ ] **Step 1: Update the reply type and mapping**

Replace the contents of `src-tauri/src/pipeline/face_actor.rs` with:

```rust
use crate::pipeline::DecodedImage;
use face_id::analyzer::FaceAnalyzer;
use face_id::detector::DetectedFace;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Per detected face: full detection (bbox + landmarks + score), its embedding,
/// and the sharpness (0..1) measured on the cropped face region.
pub type FaceResult = (DetectedFace, Vec<f32>, f32);

pub struct FaceRequest {
    pub decoded: DecodedImage,
    pub reply: oneshot::Sender<anyhow::Result<Vec<FaceResult>>>,
}

pub fn spawn_face_actor(analyzer: Arc<FaceAnalyzer>, channel_depth: usize) -> mpsc::Sender<FaceRequest> {
    let (tx, mut rx) = mpsc::channel::<FaceRequest>(channel_depth);
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let analyzer_c = analyzer.clone();
            let img = req.decoded.full.clone();
            let res = tokio::task::spawn_blocking(move || {
                analyzer_c
                    .analyze(img.as_ref())
                    .map(|faces| {
                        faces
                            .into_iter()
                            .map(|f| {
                                // bbox coords are relative (0..1); crop the region to measure sharpness.
                                let (iw, ih) = (img.width() as f32, img.height() as f32);
                                let x = (f.detection.bbox.x1 * iw).max(0.0) as u32;
                                let y = (f.detection.bbox.y1 * ih).max(0.0) as u32;
                                let w = ((f.detection.bbox.x2 - f.detection.bbox.x1) * iw).max(1.0) as u32;
                                let h = ((f.detection.bbox.y2 - f.detection.bbox.y1) * ih).max(1.0) as u32;
                                let region = img.crop_imm(x, y, w, h);
                                let sharp = crate::face_quality::sharpness(&region);
                                (f.detection, f.embedding, sharp)
                            })
                            .collect::<Vec<_>>()
                    })
                    .map_err(|e| anyhow::anyhow!("{}", e))
            })
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("face task panicked: {e}")));
            let _ = req.reply.send(res);
        }
    });
    tx
}
```

- [ ] **Step 2: Verify it compiles (pipeline/mod.rs will not yet — that is Task 4)**

Run: `cargo check -p nebula 2>&1 | head -30`
Expected: errors ONLY in `pipeline/mod.rs` about the changed `faces` tuple shape (handled next). `face_actor.rs` itself must compile clean. If `crop_imm` is unknown, confirm `image::GenericImageView` is in scope via `DynamicImage` (it is; `crop_imm` is an inherent method on `DynamicImage`).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/pipeline/face_actor.rs
git commit -m "feat(TT-45): carry detection score/landmarks + sharpness out of face actor"
```

---

## Task 4: Persist det_score and quality_score in `save_faces`

`save_faces` receives the widened tuples, computes frontality + composite, and persists both scores. `insert_face` gains two parameters.

**Files:**
- Modify: `src-tauri/src/db.rs` (`insert_face`)
- Modify: `src-tauri/src/pipeline/mod.rs` (`save_faces`)
- Test: `src-tauri/src/db.rs` (tests module)

- [ ] **Step 1: Write the failing test for `insert_face`**

Add to `db.rs` tests:

```rust
#[tokio::test]
async fn insert_face_persists_quality_scores() {
    let dir = std::env::temp_dir().join(format!("nebula_if_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    let face_id = insert_face(&pool, 1, None, (0.1, 0.1, 0.2, 0.2), Some(0.9), Some(0.75))
        .await
        .unwrap();
    let (det, qual): (Option<f64>, Option<f64>) =
        sqlx::query_as("SELECT det_score, quality_score FROM faces WHERE id = ?")
            .bind(face_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(det, Some(0.9));
    assert_eq!(qual, Some(0.75));
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p nebula insert_face_persists_quality_scores`
Expected: FAIL to compile (`insert_face` takes 4 args, not 6).

- [ ] **Step 3: Widen `insert_face`**

Replace `insert_face` in `db.rs` with:

```rust
pub async fn insert_face(
    pool: &SqlitePool,
    image_id: i64,
    subject_id: Option<i64>,
    bbox: (f64, f64, f64, f64),
    det_score: Option<f64>,
    quality_score: Option<f64>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO faces (image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at, det_score, quality_score)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(image_id)
    .bind(subject_id)
    .bind(bbox.0)
    .bind(bbox.1)
    .bind(bbox.2)
    .bind(bbox.3)
    .bind(now)
    .bind(det_score)
    .bind(quality_score)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nebula insert_face_persists_quality_scores`
Expected: PASS.

- [ ] **Step 5: Update `save_faces` to compute and pass scores**

In `src-tauri/src/pipeline/mod.rs`, replace the `save_faces` function. The `faces` parameter type changes to the widened tuple, and we compute frontality + composite per face:

```rust
async fn save_faces(
    pool: &sqlx::SqlitePool,
    image_id: i64,
    sub_qid: i64,
    sub_attempts: i32,
    faces: Vec<face_actor::FaceResult>,
) {
    let mut all_ok = true;
    for (detection, face_emb, sharp) in faces {
        let bbox = detection.bbox;
        let rel_x = bbox.x1 as f64;
        let rel_y = bbox.y1 as f64;
        let rel_w = (bbox.x2 - bbox.x1) as f64;
        let rel_h = (bbox.y2 - bbox.y1) as f64;

        let frontality = crate::face_quality::frontality(detection.landmarks.as_ref());
        let quality = crate::face_quality::composite(detection.score, frontality, sharp);

        match crate::db::insert_face(
            pool,
            image_id,
            None,
            (rel_x, rel_y, rel_w, rel_h),
            Some(detection.score as f64),
            Some(quality as f64),
        )
        .await
        {
            Ok(face_id) => {
                if let Err(e) = crate::face_store::upsert_vector(pool, face_id, &face_emb).await {
                    eprintln!("[pipeline] upsert_vector failed for face {face_id}: {e}");
                    all_ok = false;
                }
            }
            Err(e) => {
                eprintln!("[pipeline] insert_face failed for image {image_id}: {e}");
                all_ok = false;
            }
        }
    }
    if all_ok {
        let _ = crate::db::mark_subject_analysis_done(pool, sub_qid, image_id).await;
    } else {
        let _ = crate::db::mark_failed(pool, sub_qid, sub_attempts, "one or more face inserts failed").await;
    }
}
```

- [ ] **Step 6: Verify the whole crate compiles**

Run: `cargo check -p nebula 2>&1 | head -30`
Expected: clean (the `faces` tuple now flows end-to-end). If other callers of `insert_face` exist, update them to pass `None, None`; find them with `grep -rn 'insert_face(' src-tauri/src`.

- [ ] **Step 7: Run the full backend test suite**

Run: `cargo test -p nebula 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/pipeline/mod.rs
git commit -m "feat(TT-45): compute and persist per-face quality_score in pipeline"
```

---

## Task 5: Select & upgrade the profile crop by quality

Two changes in `db.rs`: (a) `auto_assign_missing_thumbnails` orders candidates by `quality_score` (fill-only, used by non-pipeline callers); (b) new `upgrade_subject_thumbnails` that upgrades to the best-quality face, never sets NULL, and returns the subject IDs whose thumbnail changed (so the pipeline can regenerate those crops).

**Files:**
- Modify: `src-tauri/src/db.rs`
- Test: `src-tauri/src/db.rs` (tests module)

- [ ] **Step 1: Write the failing test for upgrade semantics**

Add to `db.rs` tests:

```rust
#[tokio::test]
async fn upgrade_subject_thumbnails_picks_best_and_upgrades_never_nulls() {
    let dir = std::env::temp_dir().join(format!("nebula_up_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();

    // One subject with a low-quality face.
    let sid: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let low = insert_face(&pool, 1, Some(sid), (0.0, 0.0, 0.2, 0.2), Some(0.5), Some(0.2))
        .await
        .unwrap();

    // First pass: picks the only face, reports the subject as changed.
    let changed = upgrade_subject_thumbnails(&pool).await.unwrap();
    assert_eq!(changed, vec![sid]);
    let thumb: Option<i64> = sqlx::query_scalar("SELECT thumbnail_face_id FROM subjects WHERE id = ?")
        .bind(sid).fetch_one(&pool).await.unwrap();
    assert_eq!(thumb, Some(low));

    // A better face arrives.
    let high = insert_face(&pool, 2, Some(sid), (0.0, 0.0, 0.3, 0.3), Some(0.9), Some(0.9))
        .await
        .unwrap();
    let changed2 = upgrade_subject_thumbnails(&pool).await.unwrap();
    assert_eq!(changed2, vec![sid], "upgrade must report the change");
    let thumb2: Option<i64> = sqlx::query_scalar("SELECT thumbnail_face_id FROM subjects WHERE id = ?")
        .bind(sid).fetch_one(&pool).await.unwrap();
    assert_eq!(thumb2, Some(high), "must upgrade to higher quality face");

    // Idempotent: no change when nothing better appears.
    let changed3 = upgrade_subject_thumbnails(&pool).await.unwrap();
    assert!(changed3.is_empty(), "stable state reports no changes");
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p nebula upgrade_subject_thumbnails_picks_best`
Expected: FAIL to compile (`upgrade_subject_thumbnails` undefined).

- [ ] **Step 3: Implement `upgrade_subject_thumbnails` and update `auto_assign_missing_thumbnails`**

Add to `db.rs`:

```rust
/// For every subject, set `thumbnail_face_id` to its highest-quality face.
/// `quality_score` NULLs sort last; ties fall back to largest bbox area.
/// Never clears an existing thumbnail. Returns subject IDs whose thumbnail changed
/// (newly set or upgraded) so callers can regenerate those crops.
pub async fn upgrade_subject_thumbnails(pool: &SqlitePool) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT s.id AS subject_id,
                s.thumbnail_face_id AS current_face,
                (SELECT f.id FROM faces f
                  WHERE f.subject_id = s.id
                  ORDER BY (f.quality_score IS NULL), f.quality_score DESC,
                           (f.bbox_w * f.bbox_h) DESC
                  LIMIT 1) AS best_face
         FROM subjects s",
    )
    .fetch_all(pool)
    .await?;

    let mut changed = Vec::new();
    for r in &rows {
        let subject_id: i64 = r.get("subject_id");
        let current: Option<i64> = r.get("current_face");
        let best: Option<i64> = r.get("best_face");
        if let Some(best_id) = best {
            if current != Some(best_id) {
                update_subject_thumbnail_face(pool, subject_id, best_id).await?;
                changed.push(subject_id);
            }
        }
        // best is None -> subject has no faces; leave thumbnail untouched (never NULL it).
    }
    Ok(changed)
}
```

Then change the candidate ordering inside `auto_assign_missing_thumbnails` so its existing (fill-only) callers also prefer quality. Replace its body's selection query — currently it calls `get_largest_face_for_subject`. Update `get_largest_face_for_subject`'s `ORDER BY` to prefer quality:

```rust
pub async fn get_largest_face_for_subject(pool: &SqlitePool, subject_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT id FROM faces WHERE subject_id = ?
         ORDER BY (quality_score IS NULL), quality_score DESC, (bbox_w * bbox_h) DESC
         LIMIT 1",
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get("id")))
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nebula upgrade_subject_thumbnails_picks_best`
Expected: PASS.

- [ ] **Step 5: Run full suite**

Run: `cargo test -p nebula 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(TT-45): select/upgrade subject thumbnail by composite quality score"
```

---

## Task 6: Fix crop framing (margin + square, no distortion) at 320px

`generate_face_crop` must expand the bbox by a margin, make it square centered on the face (no aspect squish), clamp to image bounds, and render at 320px.

**Files:**
- Modify: `src-tauri/src/thumbnail.rs`
- Test: `src-tauri/src/thumbnail.rs` (tests module)

- [ ] **Step 1: Write the failing test**

Add to `thumbnail.rs` tests:

```rust
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p nebula face_crop_is_square_320`
Expected: FAIL (current output is 200x200).

- [ ] **Step 3: Rewrite the crop geometry**

Replace the body of `generate_face_crop` in `thumbnail.rs` (keep the signature). The new geometry: take the bbox center, take the larger of (w,h), expand by a margin, make a square side, clamp the square to the image, then resize square→square (no aspect change):

```rust
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
        let mut side = face_px * (1.0 + 2.0 * MARGIN);
        // Side cannot exceed the image's smaller dimension.
        side = side.min(iw).min(ih).max(1.0);

        // Top-left, clamped so the square stays inside the image.
        let x = (cx - side / 2.0).clamp(0.0, iw - side);
        let y = (cy - side / 2.0).clamp(0.0, ih - side);

        let square = img.crop_imm(x as u32, y as u32, side as u32, side as u32);
        // Square -> square keeps aspect ratio (no squish).
        let resized = square.resize_exact(OUT, OUT, image::imageops::FilterType::CatmullRom);
        resized.save_with_format(&dest_path, image::ImageFormat::WebP)?;
        Ok(())
    })
    .await??;

    Ok(())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nebula face_crop_is_square_320`
Expected: PASS. Also run the existing `thumbnail` tests: `cargo test -p nebula thumbnail` → PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/thumbnail.rs
git commit -m "feat(TT-45): well-framed 320px square face crops (margin, no aspect distortion)"
```

---

## Task 7: Eager crop generation after clustering

After each clustering pass, upgrade thumbnails by quality and **immediately generate the crop file** for any subject whose thumbnail changed, then emit `subjects_updated`. Add a DB helper to fetch a face's bbox + source image path together.

**Files:**
- Modify: `src-tauri/src/db.rs` (add `get_face_with_image`)
- Modify: `src-tauri/src/pipeline/mod.rs` (the recluster block at the end of the loop)
- Test: `src-tauri/src/db.rs` (tests module)

- [ ] **Step 1: Write the failing test for the DB helper**

Add to `db.rs` tests:

```rust
#[tokio::test]
async fn get_face_with_image_returns_bbox_and_path() {
    let dir = std::env::temp_dir().join(format!("nebula_fwi_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pool = init_db(&dir).await.unwrap();
    // images.folder_id is a NOT NULL FK to folders(id) and foreign_keys=ON,
    // so insert a folder first, then the image, then a face referencing it.
    let folder_id: i64 = sqlx::query_scalar(
        "INSERT INTO folders (path, added_at) VALUES ('/tmp', 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let img_id: i64 = sqlx::query_scalar(
        "INSERT INTO images (folder_id, path, file_hash, mtime, added_at, updated_at)
         VALUES (?, '/tmp/x.jpg', 'hash', 0, 0, 0) RETURNING id",
    )
    .bind(folder_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let fid = insert_face(&pool, img_id, None, (0.1, 0.2, 0.3, 0.4), Some(0.8), Some(0.7))
        .await
        .unwrap();

    let (path, bbox) = get_face_with_image(&pool, fid).await.unwrap().unwrap();
    assert_eq!(path, "/tmp/x.jpg");
    assert!((bbox.0 - 0.1).abs() < 1e-9 && (bbox.3 - 0.4).abs() < 1e-9);
    std::fs::remove_dir_all(&dir).ok();
}
```

> **Note:** verify the `folders` columns too (search `CREATE TABLE IF NOT EXISTS folders` in `db.rs`); adjust the folder INSERT if its NOT NULL set differs.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p nebula get_face_with_image_returns_bbox`
Expected: FAIL to compile (`get_face_with_image` undefined).

- [ ] **Step 3: Implement the helper**

Add to `db.rs`:

```rust
/// Returns (image_path, (bbox_x, bbox_y, bbox_w, bbox_h)) for a face, or None if missing.
pub async fn get_face_with_image(
    pool: &SqlitePool,
    face_id: i64,
) -> Result<Option<(String, (f64, f64, f64, f64))>> {
    let row = sqlx::query(
        "SELECT i.path AS path, f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h
         FROM faces f JOIN images i ON i.id = f.image_id
         WHERE f.id = ?",
    )
    .bind(face_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (
            r.get::<String, _>("path"),
            (
                r.get::<f64, _>("bbox_x"),
                r.get::<f64, _>("bbox_y"),
                r.get::<f64, _>("bbox_w"),
                r.get::<f64, _>("bbox_h"),
            ),
        )
    }))
}
```

- [ ] **Step 4: Run the helper test to verify it passes**

Run: `cargo test -p nebula get_face_with_image_returns_bbox`
Expected: PASS.

- [ ] **Step 5: Wire eager generation into the pipeline loop**

In `src-tauri/src/pipeline/mod.rs`, replace the auto-recluster block at the end of `run_pipeline` (currently):

```rust
        // Auto-recluster only when subject work was done this iteration
        if processed_subject_work {
            if let Ok(_result) = crate::clustering::cluster_unassigned_faces(&pool).await {
                let _ = app.emit("subjects_updated", ());
            }
        }
```

with:

```rust
        // Auto-recluster only when subject work was done this iteration
        if processed_subject_work {
            if let Ok(_result) = crate::clustering::cluster_unassigned_faces(&pool).await {
                // Upgrade each subject's profile crop to its best-quality face, then
                // generate the crop file eagerly so the People grid has it before the
                // frontend asks (closes the lazy-generation first-paint delay).
                if let Ok(changed) = crate::db::upgrade_subject_thumbnails(&pool).await {
                    for subject_id in changed {
                        let thumb_face: Option<i64> = sqlx::query_scalar(
                            "SELECT thumbnail_face_id FROM subjects WHERE id = ?",
                        )
                        .bind(subject_id)
                        .fetch_one(&pool)
                        .await
                        .unwrap_or(None);
                        if let Some(face_id) = thumb_face {
                            if let Ok(Some((path, bbox))) =
                                crate::db::get_face_with_image(&pool, face_id).await
                            {
                                let dest = crate::thumbnail::face_crop_path_for(&data_dir, face_id);
                                if let Err(e) = crate::thumbnail::generate_face_crop(
                                    std::path::PathBuf::from(path),
                                    dest,
                                    bbox,
                                )
                                .await
                                {
                                    eprintln!("[pipeline] eager crop gen failed for face {face_id}: {e}");
                                }
                            }
                        }
                    }
                }
                let _ = app.emit("subjects_updated", ());
            }
        }
```

- [ ] **Step 6: Verify compile + full suite**

Run: `cargo test -p nebula 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/pipeline/mod.rs
git commit -m "feat(TT-45): eagerly generate profile crop after clustering upgrade"
```

---

## Task 8: Lazy-load crops on the People grid

`get_face_crop` already serves as the idempotent fallback (it generates only when the file is missing — now usually already present). Add `loading="lazy"` so off-screen grid crops don't block visible ones.

**Files:**
- Modify: `src/app/components/people-view/people-view.component.html`

- [ ] **Step 1: Add lazy loading to the crop image**

In `people-view.component.html`, find the subject-card crop image (line ~61):

```html
<img [src]="faceCropUrls()[subject.id]" alt="Face Crop" class="w-full h-full object-cover" />
```

Change it to:

```html
<img [src]="faceCropUrls()[subject.id]" alt="Face Crop" loading="lazy" decoding="async" class="w-full h-full object-cover" />
```

- [ ] **Step 2: Run the frontend unit tests**

Run: `pnpm vitest run src/app/components/people-view`
Expected: PASS (existing `people-view.component.spec.ts` still green).

- [ ] **Step 3: Commit**

```bash
git add src/app/components/people-view/people-view.component.html
git commit -m "feat(TT-45): lazy-load profile crops on the People grid"
```

---

## Task 9: Manual verification & benchmark

No code; this validates the acceptance criteria against a real import.

- [ ] **Step 1: Build and run the app**

Run: `pnpm tauri dev` (or the project's run command — check `package.json` scripts).

- [ ] **Step 2: Import 100+ photos and watch the People page**

Confirm, while the import is still running:
- Subjects appear **with** profile pictures as soon as clusters form (not blank).
- Clicking a subject mid-import shows its already-linked photos.
- A subject's crop **upgrades** as better detections arrive and never blanks out.
- Crops are centered, padded, not clipped, not aspect-distorted.

- [ ] **Step 3: Record the load-time benchmark**

Note median People-grid crop load time on a ~400-photo library, before vs after (compare against `main`). Capture in the PR description.

- [ ] **Step 4: Final full-suite run before PR**

Run: `cargo test -p nebula 2>&1 | tail -20 && pnpm vitest run 2>&1 | tail -10`
Expected: all PASS.

---

## Deferred / explicitly out of scope

- **Explicit concurrency locking:** clustering runs single-threaded per batch, so subject writes are already serialized; a convergence test (`upgrade_subject_thumbnails` idempotency in Task 5) covers the criterion. No new locks.
- **Cache headers on the asset protocol:** Tauri's `asset://` protocol already serves local files with content-stable per-`face_id` filenames; revisit only if Step-3 benchmark shows re-fetch churn.
- **Re-detect backfill for existing libraries:** moot given the alpha DB wipe. If ever needed, a one-time pass re-running detection to populate `quality_score` would go here.

---

## Self-review notes

- **Spec coverage:** A/timing → Task 7 (eager) + existing per-batch clustering; B/quality → Tasks 2–6; C/serving → Tasks 7 (warm) + 8 (lazy) + deferred cache note; tests/benchmark → Tasks 1–9.
- **Type consistency:** `FaceResult = (DetectedFace, Vec<f32>, f32)` defined in Task 3 is consumed by `save_faces` in Task 4; `insert_face` 6-arg signature (Task 4) is used by all later test inserts; `upgrade_subject_thumbnails -> Result<Vec<i64>>` (Task 5) is consumed in Task 7.
- **Schema:** columns added to base schema only (Task 1); no versioned migration, per alpha-wipe instruction.
