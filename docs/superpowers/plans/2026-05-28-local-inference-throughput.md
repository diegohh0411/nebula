# Local Inference Throughput Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Increase image-processing throughput on the existing laptop (Windows 11, integrated GPU) by decoding each image once, parallelizing/batching ONNX inference, splitting the SigLIP towers, and adding an execution-provider option — without changing the ML models' quality.

**Architecture:** Replace the two competing background workers with one staged, decode-once pipeline (LOAD → INFER → WRITE) connected by bounded channels. A dedicated inference actor owns each ONNX session (removing the session-mutex serialization), runs batched embedding on split vision/text towers, and can optionally offload to the iGPU via the DirectML execution provider. A benchmark harness measures every step.

**Tech Stack:** Rust, Tauri 2, `ort` 2.0.0-rc.12 (ONNX Runtime), `face_id` 0.4.1, `image` 0.25, `ndarray` 0.17, `tokio`, `rayon`, SQLite via `sqlx`.

**Spec:** `docs/superpowers/specs/2026-05-28-local-inference-throughput-design.md`

---

## File Structure

**New files:**
- `src-tauri/src/pipeline/mod.rs` — pipeline coordinator `run_pipeline`, `PipelineConfig`, `ComputePlacement`.
- `src-tauri/src/pipeline/decoded_image.rs` — `DecodedImage` (the single decoded buffer) + `load_decoded` helper.
- `src-tauri/src/pipeline/embed_actor.rs` — embedding inference actor owning the SigLIP `Session`; batched.
- `src-tauri/src/pipeline/face_actor.rs` — face inference actor owning the `FaceAnalyzer`.
- `src-tauri/src/preprocess.rs` — vectorized image→tensor preprocessing.
- `src-tauri/examples/bench.rs` — standalone benchmark harness binary.

**Modified files:**
- `src-tauri/Cargo.toml` — add `num_cpus`; add `directml` feature to `ort` and `face_id`.
- `src-tauri/src/models/registry.rs` — split SigLIP into separate vision/text model files + configurable tensor names.
- `src-tauri/src/vision_engine.rs` — split-tower embed, batched embed, EP config, vectorized preprocess; remove session-mutex-across-run.
- `src-tauri/src/thumbnail.rs` — generate thumbnail + face crops from an in-memory `DynamicImage`.
- `src-tauri/src/embedder.rs` — keep DB/queue/index helpers; remove the two `run_*_worker` loops (moved into pipeline).
- `src-tauri/src/indexer.rs` — stop the separate `spawn_thumbnail` at scan time (thumbnails now produced by the pipeline).
- `src-tauri/src/lib.rs` — spawn a single `pipeline::run_pipeline` instead of the two workers; thread `PipelineConfig`.

**Conventions to follow (from existing code):**
- Errors: `anyhow::Result` in app code; map `face_id`/`ort` errors with `anyhow::anyhow!`.
- Embeddings stored as little-endian f32 BLOBs via `embedder::f32_slice_to_bytes`.
- Face bbox stored as **relative** fractions `(x, y, w, h)` (see `embedder.rs:170-177`).
- Tauri events: `image_updated`, `subjects_updated`, `processing_progress` (keep emitting these).
- Tests live in `#[cfg(test)] mod tests` at the bottom of each file, like `vector_index.rs`.

---

## Task 0: Pre-flight — confirm upstream artifact names

This task gathers facts the later tasks depend on. No code change; record the answers in the plan checkboxes.

- [ ] **Step 1: Pick the split SigLIP repo and list its files**

The current combined model is `diegohh/siglip2-base-patch16-224`. The `onnx-community` mirror ships split towers. Open https://huggingface.co/onnx-community/siglip2-base-patch16-224/tree/main/onnx and record the exact filenames. Expected to exist:
- `vision_model.onnx` (+ `vision_model_fp16.onnx`, `vision_model_quantized.onnx`)
- `text_model.onnx` (+ fp16 / quantized variants)

Write the chosen filenames here: `vision = onnx/vision_model.onnx`, `text = onnx/text_model.onnx` (adjust if different).

- [ ] **Step 2: Record the vision model's input/output tensor names**

Download `vision_model.onnx` and inspect it (Netron, or `python -c "import onnx; m=onnx.load('vision_model.onnx'); print([i.name for i in m.graph.input], [o.name for o in m.graph.output])"`).

Record:
- Vision input name (expected `pixel_values`): `__________`
- Vision output name (expected `image_embeds`; may be `pooler_output`): `__________`
- Text input name (expected `input_ids`): `__________`
- Text output name (expected `text_embeds`; may be `pooler_output`): `__________`
- Confirm the batch axis (dim 0) is **dynamic** (shown as a symbolic name, not a fixed `1`).

These names feed Task 4 (`ModelSpec` tensor-name fields). Do not proceed to Task 4 until recorded.

---

## Task 1: Benchmark harness (baseline first)

**Files:**
- Create: `src-tauri/examples/bench.rs`
- Modify: `src-tauri/Cargo.toml`

The harness processes a fixed folder end-to-end using the *current* code paths and prints per-stage timings + images/sec, so every later task is measured against a baseline.

- [ ] **Step 1: Add `num_cpus` dependency**

In `src-tauri/Cargo.toml` under `[dependencies]`, add:

```toml
num_cpus = "1.16"
```

- [ ] **Step 2: Write the benchmark binary**

Create `src-tauri/examples/bench.rs`:

