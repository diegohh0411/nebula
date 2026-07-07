# TT-93 Bounded-Resolution Decode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound the inference pipeline's image decode to a maximum long edge of 2048px, eliminating full-size (70–140MB) bitmap decodes on the import critical path.

**Architecture:** Extract the existing DCT-scaled coarse-decode helper (`decode_at_most`, currently private to `media/preview.rs`) into a shared `media/decode.rs` primitive, then have `pipeline/decoded_image.rs::load_decoded` consume it with a 2048px bound plus an exact resize pass. The `DecodedImage` contract is unchanged, so no downstream consumer (embed, face, sharpness) changes.

**Tech Stack:** Rust, `image` 0.25, `jpeg-decoder` 0.3 (both already dependencies), `anyhow`. Backend crate root: `src-tauri/`.

## Global Constraints

- No new dependencies. `jpeg-decoder` is already in `src-tauri/Cargo.toml`; do NOT add `zune-jpeg` or `turbojpeg`. `Cargo.toml` must not change.
- Slice architecture (see `CLAUDE.md`): `pipeline → media` cross-slice calls go through the target slice's public API (`crate::media::decode::...`). Never reach into another slice's internals.
- The `DecodedImage { image_id: i64, full: Arc<DynamicImage> }` public contract must not change.
- The inference-path resize filter is `image::imageops::FilterType::Triangle`. Previews keep their own `CatmullRom` filter — do not change preview filters.
- Run all commands from `src-tauri/` (the crate root). The repo worktree root is `/home/pi/nebula/.worktrees/optimize-inference-pipeline`.

---

## File Structure

- `src-tauri/src/media/decode.rs` — **new.** Owns the shared coarse bounded-decode primitive: `decode_at_most` + private `decode_jpeg_scaled`, plus their unit tests.
- `src-tauri/src/media/mod.rs` — **modify.** Add `pub mod decode;`.
- `src-tauri/src/media/preview.rs` — **modify.** Remove the moved functions and their tests; update two call sites to `crate::media::decode::decode_at_most`; drop now-unused imports.
- `src-tauri/src/pipeline/decoded_image.rs` — **modify.** Add `DECODE_MAX_LONG_EDGE` constant; rewrite `load_decoded` to use the shared primitive + exact resize; add bound/passthrough tests.

---

## Task 1: Extract shared decode primitive into `media/decode.rs`

Pure refactor (code move), no behavior change. The gate is that the relocated tests and all existing `media` tests still pass.

**Files:**
- Create: `src-tauri/src/media/decode.rs`
- Modify: `src-tauri/src/media/mod.rs`
- Modify: `src-tauri/src/media/preview.rs`

**Interfaces:**
- Produces: `crate::media::decode::decode_at_most(path: &Path, target_long_edge: u32) -> anyhow::Result<image::DynamicImage>` — decodes a JPEG at a coarse power-of-two DCT downscale whose long edge is ≥ `target_long_edge`, or full-decodes other formats; never upscales; the caller does any exact final resize.
- Consumes: nothing from earlier tasks.

- [ ] **Step 1: Create `media/decode.rs` with the moved functions and tests**

Create `src-tauri/src/media/decode.rs` with exactly this content (functions moved verbatim from `preview.rs`, plus a local copy of the `write_jpeg` test helper):

```rust
use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::Path;

/// Decode an image at a coarse downscale such that the longest edge is
/// ≤ `target_long_edge`, preserving aspect ratio. For JPEG this scales
/// DURING decode (power-of-two factor) via `jpeg-decoder`; other formats
/// decode fully via `image`. The caller is responsible for the final exact
/// resize to the target dimensions.
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
```

- [ ] **Step 2: Register the module in `media/mod.rs`**

Modify `src-tauri/src/media/mod.rs` to add the new module. After the change the file reads:

```rust
pub mod commands;
pub mod decode;
pub mod preview;
pub mod thumbnail;
```

- [ ] **Step 3: Remove the moved functions from `preview.rs`**

In `src-tauri/src/media/preview.rs`, delete the entire `decode_at_most` function and the entire `decode_jpeg_scaled` function (the block that currently spans from the `/// Decode an image at a coarse downscale ...` doc comment through the closing brace of `decode_jpeg_scaled`, ending just before `/// Tier 1: decode coarsely, ...`). Do not delete the `write_preview` / `write_thumbnail` functions that follow.

- [ ] **Step 4: Update the two call sites in `preview.rs`**

In `write_preview`, change:

```rust
    let img = decode_at_most(src, 256)?;
```
to:
```rust
    let img = crate::media::decode::decode_at_most(src, 256)?;
```

In `write_thumbnail`, change:

```rust
    let img = decode_at_most(src, 1600)?;
```
to:
```rust
    let img = crate::media::decode::decode_at_most(src, 1600)?;
```

- [ ] **Step 5: Remove the moved tests from `preview.rs` and drop now-unused imports**

In the `#[cfg(test)] mod tests` block of `preview.rs`, delete these three test functions (they now live in `decode.rs`): `decode_at_most_scales_large_jpeg_down`, `decode_at_most_does_not_upscale_small_image`, `decode_at_most_errors_on_missing_file`. **Keep** the `write_jpeg` helper — it is still used by `write_preview_creates_small_webp` and `write_thumbnail_creates_800px_webp`.