```rust
//! End-to-end throughput benchmark.
//! Usage: NEBULA_BENCH_DIR=path/to/folder cargo run --release --example bench
//!
//! Decodes every JPEG/PNG in the folder and runs the current embed + face paths,
//! printing per-stage timings and images/sec. This is the baseline that every
//! optimization task is measured against.

use std::path::PathBuf;
use std::time::Instant;

fn list_images(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase());
            if matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png")) {
                out.push(p);
            }
        }
    }
    out
}

#[derive(Default)]
struct Stage {
    count: u64,
    total_ms: f64,
}
impl Stage {
    fn add(&mut self, ms: f64) {
        self.count += 1;
        self.total_ms += ms;
    }
    fn avg(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.total_ms / self.count as f64 }
    }
}

fn main() {
    let dir = std::env::var("NEBULA_BENCH_DIR")
        .expect("set NEBULA_BENCH_DIR to a folder of images");
    let dir = PathBuf::from(dir);
    let images = list_images(&dir);
    assert!(!images.is_empty(), "no images found in {}", dir.display());
    eprintln!("benchmarking {} images from {}", images.len(), dir.display());

    let mut decode = Stage::default();

    let wall = Instant::now();
    for path in &images {
        let t = Instant::now();
        let _img = image::open(path).expect("decode");
        decode.add(t.elapsed().as_secs_f64() * 1000.0);
        // NOTE: embed/face stages are wired in once the pipeline exposes a
        // reusable single-image entry point (Task 9). For the baseline run we
        // measure decode only; record this number now.
    }
    let secs = wall.elapsed().as_secs_f64();

    println!("--- bench results ---");
    println!("images:        {}", images.len());
    println!("decode avg ms: {:.1}", decode.avg());
    println!("wall secs:     {:.2}", secs);
    println!("images/sec:    {:.2}", images.len() as f64 / secs);
}
```

- [ ] **Step 3: Run the baseline and record numbers**