Then fix the two imports at the top of `preview.rs` that are now unused (all their uses lived in the moved code):

- Change `use anyhow::{Context, Result};` to `use anyhow::Result;`
- Delete the line `use image::DynamicImage;` (remaining references in the file use the fully-qualified `image::DynamicImage`).

- [ ] **Step 6: Run the media tests and confirm they pass**

Run: `cargo test -p nebula media::`

Expected: PASS. The three `decode_at_most_*` tests run from their new location under `media::decode::tests`; `write_preview_creates_small_webp` and `write_thumbnail_creates_800px_webp` still pass.

If the crate/package name is not `nebula`, run `cargo test decode_at_most write_preview write_thumbnail` instead and confirm those tests pass.

- [ ] **Step 7: Confirm a clean build with no unused-import warnings**

Run: `cargo build 2>&1 | grep -i "warning: unused" || echo "no unused-import warnings"`

Expected: `no unused-import warnings` (specifically none pointing at `preview.rs`).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/media/decode.rs src-tauri/src/media/mod.rs src-tauri/src/media/preview.rs
git commit -m "refactor(media): extract decode_at_most into shared media::decode module

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01LmunYmT5jPZZt2pxw2sMrb"
```

---

## Task 2: Bound `load_decoded` to 2048px on the long edge

**Files:**
- Modify: `src-tauri/src/pipeline/decoded_image.rs`

**Interfaces:**
- Consumes: `crate::media::decode::decode_at_most(path, target_long_edge)` from Task 1.
- Produces: `pub const DECODE_MAX_LONG_EDGE: u32 = 2048;` and an updated `load_decoded(image_id: i64, path: &Path) -> anyhow::Result<DecodedImage>` whose returned `full` image never exceeds 2048px on the long edge for oversized inputs and is byte-for-byte the coarse decode for inputs already within the bound.

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/pipeline/decoded_image.rs`, add these two tests inside the existing `#[cfg(test)] mod tests` block (keep the two existing tests):

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p nebula load_decoded_bounds_oversized_to_2048_long_edge`

Expected: FAIL. With the current unbounded `load_decoded`, the returned image is 4000x2000, so `long` is 4000 and the assertion `long <= 2048` fails.

- [ ] **Step 3: Add the constant and rewrite `load_decoded`**

In `src-tauri/src/pipeline/decoded_image.rs`, add the constant just above `load_decoded`, and replace the body of `load_decoded`. The functions become:

```rust
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
```

Note: the top-of-file imports `use anyhow::{Context, Result};`, `use image::DynamicImage;`, `use std::path::Path;`, and `use std::sync::Arc;` are all still required (`Context` via `with_context`, `DynamicImage` via the `DecodedImage` struct field). Leave them as they are.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p nebula load_decoded`

Expected: PASS — all four tests: `load_decoded_bounds_oversized_to_2048_long_edge`, `load_decoded_passes_small_image_through_untouched`, `load_decoded_decodes_once_and_keeps_dimensions` (2x2 image, within bound → passthrough), and `load_decoded_errors_on_missing_file`.

- [ ] **Step 5: Run the full pipeline + media suites to confirm no regressions**

Run: `cargo test -p nebula pipeline:: media::`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/pipeline/decoded_image.rs
git commit -m "perf(pipeline): decode inference images bounded to 2048px long edge

Closes TT-93. load_decoded now uses the shared media::decode primitive with a
Triangle resize to the 2048 bound; DecodedImage contract unchanged.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01LmunYmT5jPZZt2pxw2sMrb"
```

---

## Manual acceptance verification (pre-merge, not automated)

These acceptance criteria from TT-93 cannot be unit-tested and must be checked by hand on a real library before opening/merging the PR:

- [ ] **Perf:** Run a bulk import of camera-resolution JPEGs with `RUST_LOG` enabled for the pipeline. Capture `[embed]` and `[face]` timing lines before (main) and after (this branch); confirm a wall-time improvement. Record the numbers in the PR description.
- [ ] **Face-count parity:** On a face-heavy reference library, confirm the total detected face count matches the pre-change baseline (±0). If it drops, the 2048 bound is too aggressive for tiny faces — reconsider `DECODE_MAX_LONG_EDGE` before merge.

---

## Self-review notes

- **Spec coverage:** Extraction → Task 1. Bounded `load_decoded` + constant + Triangle resize → Task 2. ≤2048 bound & passthrough tests → Task 2 Steps 1–4. No `Cargo.toml` change → Global Constraints (verified: `jpeg-decoder` already present). Downstream-safety (relative bboxes) requires no code change and is documented in the spec. Perf + face-count criteria → manual verification section.
- **No new types/functions referenced that a task doesn't define:** `decode_at_most` (Task 1), `DECODE_MAX_LONG_EDGE` + `load_decoded` (Task 2). `DecodedImage` pre-exists and is unchanged.
- **Type consistency:** `decode_at_most(&Path, u32) -> Result<DynamicImage>` and `load_decoded(i64, &Path) -> Result<DecodedImage>` are used identically everywhere they appear.