Run: `cd src-tauri && set NEBULA_BENCH_DIR=C:\path\to\sample && cargo run --release --example bench`
Expected: prints `images`, `decode avg ms`, `images/sec`. Record these numbers in the PR description as the baseline. (Use a real 300–1000 image sample folder.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/examples/bench.rs src-tauri/Cargo.toml
git commit -m "test: add end-to-end throughput benchmark harness"
```

---

## Task 2: `DecodedImage` + decode-once helper (Stage 1)

**Files:**
- Create: `src-tauri/src/pipeline/mod.rs`
- Create: `src-tauri/src/pipeline/decoded_image.rs`
- Modify: `src-tauri/src/lib.rs` (register module)

- [ ] **Step 1: Register the `pipeline` module**

In `src-tauri/src/lib.rs`, add to the module list near the top (after `mod models;`):

```rust
mod pipeline;
```

- [ ] **Step 2: Create the module file**

Create `src-tauri/src/pipeline/mod.rs`:

```rust
pub mod decoded_image;

pub use decoded_image::{DecodedImage, load_decoded};
```

- [ ] **Step 3: Write the failing test for `load_decoded`**

Create `src-tauri/src/pipeline/decoded_image.rs`:

```rust
use anyhow::{Context, Result};
use image::DynamicImage;
use std::path::Path;
use std::sync::Arc;

/// An image decoded exactly once, shared (read-only) across all pipeline stages.
///
/// `full` is the originally-decoded image, reused for thumbnail and face crops.
/// Embedding and face detection both read from `full` — the file is never
/// re-opened after Stage 1.
#[derive(Clone)]
pub struct DecodedImage {
    pub image_id: i64,
    pub full: Arc<DynamicImage>,
}

/// Decode an image from disk once. CPU/IO bound — call inside `spawn_blocking`
/// or a rayon task, never on the async runtime.
pub fn load_decoded(image_id: i64, path: &Path) -> Result<DecodedImage> {
    let full = image::open(path)
        .with_context(|| format!("failed to decode image at {}", path.display()))?;
    Ok(DecodedImage { image_id, full: Arc::new(full) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_decoded_decodes_once_and_keeps_dimensions() {
        // 2x2 red PNG written to a temp file.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nebula_decode_{}.png", std::process::id()));
        let mut img = image::RgbImage::new(2, 2);
        for p in img.pixels_mut() {
            *p = image::Rgb([255, 0, 0]);
        }
        image::DynamicImage::ImageRgb8(img).save(&path).unwrap();

        let decoded = load_decoded(42, &path).unwrap();
        assert_eq!(decoded.image_id, 42);
        assert_eq!(decoded.full.width(), 2);
        assert_eq!(decoded.full.height(), 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_decoded_errors_on_missing_file() {
        let res = load_decoded(1, Path::new("definitely-not-here.jpg"));
        assert!(res.is_err());
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib pipeline::decoded_image`
Expected: 2 tests pass. (If the `pipeline` module isn't found, recheck Step 1.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pipeline/ src-tauri/src/lib.rs
git commit -m "feat(pipeline): add DecodedImage and decode-once loader"
```

---

## Task 3: Thumbnail + face crops from an in-memory buffer

**Files:**
- Modify: `src-tauri/src/thumbnail.rs`
- Test: `src-tauri/src/thumbnail.rs` (`#[cfg(test)]`)

Today `generate_thumbnail` and `generate_face_crop` each call `image::open` (`thumbnail.rs:29,52`). Add in-memory variants that take a `&DynamicImage`, so the pipeline produces both from the single decoded buffer. Keep the old fns as thin wrappers for any remaining callers, then remove their callers in later tasks.

- [ ] **Step 1: Write failing tests for the in-memory variants**

Add to the bottom of `src-tauri/src/thumbnail.rs` (inside a new `#[cfg(test)] mod tests`):

```rust
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd src-tauri && cargo test --lib thumbnail::tests`
Expected: FAIL — `write_thumbnail_from_image` / `write_face_crop_from_image` not found.

- [ ] **Step 3: Implement the in-memory variants**

In `src-tauri/src/thumbnail.rs`, add these functions (and add `use image::DynamicImage;` at the top if not present):

```rust
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
```

- [ ] **Step 4: Run to verify they pass**

Run: `cd src-tauri && cargo test --lib thumbnail::tests`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/thumbnail.rs
git commit -m "feat(thumbnail): add in-memory thumbnail and face-crop variants"
```

---

## Task 4: Split SigLIP towers in the registry

**Files:**
- Modify: `src-tauri/src/models/registry.rs`
- Test: `src-tauri/src/models/registry.rs` (`#[cfg(test)]`)

Add separate vision/text model files and configurable tensor names to `ModelSpec`, then point the standard model at the `onnx-community` split exports (filenames + names recorded in Task 0).

- [ ] **Step 1: Add fields to `ModelSpec`**

In `src-tauri/src/models/registry.rs`, extend the `ModelSpec` struct (after `image_size`):

```rust
  /// Separate vision-tower ONNX file (image encoder). When set, embed_image uses this.
  pub vision_file: Option<ModelFile>,
  /// Separate text-tower ONNX file (text encoder). When set, embed_text uses this.
  pub text_file: Option<ModelFile>,
  /// Input tensor name for the vision tower (e.g. "pixel_values").
  pub vision_input: &'static str,
  /// Output tensor name for the vision tower (e.g. "image_embeds").
  pub vision_output: &'static str,
  /// Input tensor name for the text tower (e.g. "input_ids").
  pub text_input: &'static str,
  /// Output tensor name for the text tower (e.g. "text_embeds").
  pub text_output: &'static str,
```

- [ ] **Step 2: Point `SIGLIP_BASE` at the split files**

Replace the `SIGLIP_BASE` const with (substitute the exact names from Task 0):

```rust
pub const SIGLIP_BASE: ModelSpec = ModelSpec {
  id: "onnx-community/siglip2-base-patch16-224",
  hf_repo: "onnx-community/siglip2-base-patch16-224",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-224-split",
  // model_file is still required by the existing download/ready machinery; point
  // it at the vision tower so "ready" means the image encoder is present.
  model_file: ModelFile { filename: "vision_model.onnx", remote_path: Some("onnx/vision_model.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Standard",
  display_description: "Balanced quality and speed (86M params, split towers)",
  image_size: 224,
  vision_file: Some(ModelFile { filename: "vision_model.onnx", remote_path: Some("onnx/vision_model.onnx") }),
  text_file: Some(ModelFile { filename: "text_model.onnx", remote_path: Some("onnx/text_model.onnx") }),
  vision_input: "pixel_values",
  vision_output: "image_embeds",
  text_input: "input_ids",
  text_output: "text_embeds",
};
```

- [ ] **Step 3: Fill the new fields on every other `ModelSpec`**

For `SIGLIP_FAST`, set `vision_file`/`text_file` to its split equivalents if available, else `None` with the tensor names matching its combined graph. For the three `BUFFALO_*` specs (face models, not embedding models), set:

```rust
  vision_file: None,
  text_file: None,
  vision_input: "",
  vision_output: "",
  text_input: "",
  text_output: "",
```

Add these six fields to `BUFFALO_S_RECOGNITION`, `BUFFALO_S_DETECTION`, `BUFFALO_S_GENDER_AGE`, and `SIGLIP_FAST` so the file compiles.

- [ ] **Step 4: Add a test asserting the standard model is split**

Add to the `#[cfg(test)] mod tests` in `registry.rs`:

```rust
    #[test]
    fn standard_model_has_split_towers() {
        let s = &SIGLIP_BASE;
        assert!(s.vision_file.is_some(), "vision tower must be configured");
        assert!(s.text_file.is_some(), "text tower must be configured");
        assert_eq!(s.vision_input, "pixel_values");
        assert!(!s.vision_output.is_empty());
        assert!(!s.text_output.is_empty());
    }
```

- [ ] **Step 5: Run tests**

Run: `cd src-tauri && cargo test --lib models::registry`
Expected: existing tests + `standard_model_has_split_towers` pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/models/registry.rs
git commit -m "feat(models): configure split SigLIP vision/text towers"
```

> **Note for ModelManager:** `manager.onnx_path(spec)` returns the path for `model_file`. Add a sibling that resolves an arbitrary `ModelFile` so the engine can load `vision_file`/`text_file`. If `ModelManager::onnx_path` builds `cache_dir.join(model_file.filename)`, add:
> ```rust
> pub fn model_file_path(&self, spec: &ModelSpec, file: &ModelFile) -> PathBuf {
>     self.models_dir().join(spec.cache_dir).join(file.filename)
> }
> ```
> and make `ensure_ready` download `vision_file` and `text_file` when present. Verify against `src-tauri/src/models/manager.rs` and mirror its existing path/download logic exactly.

---

## Task 5: Vectorized preprocessing

**Files:**
- Create: `src-tauri/src/preprocess.rs`
- Modify: `src-tauri/src/lib.rs` (register module)
- Test: `src-tauri/src/preprocess.rs`

Replace the scalar per-pixel loop (`vision_engine.rs:127-132`) with a reusable, vectorized function that also supports a batch dimension (used in Task 7).

- [ ] **Step 1: Register the module**

In `src-tauri/src/lib.rs` add: `mod preprocess;`

- [ ] **Step 2: Write the failing test**

Create `src-tauri/src/preprocess.rs`:

```rust
use image::DynamicImage;
use ndarray::Array4;

/// Resize `img` to `size`x`size` and write it into `dst` at batch index `b`,
/// normalized to [-1, 1] in CHW order. `dst` must have shape (B, 3, size, size).
pub fn fill_pixel_values(
    dst: &mut Array4<f32>,
    b: usize,
    img: &DynamicImage,
    size: usize,
) {
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
        for p in img.pixels_mut() { *p = image::Rgb([255, 0, 0]); }
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
```

- [ ] **Step 3: Run to verify pass**

Run: `cd src-tauri && cargo test --lib preprocess`
Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/preprocess.rs src-tauri/src/lib.rs
git commit -m "feat(preprocess): vectorized batch-aware pixel normalization"
```

---

## Task 6: Embed using split towers (single image, no mutex-across-run)

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`
- Test: `src-tauri/src/vision_engine.rs`

Rework `embed_image`/`embed_text` to (a) use the separate vision/text sessions and (b) stop running the unused tower. This task keeps batch size 1; Task 7 adds batching. The session is still loaded lazily but each tower has its own slot.

- [ ] **Step 1: Replace the session storage with per-tower slots**

In `vision_engine.rs`, change the struct:

```rust
pub struct VisionEngine {
    pub data_dir: PathBuf,
    vision_session: std::sync::Mutex<Option<(String, Session)>>,
    text_session: std::sync::Mutex<Option<(String, Session)>>,
    tokenizer: std::sync::Mutex<Option<(String, tokenizers::Tokenizer)>>,
    face_analyzer: std::sync::Mutex<Option<(String, Arc<FaceAnalyzer>)>>,
}
```

Update `new` to initialize both session slots to `None`.

- [ ] **Step 2: Add a session loader that takes a specific file + tensor names**

Add a helper that loads a session for a given `ModelFile` (uses `model_file_path` from Task 4):

```rust
fn load_session(path: &std::path::Path) -> Result<Session> {
    Session::builder()
        .map_err(|e| anyhow!("failed to create session builder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow!("failed to set optimization level: {e}"))?
        .with_intra_threads(num_cpus::get_physical())
        .map_err(|e| anyhow!("failed to set intra threads: {e}"))?
        .commit_from_file(path)
        .map_err(|e| anyhow!("failed to load ONNX model '{}': {e}", path.display()))
}
```

(Add `use num_cpus;` is unnecessary — call `num_cpus::get_physical()` directly; ensure `num_cpus` is a dependency from Task 1.)

- [ ] **Step 3: Rewrite `embed_image` to use only the vision tower**

Replace the body of `embed_image` with:

```rust
pub fn embed_image(&self, manager: &ModelManager, img: &image::DynamicImage, spec: &ModelSpec) -> Result<Vec<f32>> {
    let size = spec.image_size;
    let mut pixel_values = ndarray::Array4::<f32>::zeros((1, 3, size, size));
    crate::preprocess::fill_pixel_values(&mut pixel_values, 0, img, size);

    let vision_file = spec.vision_file.as_ref()
        .ok_or_else(|| anyhow!("model '{}' has no vision tower configured", spec.id))?;
    let path = manager.model_file_path(spec, vision_file);

    let mut lock = self.vision_session.lock().map_err(|e| anyhow!("mutex poisoned: {e}"))?;
    let needs_load = match &*lock { Some((id, _)) => id != spec.id, None => true };
    if needs_load {
        *lock = Some((spec.id.to_string(), Self::load_session(&path)?));
    }
    let (_, session) = lock.as_mut().unwrap();

    let pv_ref = TensorRef::from_array_view(pixel_values.view())
        .map_err(|e| anyhow!("failed to create pixel_values tensor: {e}"))?;
    let outputs = session
        .run(ort::inputs![spec.vision_input => pv_ref])
        .map_err(|e| anyhow!("image inference failed: {e}"))?;
    let (_shape, data) = outputs[spec.vision_output]
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow!("failed to extract image embedding: {e}"))?;
    Ok(data.to_vec())
}
```

Note: no more dummy `input_ids`; the vision graph takes only `pixel_values`.

- [ ] **Step 4: Rewrite `embed_text` to use only the text tower**

Mirror Step 3 using `text_session`, `spec.text_file`, `spec.text_input`, `spec.text_output`, and the existing tokenizer logic (keep `MAX_SEQ_LEN = 64`). Remove the dummy `pixel_values`. Delete the old combined `get_session` once both `embed_image` and `embed_text` no longer call it.

- [ ] **Step 5: Write a smoke test guarded behind the model being present**

Add to `vision_engine.rs` tests (skips when the model isn't downloaded, so CI without models still passes):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_image_returns_expected_dim_when_model_present() {
        let data_dir = match std::env::var("NEBULA_TEST_DATA_DIR") {
            Ok(d) => std::path::PathBuf::from(d),
            Err(_) => { eprintln!("skipping: NEBULA_TEST_DATA_DIR not set"); return; }
        };
        let manager = crate::models::ModelManager::new(data_dir.clone());
        let spec = &crate::models::registry::SIGLIP_BASE;
        let vf = spec.vision_file.as_ref().unwrap();
        if !manager.model_file_path(spec, vf).exists() {
            eprintln!("skipping: vision model not downloaded");
            return;
        }
        let engine = VisionEngine::new(data_dir);
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(64, 64));
        let emb = engine.embed_image(&manager, &img, spec).unwrap();
        assert_eq!(emb.len(), 768, "SigLIP base image embedding dim");
    }
}
```

- [ ] **Step 6: Build and test**

Run: `cd src-tauri && cargo build && cargo test --lib vision_engine`
Expected: compiles; test passes or prints a skip message. Then run `cargo run --release --example bench` and confirm decode numbers unchanged (embed path not yet in bench).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat(vision): embed via split towers, drop unused-tower compute"
```

---

## Task 7: Batched embedding inference

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`
- Test: `src-tauri/src/vision_engine.rs`

Add `embed_images_batch` that preprocesses N images into one `(N,3,H,W)` tensor and runs a single `session.run`. Single-image `embed_image` becomes a batch-of-one call to keep one code path.

- [ ] **Step 1: Add the batched method**

```rust
pub fn embed_images_batch(
    &self,
    manager: &ModelManager,
    imgs: &[&image::DynamicImage],
    spec: &ModelSpec,
) -> Result<Vec<Vec<f32>>> {
    if imgs.is_empty() {
        return Ok(vec![]);
    }
    let size = spec.image_size;
    let n = imgs.len();
    let mut pixel_values = ndarray::Array4::<f32>::zeros((n, 3, size, size));
    for (b, img) in imgs.iter().enumerate() {
        crate::preprocess::fill_pixel_values(&mut pixel_values, b, img, size);
    }

    let vision_file = spec.vision_file.as_ref()
        .ok_or_else(|| anyhow!("model '{}' has no vision tower configured", spec.id))?;
    let path = manager.model_file_path(spec, vision_file);

    let mut lock = self.vision_session.lock().map_err(|e| anyhow!("mutex poisoned: {e}"))?;
    let needs_load = match &*lock { Some((id, _)) => id != spec.id, None => true };
    if needs_load {
        *lock = Some((spec.id.to_string(), Self::load_session(&path)?));
    }
    let (_, session) = lock.as_mut().unwrap();

    let pv_ref = TensorRef::from_array_view(pixel_values.view())
        .map_err(|e| anyhow!("failed to create pixel_values tensor: {e}"))?;
    let outputs = session
        .run(ort::inputs![spec.vision_input => pv_ref])
        .map_err(|e| anyhow!("batched image inference failed: {e}"))?;
    let (shape, data) = outputs[spec.vision_output]
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow!("failed to extract batched image embeddings: {e}"))?;

    let dim = (data.len() / n) as usize;
    anyhow::ensure!(
        shape.first().copied() == Some(n as i64) || data.len() % n == 0,
        "unexpected batch output shape {:?} for n={}", shape, n
    );
    Ok((0..n).map(|i| data[i * dim..(i + 1) * dim].to_vec()).collect())
}
```

Then make `embed_image` delegate:

```rust
pub fn embed_image(&self, manager: &ModelManager, img: &image::DynamicImage, spec: &ModelSpec) -> Result<Vec<f32>> {
    let mut out = self.embed_images_batch(manager, &[img], spec)?;
    out.pop().ok_or_else(|| anyhow!("empty batch result"))
}
```

- [ ] **Step 2: Add a test that batched == single (within tolerance)**

```rust
    #[test]
    fn batched_embeddings_match_single_when_model_present() {
        let data_dir = match std::env::var("NEBULA_TEST_DATA_DIR") {
            Ok(d) => std::path::PathBuf::from(d),
            Err(_) => { eprintln!("skipping"); return; }
        };
        let manager = crate::models::ModelManager::new(data_dir.clone());
        let spec = &crate::models::registry::SIGLIP_BASE;
        let vf = spec.vision_file.as_ref().unwrap();
        if !manager.model_file_path(spec, vf).exists() { eprintln!("skipping"); return; }
        let engine = VisionEngine::new(data_dir);

        let a = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb([200,40,40])));
        let b = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb([40,40,200])));
        let single_a = engine.embed_image(&manager, &a, spec).unwrap();
        let batch = engine.embed_images_batch(&manager, &[&a, &b], spec).unwrap();
        assert_eq!(batch.len(), 2);
        for (x, y) in single_a.iter().zip(batch[0].iter()) {
            assert!((x - y).abs() < 1e-3, "batched vs single mismatch: {x} vs {y}");
        }
    }
```

- [ ] **Step 3: Build and test**

Run: `cd src-tauri && cargo test --lib vision_engine`
Expected: passes or skips. If the batch output shape assertion fires, re-check the vision output name from Task 0.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat(vision): batched embedding inference"
```

---

## Task 8: Execution-provider config + DirectML option

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/vision_engine.rs`, `src-tauri/src/pipeline/mod.rs`
- Test: manual benchmark comparison

- [ ] **Step 1: Enable EP features**

In `src-tauri/Cargo.toml`:

```toml
ort = { version = "2.0.0-rc.12", features = ["directml"] }
face_id = { version = "0.4.1", features = ["directml"] }
```

- [ ] **Step 2: Add `ComputePlacement` and thread it into session creation**

In `src-tauri/src/pipeline/mod.rs` add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputePlacement {
    /// SigLIP runs on CPU (default, always available).
    Cpu,
    /// SigLIP offloaded to the iGPU via DirectML; CPU stays free for face work.
    Gpu,
}

#[derive(Clone, Debug)]
pub struct PipelineConfig {
    pub batch_size: usize,        // embed batch (default 12)
    pub load_channel_depth: usize, // bounded in-flight decoded images (default 24)
    pub infer_channel_depth: usize,
    pub placement: ComputePlacement,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            batch_size: 12,
            load_channel_depth: 24,
            infer_channel_depth: 24,
            placement: ComputePlacement::Cpu,
        }
    }
}
```

- [ ] **Step 3: Make `load_session` accept placement**

Change `load_session` in `vision_engine.rs` to register DirectML when requested, with CPU fallback:

```rust
fn load_session(path: &std::path::Path, placement: crate::pipeline::ComputePlacement) -> Result<Session> {
    let mut builder = Session::builder()
        .map_err(|e| anyhow!("failed to create session builder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow!("failed to set optimization level: {e}"))?
        .with_intra_threads(num_cpus::get_physical())
        .map_err(|e| anyhow!("failed to set intra threads: {e}"))?;

    if placement == crate::pipeline::ComputePlacement::Gpu {
        use ort::ep::DirectML;
        // If DirectML can't initialize, ort silently falls through to CPU.
        builder = builder
            .with_execution_providers([DirectML::default().build()])
            .map_err(|e| anyhow!("failed to register DirectML EP: {e}"))?;
    }

    builder
        .commit_from_file(path)
        .map_err(|e| anyhow!("failed to load ONNX model '{}': {e}", path.display()))
}
```

Thread a `ComputePlacement` parameter from `embed_images_batch`/`embed_image` callers (store the chosen placement on `VisionEngine` at construction, or pass it through). Simplest: add `placement: ComputePlacement` field to `VisionEngine`, set in `new`, and use it in `load_session` calls.

- [ ] **Step 4: Pass placement to the face analyzer**

In `get_face_analyzer` (`vision_engine.rs:30`), when building, add EPs for `Gpu`:

```rust
let eps: Vec<ort::ep::ExecutionProviderDispatch> = match self.placement {
    crate::pipeline::ComputePlacement::Gpu => vec![ort::ep::DirectML::default().build()],
    crate::pipeline::ComputePlacement::Cpu => vec![],
};
let analyzer = FaceAnalyzer::builder(det_path, rec_path, gender_age_path)
    .detector_input_size(preset.detector_input_size)
    .with_execution_providers(&eps)
    .build()
    .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))?;
```

- [ ] **Step 5: Build on Windows**

Run: `cd src-tauri && cargo build --release`
Expected: compiles. (DirectML links the DirectML EP shipped with the ORT binaries; `download-binaries` is on by default via `face_id`.)

- [ ] **Step 6: Benchmark CPU vs GPU placement**

Temporarily set `VisionEngine` placement to `Gpu`, run the bench (Task 9 will wire embed into bench), compare images/sec and CPU utilization against the `Cpu` baseline. Record both. **Keep whichever wins on this machine as the default in `PipelineConfig::default`.** If GPU is not faster but frees the CPU, prefer `Gpu` so face work runs concurrently (decided in Task 10).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/vision_engine.rs src-tauri/src/pipeline/mod.rs
git commit -m "feat(vision): add DirectML execution-provider option and PipelineConfig"
```

---

## Task 9: Pipeline coordinator (decode-once, merge the two workers)

**Files:**
- Create: `src-tauri/src/pipeline/embed_actor.rs`, `src-tauri/src/pipeline/face_actor.rs`
- Modify: `src-tauri/src/pipeline/mod.rs`, `src-tauri/src/embedder.rs`, `src-tauri/src/indexer.rs`, `src-tauri/src/lib.rs`, `src-tauri/examples/bench.rs`
- Test: `src-tauri/src/pipeline/mod.rs`

This is the largest task. It replaces `run_semantic_worker` + `run_subject_worker` with one coordinator: Stage 1 decodes once (rayon/blocking), Stage 2 runs batched embed + per-image face analyze via actors, Stage 3 writes DB + index + thumbnail + face crops from the single buffer.

- [ ] **Step 1: Embed actor — owns the embed batching loop**

Create `src-tauri/src/pipeline/embed_actor.rs`:

```rust
use crate::pipeline::DecodedImage;
use crate::models::{ModelManager, registry::ModelSpec};
use crate::vision_engine::VisionEngine;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// A request to embed one decoded image; reply carries the embedding vector.
pub struct EmbedRequest {
    pub decoded: DecodedImage,
    pub reply: oneshot::Sender<anyhow::Result<Vec<f32>>>,
}

/// Spawns the embed actor. It accumulates up to `batch_size` requests (or flushes
/// on a short timeout) and runs one batched `session.run`, owning the session for
/// the lifetime of the task so no per-call mutex contention occurs.
pub fn spawn_embed_actor(
    engine: Arc<VisionEngine>,
    manager: Arc<ModelManager>,
    spec: &'static ModelSpec,
    batch_size: usize,
) -> mpsc::Sender<EmbedRequest> {
    let (tx, mut rx) = mpsc::channel::<EmbedRequest>(batch_size * 2);
    tokio::spawn(async move {
        loop {
            // Block for the first item.
            let first = match rx.recv().await { Some(r) => r, None => break };
            let mut batch = vec![first];
            // Drain up to batch_size without waiting.
            while batch.len() < batch_size {
                match rx.try_recv() {
                    Ok(r) => batch.push(r),
                    Err(_) => break,
                }
            }

            let imgs: Vec<Arc<image::DynamicImage>> =
                batch.iter().map(|r| r.decoded.full.clone()).collect();
            let engine_c = engine.clone();
            let manager_c = manager.clone();
            let refs: Vec<&image::DynamicImage> = imgs.iter().map(|a| a.as_ref()).collect();

            // Inference is CPU/GPU heavy → run on blocking pool.
            let results = tokio::task::block_in_place(|| {
                engine_c.embed_images_batch(manager_c.as_ref(), &refs, spec)
            });

            match results {
                Ok(vecs) => {
                    for (req, v) in batch.into_iter().zip(vecs) {
                        let _ = req.reply.send(Ok(v));
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    for req in batch {
                        let _ = req.reply.send(Err(anyhow::anyhow!(msg.clone())));
                    }
                }
            }
        }
    });
    tx
}
```

- [ ] **Step 2: Face actor — owns the analyzer, per-image**

Create `src-tauri/src/pipeline/face_actor.rs`:

```rust
use crate::pipeline::DecodedImage;
use face_id::analyzer::FaceAnalyzer;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub struct FaceRequest {
    pub decoded: DecodedImage,
    pub reply: oneshot::Sender<anyhow::Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>>>,
}

/// Spawns the face actor. `analyze` is internally batched across faces in one
/// image; we process one image at a time (the analyzer serializes on internal
/// mutexes anyway) on the blocking pool.
pub fn spawn_face_actor(analyzer: Arc<FaceAnalyzer>) -> mpsc::Sender<FaceRequest> {
    let (tx, mut rx) = mpsc::channel::<FaceRequest>(8);
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let analyzer_c = analyzer.clone();
            let img = req.decoded.full.clone();
            let res = tokio::task::block_in_place(|| {
                analyzer_c
                    .analyze(img.as_ref())
                    .map(|faces| {
                        faces.into_iter()
                            .map(|f| (f.detection.bbox, f.embedding))
                            .collect::<Vec<_>>()
                    })
                    .map_err(|e| anyhow::anyhow!("{}", e))
            });
            let _ = req.reply.send(res);
        }
    });
    tx
}
```

> Verify `face_id::detector::BoundingBox` field names (`x1,y1,x2,y2`) against `embedder.rs:170-177`, which already uses them.

- [ ] **Step 3: Coordinator — wire the stages with bounded channels**

In `src-tauri/src/pipeline/mod.rs` add `pub mod embed_actor; pub mod face_actor;` and the coordinator. Reuse existing DB/queue/index helpers from `embedder.rs` (`f32_slice_to_bytes`, `mark_semantic_analysis_done`, `insert_face`, `mark_subject_analysis_done`, `mark_failed`, `emit_progress`, `get_queue_batch`).

```rust
use crate::models::ModelManager;
use crate::vision_engine::VisionEngine;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

/// Single coordinator replacing run_semantic_worker + run_subject_worker.
/// Pulls a merged batch of pending image ids, decodes each once, fans the buffer
/// out to embed + face actors, then writes DB rows, index entries, thumbnail and
/// face crops from that same buffer.
#[allow(clippy::too_many_arguments)]
pub async fn run_pipeline(
    pool: sqlx::SqlitePool,
    app: AppHandle,
    engine: Arc<VisionEngine>,
    manager: Arc<ModelManager>,
    index: crate::vector_index::IndexStore,
    data_dir: std::path::PathBuf,
    config: PipelineConfig,
) {
    let spec = &crate::models::registry::SIGLIP_BASE;
    let preset = &crate::models::registry::BUFFALO_S_PRESET;

    if let Err(e) = manager.ensure_ready(&app, spec).await {
        eprintln!("[pipeline] embed model not ready: {e}");
    }
    let analyzer = match engine.get_face_analyzer(&manager, preset).await {
        Ok(a) => a,
        Err(e) => { eprintln!("[pipeline] face analyzer init failed: {e}"); return; }
    };

    let embed_tx = embed_actor::spawn_embed_actor(
        engine.clone(), manager.clone(), spec, config.batch_size);
    let face_tx = face_actor::spawn_face_actor(analyzer);

    loop {
        // Merge the two queues: process any image pending in EITHER queue.
        let batch = match crate::db::get_queue_batch(&pool, "semantic", config.batch_size as i64).await {
            Ok(b) if !b.is_empty() => b,
            _ => match crate::db::get_queue_batch(&pool, "subject", config.batch_size as i64).await {
                Ok(b) => b,
                Err(_) => { tokio::time::sleep(Duration::from_secs(2)).await; continue; }
            }
        };
        if batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        // Stage 1: decode once (bounded concurrency on blocking pool).
        let mut decoded = Vec::with_capacity(batch.len());
        for (queue_id, image_id, attempts) in batch {
            let image = match crate::db::get_image_by_id(&pool, image_id).await {
                Ok(Some(i)) => i, _ => continue,
            };
            let path = image.path.clone();
            let res = tokio::task::spawn_blocking(move || {
                decoded_image::load_decoded(image_id, std::path::Path::new(&path))
            }).await;
            match res {
                Ok(Ok(d)) => decoded.push((queue_id, image_id, attempts, d)),
                Ok(Err(e)) => {
                    let _ = crate::db::mark_failed(&pool, queue_id, attempts, &e.to_string()).await;
                }
                Err(e) => {
                    let _ = crate::db::mark_failed(&pool, queue_id, attempts, &e.to_string()).await;
                }
            }
        }

        // Stage 2: dispatch embed + face for each decoded image.
        for (queue_id, image_id, _attempts, d) in decoded {
            // Embed
            let (etx, erx) = oneshot::channel();
            let _ = embed_tx.send(embed_actor::EmbedRequest { decoded: d.clone(), reply: etx }).await;
            // Face
            let (ftx, frx) = oneshot::channel();
            let _ = face_tx.send(face_actor::FaceRequest { decoded: d.clone(), reply: ftx }).await;

            // Stage 3: write results.
            if let Ok(Ok(emb)) = erx.await {
                let blob = crate::embedder::f32_slice_to_bytes(&emb);
                if crate::db::mark_semantic_analysis_done(&pool, queue_id, image_id, &blob).await.is_ok() {
                    index.write().unwrap().add(image_id, &emb);
                }
            }
            if let Ok(Ok(faces)) = frx.await {
                for (bbox, face_emb) in faces {
                    let face_blob = crate::embedder::f32_slice_to_bytes(&face_emb);
                    let _ = crate::db::insert_face(
                        &pool, image_id, None,
                        (bbox.x1 as f64, bbox.y1 as f64,
                         (bbox.x2 - bbox.x1) as f64, (bbox.y2 - bbox.y1) as f64),
                        Some(&face_blob),
                    ).await;
                }
                let _ = crate::db::mark_subject_analysis_done(&pool, queue_id, image_id).await;
            }

            // Thumbnail from the same buffer.
            let thumb_path = crate::thumbnail::thumbnail_path_for(&data_dir, image_id);
            let d_thumb = d.clone();
            let _ = tokio::task::spawn_blocking(move || {
                crate::thumbnail::write_thumbnail_from_image(d_thumb.full.as_ref(), &thumb_path)
            }).await;
            let _ = app.emit("image_updated",
                crate::models::ImageUpdatedPayload { image_id });
        }

        crate::embedder::emit_progress(&pool, &app).await;

        // Persist index snapshot after each batch.
        let snap_path = data_dir.join("nebula.idx");
        let index_snap = Arc::clone(&index);
        tokio::task::spawn_blocking(move || {
            let guard = index_snap.read().unwrap();
            if let Err(e) = guard.save(&snap_path) {
                eprintln!("[pipeline] failed to save index snapshot: {e}");
            }
        }).await.ok();

        // Auto-recluster after a batch that produced faces.
        if let Ok(result) = crate::clustering::cluster_unassigned_faces(&pool).await {
            let _ = app.emit("subjects_updated", ());
            let _ = result;
        }
    }
}
```

> The exact `get_queue_batch` return tuple is `(queue_id, image_id, attempts)` (see `embedder.rs:247`). Keep `update_thumbnail_path` if the schema requires a thumbnail path row — call `crate::db::update_thumbnail_path` after writing the thumbnail, mirroring `indexer.rs:299`.

- [ ] **Step 4: Remove the old workers and the scan-time thumbnail**

In `src-tauri/src/embedder.rs`, delete `run_semantic_worker` and `run_subject_worker` (keep all helper fns: `f32_slice_to_bytes`, `bytes_to_f32_vec`, `cosine_similarity`, `emit_progress`). In `src-tauri/src/indexer.rs`, remove the `self.spawn_thumbnail(...)` calls (lines ~211, ~275) and the `spawn_thumbnail` method, since thumbnails now come from the pipeline. Remove the now-unused `thumb_semaphore`.

- [ ] **Step 5: Spawn the coordinator in `lib.rs`**

In `src-tauri/src/lib.rs`, replace the two `tokio::async_runtime::spawn` blocks that start `run_semantic_worker` and `run_subject_worker` with one:

```rust
let pool_pipe = pool.clone();
let app_pipe = app.handle().clone();
let ve_pipe = Arc::clone(&vision_engine);
let mm_pipe = Arc::clone(&model_manager);
let index_pipe = index.clone();
let data_dir_pipe = data_dir.clone();
tauri::async_runtime::spawn(async move {
    pipeline::run_pipeline(
        pool_pipe, app_pipe, ve_pipe, mm_pipe, index_pipe, data_dir_pipe,
        pipeline::PipelineConfig::default(),
    ).await;
});
```

- [ ] **Step 6: Wire embed+face into the bench harness**

Update `examples/bench.rs` to also embed each decoded image (batched) and analyze faces, recording `embed avg ms` and `face avg ms`, by constructing a `VisionEngine` + `ModelManager` with `NEBULA_TEST_DATA_DIR` pointing at the app data dir. (Mirror the smoke-test setup from Task 6 Step 5.)

- [ ] **Step 7: Build, test, run app**

Run: `cd src-tauri && cargo build --release && cargo test --lib`
Expected: compiles; all unit tests pass/skip.
Then: `pnpm tauri dev`, add a folder of images, confirm thumbnails, embeddings (search works), and faces/subjects all populate.

- [ ] **Step 8: Benchmark against baseline**

Run the bench on the same sample folder used in Task 1. Expected: images/sec materially higher than baseline (decode-once + split towers + batching). Record the new number.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/pipeline/ src-tauri/src/embedder.rs src-tauri/src/indexer.rs src-tauri/src/lib.rs src-tauri/examples/bench.rs
git commit -m "feat(pipeline): unified decode-once staged pipeline with embed/face actors"
```

---

## Task 10: Resource arbitration + tuning pass

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`
- Test: manual benchmark sweep

- [ ] **Step 1: Make Stage 1 decode concurrency bounded and parallel**

Replace the serial decode loop in `run_pipeline` (Task 9 Step 3) with bounded-concurrency decoding using a `tokio::sync::Semaphore` of size `config.load_channel_depth`, spawning `spawn_blocking` decodes and collecting results — so decoding overlaps inference instead of running before it. Keep the semaphore so in-flight 24 MP buffers stay capped.

```rust
let sem = Arc::new(tokio::sync::Semaphore::new(config.load_channel_depth));
let mut handles = Vec::new();
for (queue_id, image_id, attempts) in batch {
    let pool_c = pool.clone();
    let permit = sem.clone().acquire_owned().await.unwrap();
    handles.push(tokio::spawn(async move {
        let _permit = permit;
        let image = crate::db::get_image_by_id(&pool_c, image_id).await.ok().flatten()?;
        let path = image.path.clone();
        let d = tokio::task::spawn_blocking(move || {
            decoded_image::load_decoded(image_id, std::path::Path::new(&path))
        }).await.ok()?.ok()?;
        Some((queue_id, image_id, attempts, d))
    }));
}
let mut decoded = Vec::new();
for h in handles { if let Ok(Some(x)) = h.await { decoded.push(x); } }
```

- [ ] **Step 2: Placement-aware concurrency for Stage 2**

When `config.placement == Gpu`, dispatch embed and face for an image **concurrently** (embed on iGPU, face on CPU) — the Task 9 code already sends both before awaiting, which achieves this. When `placement == Cpu`, await the embed result before sending the face request, to avoid both model stages thrashing the CPU at once:

```rust
if config.placement == ComputePlacement::Cpu {
    // serialize: embed, then face
    let emb = erx.await;
    // ...write emb...
    let (ftx, frx) = oneshot::channel();
    let _ = face_tx.send(/* ... */).await;
    let faces = frx.await;
    // ...write faces...
} else {
    // concurrent (as written in Task 9)
}
```

Factor the DB-write logic into a small local async helper to avoid duplication between the two branches.

- [ ] **Step 3: Tune batch size and channel depth**

Sweep `batch_size` ∈ {4, 8, 12, 16, 24} and `load_channel_depth` ∈ {8, 16, 24, 32} via the bench harness (read them from env vars in `bench.rs`). Pick the pair with the best images/sec at acceptable memory. Set those as `PipelineConfig::default`.

- [ ] **Step 4: Final end-to-end benchmark + record results**

Run the bench on the same sample folder. Record final images/sec and per-stage averages vs. the Task 1 baseline in the PR description.

- [ ] **Step 5: Build, full test, manual app run**

Run: `cd src-tauri && cargo build --release && cargo test --lib`
Expected: green. Manual run: process a 300–1000 image burst, confirm no memory blowup (watch Task Manager) and the UI stays responsive.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs src-tauri/examples/bench.rs
git commit -m "feat(pipeline): bounded parallel decode + placement-aware scheduling + tuning"
```

---

## Self-Review (completed during planning)

- **Spec coverage:** decode-once (Tasks 2,3,9) ✓; remove session-mutex serialization via actor (Tasks 6,9) ✓; split towers (Tasks 4,6) ✓; batched inference (Task 7) ✓; EP/DirectML + auto thread count (Task 8) ✓; vectorized preprocess + faster resize + (note) scaled decode (Tasks 3,5) ✓; scheduling/backpressure + placement-aware concurrency (Task 10) ✓; benchmark harness first + kept (Tasks 1,9,10) ✓; tests unit/integration/regression (per-task tests + manual app run) ✓; rollout order matches spec ✓.
- **Scaled JPEG decode (spec 2.5):** the resize-filter change is implemented (Triangle/CatmullRom); DCT-domain scaled decode is left as an optional follow-up because `image` 0.25's `jpeg` decoder scaled-decoding API should be confirmed before relying on it — flagged here rather than written as unverified code.
- **Placeholder scan:** Task 0 contains intentional record-the-value blanks (upstream facts to confirm), not code placeholders. All code steps contain complete code.
- **Type consistency:** `DecodedImage.full: Arc<DynamicImage>`, `embed_images_batch(&[&DynamicImage])`, `EmbedRequest`/`FaceRequest` with `oneshot` replies, `PipelineConfig`/`ComputePlacement` used consistently across Tasks 8–10; `model_file_path` introduced in Task 4 and used in Tasks 6–7.

---

## Risks & Watch-points

- **onnx-community tensor names** may differ (`image_embeds` vs `pooler_output`) — Task 0 nails them; Tasks 6–7 read them from `ModelSpec`, so no code change is needed if they differ.
- **DirectML on the iGPU** may not beat CPU; Task 8 keeps a CPU fallback and Task 10 only prefers GPU if it helps end-to-end.
- **`face_id` internal mutexes** serialize face inference; the face actor is honest about this — throughput gains for faces come from decode-once and CPU freed by GPU embedding, not from face parallelism.
- **Memory during bursts:** bounded decode semaphore (Task 10) caps in-flight buffers; if RAM is still high, lower `load_channel_depth`.
- **Embedding-dim change:** combined model output dim must match the split vision tower (768 for SigLIP base). Task 6 Step 5 asserts it. Since nothing is permanently indexed, the one-time re-embed from the resize-filter change is a non-issue.
```
