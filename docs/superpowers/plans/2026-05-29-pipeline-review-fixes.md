# Pipeline PR Code Review Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve all code review blockers, quick wins, and the root-cause batching bug before merging the decode-once actor pipeline PR.

**Architecture:** Four targeted changes across four files. No new abstractions. The batching fix splits the Stage 2 CPU loop into two phases: a pre-dispatch pass (all embed requests sent before any are awaited) and a result-handling pass (face dispatched + `join!(embed, face)` per image). This unifies the CPU and GPU paths into a single loop.

**Tech Stack:** Rust, Tokio, ONNX Runtime (ORT), SQLite/SQLx

---

## Note on Issue #1 — pooler_output ("BLOCKING" in review)

`registry.rs:119-120` already contains: `// Confirmed by inspecting the ONNX graph: onnx-community exports all SigLIP2 variants with pooler_output (not image_embeds/text_embeds).` — added in commit `d839260`. For SigLIP2, `pooler_output` is the shared-space cross-modal projection; this is correct. Task 5 adds the missing regression test.

---

## Files

| File | Tasks |
|---|---|
| `src-tauri/src/pipeline/mod.rs` | T1 (thumbnail bug), T4 (batching fix) |
| `src-tauri/src/thumbnail.rs` | T2 (remove dead code) |
| `src-tauri/src/lib.rs` + `src-tauri/examples/bench.rs` | T3 (extend bench) |
| `src-tauri/src/vision_engine.rs` | T5 (cross-modal test) |

---

### Task 1: Fix thumbnail path recorded on write failure

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs:369-373`

- [x] **Step 1: Apply the fix**

Current code at lines 369-373:
```rust
let _ = tokio::task::spawn_blocking(move || {
    crate::thumbnail::write_thumbnail_from_image(d_thumb.full.as_ref(), &thumb_path)
})
.await;
let _ = crate::db::update_thumbnail_path(&pool, image_id, &thumb_path_str).await;
```

Replace with:
```rust
let write_ok = tokio::task::spawn_blocking(move || {
    crate::thumbnail::write_thumbnail_from_image(d_thumb.full.as_ref(), &thumb_path)
})
.await
.map(|r| r.is_ok())
.unwrap_or(false);
if write_ok {
    let _ = crate::db::update_thumbnail_path(&pool, image_id, &thumb_path_str).await;
}
```

- [x] **Step 2: Build and test**

```bash
cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -5
```
Expected: no errors.

```bash
cargo test 2>&1 | tail -5
```
Expected: 29 tests pass.

- [x] **Step 3: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "fix(pipeline): gate update_thumbnail_path on write success"
```

---

### Task 2: Remove dead code `write_face_crop_from_image`

**Files:**
- Modify: `src-tauri/src/thumbnail.rs:64-87` (function), `thumbnail.rs:112-122` (test)

The function was introduced to avoid re-opening the source file for face crops during the pipeline run. The coordinator never calls it — face crops are still generated lazily by `generate_face_crop` → file reopen. YAGNI: remove.

- [x] **Step 1: Delete the function and its test**

In `src-tauri/src/thumbnail.rs`, delete lines 64–87 (the `#[allow(dead_code)]` attribute + full `write_face_crop_from_image` function body) and lines 112–122 (the `face_crop_from_image_is_square_200` test). The file's remaining test `thumbnail_from_image_fits_within_box_and_writes_file` is untouched.

- [x] **Step 2: Build and test**

```bash
cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -5
cargo test 2>&1 | tail -5
```
Expected: clean build, 28 tests pass (one test removed).

- [x] **Step 3: Commit**

```bash
git add src-tauri/src/thumbnail.rs
git commit -m "chore: remove unused write_face_crop_from_image"
```

---

### Task 3: Extend benchmark to embed stage + record baseline

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/examples/bench.rs`

The bench currently times decode only. We extend it to time embed **before** applying the batching fix so we have a before/after measurement.

- [x] **Step 1: Expose needed modules in lib.rs**

In `src-tauri/src/lib.rs`, change three `mod` declarations to `pub mod`:

```rust
// change these three lines from `mod` to `pub mod`:
pub mod models;
pub mod pipeline;
pub mod vision_engine;
```

- [x] **Step 2: Replace bench.rs contents**

Replace the full contents of `src-tauri/examples/bench.rs` with:

```rust
//! End-to-end throughput benchmark.
//!
//! Decode only:
//!   NEBULA_BENCH_DIR=/path/to/images cargo run --release --example bench
//!
//! Decode + embed:
//!   NEBULA_BENCH_DIR=/path/to/images NEBULA_DATA_DIR=/path/to/app-data \
//!     cargo run --release --example bench

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

    // Embed stage is optional — enabled when NEBULA_DATA_DIR is set to the
    // directory that contains the `models/` subdirectory (the app's data dir).
    let embed_ctx = std::env::var("NEBULA_DATA_DIR").ok().map(|d| {
        let data_dir = PathBuf::from(d);
        let engine = nebula_lib::vision_engine::VisionEngine::new(
            data_dir.clone(),
            nebula_lib::pipeline::ComputePlacement::Cpu,
        );
        let manager = nebula_lib::models::ModelManager::new(data_dir);
        (engine, manager)
    });
    let spec = &nebula_lib::models::registry::SIGLIP_BASE;

    let mut decode = Stage::default();
    let mut embed = Stage::default();

    let wall = Instant::now();
    for path in &images {
        let t = Instant::now();
        let img = image::open(path).expect("decode");
        decode.add(t.elapsed().as_secs_f64() * 1000.0);

        if let Some((engine, manager)) = &embed_ctx {
            let t = Instant::now();
            match engine.embed_image(manager, &img, spec) {
                Ok(_) => embed.add(t.elapsed().as_secs_f64() * 1000.0),
                Err(e) => { eprintln!("embed failed (stopping embed stage): {e}"); break; }
            }
        }
    }
    let secs = wall.elapsed().as_secs_f64();

    println!("--- bench results ---");
    println!("images:        {}", images.len());
    println!("decode avg ms: {:.1}", decode.avg());
    if embed.count > 0 {
        println!("embed avg ms:  {:.1}", embed.avg());
    } else {
        println!("embed:         (set NEBULA_DATA_DIR to enable)");
    }
    println!("wall secs:     {:.2}", secs);
    println!("images/sec:    {:.2}", images.len() as f64 / secs);
}
```

- [x] **Step 3: Build the example**

```bash
cd /home/pi/nebula/src-tauri && cargo build --release --example bench 2>&1 | tail -10
```
Expected: compiles clean.

- [ ] **Step 4: Record baseline numbers (BEFORE applying Task 4)**

```bash
NEBULA_BENCH_DIR=/path/to/test/images \
NEBULA_DATA_DIR=/path/to/nebula/data \
  cargo run --release --example bench
```

**Write down the output here** — these numbers are the before-batching-fix baseline.

- [x] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/examples/bench.rs
git commit -m "feat(bench): add embed stage timing; pub mod models/pipeline/vision_engine"
```

---

### Task 4: Fix Stage 2 batching — two-phase embed dispatch

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs:192-363`

**Root cause:** The CPU path at lines 198-242 sends one embed request then immediately `.await`s the reply before the next image. The embed actor's `try_recv` batching loop therefore never sees more than one item in the channel — every "batch" is size 1. The fix: pre-dispatch all N embed requests in one pass (Phase A), then handle face + join with embed results in a second pass (Phase B). This also eliminates the CPU/GPU branch entirely — both placements use the same code.

- [x] **Step 1: Replace Stage 2 (lines 192-363) with the two-phase loop**

Delete the existing block from `// Stage 2 & 3: dispatch embed + face, write results` (line 192) through the closing `}` of the placement `if/else` (line 363), and replace with:

```rust
        // Stage 2: dispatch embed + face, write results
        let mut processed_subject_work = false;

        // Phase A — pre-dispatch all embed requests before awaiting any reply.
        // This fills the embed actor's channel so its try_recv loop drains a
        // real batch (up to batch_size) instead of processing images one-by-one.
        struct Pending {
            image_id: i64,
            sem_entry: Option<(i64, i32)>,
            sub_entry: Option<(i64, i32)>,
            d: DecodedImage,
            erx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<Vec<f32>>>>,
        }
        let mut pending: Vec<Pending> = Vec::with_capacity(decoded.len());
        for (image_id, sem_entry, sub_entry, d) in decoded {
            let erx = if let Some((sem_qid, sem_attempts)) = sem_entry {
                let (etx, erx) = tokio::sync::oneshot::channel();
                if embed_tx.send(embed_actor::EmbedRequest { decoded: d.clone(), reply: etx }).await.is_ok() {
                    Some(erx)
                } else {
                    let _ = crate::db::mark_failed(&pool, sem_qid, sem_attempts, "embed actor closed").await;
                    None
                }
            } else {
                None
            };
            pending.push(Pending { image_id, sem_entry, sub_entry, d, erx });
        }

        // Phase B — for each image: dispatch face then join!(embed_result, face_result).
        // This restores the embed/face overlap that the old serial CPU path dropped,
        // while the pre-dispatched embed batch is processed by the actor.
        for Pending { image_id, sem_entry, sub_entry, d, erx } in pending {
            let img_w = d.full.width() as f64;
            let img_h = d.full.height() as f64;

            let frx = if let Some((sub_qid, sub_attempts)) = sub_entry {
                let (ftx, frx) = tokio::sync::oneshot::channel();
                if face_tx.send(face_actor::FaceRequest { decoded: d.clone(), reply: ftx }).await.is_ok() {
                    Some(frx)
                } else {
                    let _ = crate::db::mark_failed(&pool, sub_qid, sub_attempts, "face actor closed").await;
                    None
                }
            } else {
                None
            };

            match (erx, frx) {
                (Some(erx), Some(frx)) => {
                    let (emb_result, face_result) = tokio::join!(erx, frx);
                    match emb_result {
                        Ok(Ok(emb)) => {
                            let blob = crate::embedder::f32_slice_to_bytes(&emb);
                            if let Some((sem_qid, _)) = sem_entry {
                                if crate::db::mark_semantic_analysis_done(&pool, sem_qid, image_id, &blob)
                                    .await.is_ok()
                                {
                                    index.write().unwrap().add(image_id, &emb);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            if let Some((sem_qid, sem_attempts)) = sem_entry {
                                let _ = crate::db::mark_failed(&pool, sem_qid, sem_attempts, &e.to_string()).await;
                            }
                        }
                        Err(_) => {
                            if let Some((sem_qid, sem_attempts)) = sem_entry {
                                let _ = crate::db::mark_failed(&pool, sem_qid, sem_attempts, "embed reply channel dropped").await;
                            }
                        }
                    }
                    match face_result {
                        Ok(Ok(faces)) => {
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                write_faces(&pool, image_id, sub_qid, sub_attempts, img_w, img_h, faces).await;
                                processed_subject_work = true;
                            }
                        }
                        Ok(Err(e)) => {
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                let _ = crate::db::mark_failed(&pool, sub_qid, sub_attempts, &e.to_string()).await;
                            }
                        }
                        Err(_) => {
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                let _ = crate::db::mark_failed(&pool, sub_qid, sub_attempts, "face reply channel dropped").await;
                            }
                        }
                    }
                }
                (Some(erx), None) => {
                    match erx.await {
                        Ok(Ok(emb)) => {
                            let blob = crate::embedder::f32_slice_to_bytes(&emb);
                            if let Some((sem_qid, _)) = sem_entry {
                                if crate::db::mark_semantic_analysis_done(&pool, sem_qid, image_id, &blob)
                                    .await.is_ok()
                                {
                                    index.write().unwrap().add(image_id, &emb);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            if let Some((sem_qid, sem_attempts)) = sem_entry {
                                let _ = crate::db::mark_failed(&pool, sem_qid, sem_attempts, &e.to_string()).await;
                            }
                        }
                        Err(_) => {
                            if let Some((sem_qid, sem_attempts)) = sem_entry {
                                let _ = crate::db::mark_failed(&pool, sem_qid, sem_attempts, "embed reply channel dropped").await;
                            }
                        }
                    }
                }
                (None, Some(frx)) => {
                    match frx.await {
                        Ok(Ok(faces)) => {
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                write_faces(&pool, image_id, sub_qid, sub_attempts, img_w, img_h, faces).await;
                                processed_subject_work = true;
                            }
                        }
                        Ok(Err(e)) => {
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                let _ = crate::db::mark_failed(&pool, sub_qid, sub_attempts, &e.to_string()).await;
                            }
                        }
                        Err(_) => {
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                let _ = crate::db::mark_failed(&pool, sub_qid, sub_attempts, "face reply channel dropped").await;
                            }
                        }
                    }
                }
                (None, None) => {}
            }

            // Thumbnail from same buffer — unchanged from original
            let thumb_path = crate::thumbnail::thumbnail_path_for(&data_dir, image_id);
            let thumb_path_str = thumb_path.to_string_lossy().to_string();
            let d_thumb = d.clone();
            let write_ok = tokio::task::spawn_blocking(move || {
                crate::thumbnail::write_thumbnail_from_image(d_thumb.full.as_ref(), &thumb_path)
            })
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
            if write_ok {
                let _ = crate::db::update_thumbnail_path(&pool, image_id, &thumb_path_str).await;
            }
            let _ = app.emit(
                "image_updated",
                crate::models::ImageUpdatedPayload { image_id },
            );
        }
```

Note: the thumbnail fix from Task 1 is **folded into** Task 4's replacement block above. If Task 1 was already committed separately, adjust the thumbnail lines in this block to match what was already applied (or just apply the thumbnail fix inside this block and the commit from T1 becomes a no-op diff).

The original GPU-specific `else` branch (old lines 243-363) is gone — the two-phase loop handles both `Cpu` and `Gpu` placements. The `if config.placement == ComputePlacement::Cpu` discriminant is removed entirely. The thumbnail + index-snapshot + recluster section that followed (old lines 380-398) remains unchanged after the inner loop closes.

- [x] **Step 2: Build**

```bash
cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -10
```
Expected: clean. If compiler flags `config.placement` as unused after the branch is removed, remove that field from the `PipelineConfig` default or keep it for future use with `#[allow(dead_code)]`.

- [x] **Step 3: Run tests**

```bash
cargo test 2>&1 | tail -10
```
Expected: 28 tests pass.

- [ ] **Step 4: Run bench — record after numbers**

```bash
NEBULA_BENCH_DIR=/path/to/test/images \
NEBULA_DATA_DIR=/path/to/nebula/data \
  cargo run --release --example bench
```

Compare embed avg ms and images/sec to the Task 3 baseline. Embed avg ms should drop as the actor now processes real batches of ≤12 instead of 1.

- [x] **Step 5: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "perf(pipeline): pre-dispatch embed batch; join embed+face per image (unifies CPU/GPU paths)"
```

---

### Task 5: Add cross-modal compatibility test

**Files:**
- Modify: `src-tauri/src/vision_engine.rs` (inside `#[cfg(test)] mod tests`)

Adds a test that verifies the text and vision towers produce embeddings with positive cosine similarity on a relevant pair. Uses the existing `NEBULA_TEST_DATA_DIR` graceful-skip pattern.

- [x] **Step 1: Add the test**

Inside the `#[cfg(test)] mod tests` block in `vision_engine.rs`, append after the last existing test:

```rust
#[test]
fn text_and_image_embeddings_are_cross_modal_compatible() {
    let data_dir = match std::env::var("NEBULA_TEST_DATA_DIR") {
        Ok(d) => std::path::PathBuf::from(d),
        Err(_) => { eprintln!("skipping: NEBULA_TEST_DATA_DIR not set"); return; }
    };
    let manager = crate::models::ModelManager::new(data_dir.clone());
    let spec = &crate::models::registry::SIGLIP_BASE;
    let vf = spec.vision_file.as_ref().unwrap();
    let tf = spec.text_file.as_ref().unwrap();
    if !manager.model_file_path(spec, vf).exists() || !manager.model_file_path(spec, tf).exists() {
        eprintln!("skipping: split towers not downloaded");
        return;
    }
    let engine = VisionEngine::new(data_dir, crate::pipeline::ComputePlacement::Cpu);

    let red = image::DynamicImage::ImageRgb8(
        image::RgbImage::from_pixel(224, 224, image::Rgb([220, 30, 30]))
    );
    let blue = image::DynamicImage::ImageRgb8(
        image::RgbImage::from_pixel(224, 224, image::Rgb([30, 30, 220]))
    );
    let img_red = engine.embed_image(&manager, &red, spec).unwrap();
    let img_blue = engine.embed_image(&manager, &blue, spec).unwrap();
    let txt_red = engine.embed_text(&manager, "a red square", spec).unwrap();
    let txt_blue = engine.embed_text(&manager, "a blue square", spec).unwrap();

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb + 1e-8)
    }

    let sim_red_red   = cosine(&img_red,  &txt_red);
    let sim_red_blue  = cosine(&img_red,  &txt_blue);
    let sim_blue_blue = cosine(&img_blue, &txt_blue);

    assert!(sim_red_red > 0.0,
        "red image vs 'red square' text: expected positive cosine, got {sim_red_red}");
    assert!(sim_red_red > sim_red_blue,
        "red image should rank higher for 'red square' ({sim_red_red:.3}) than 'blue square' ({sim_red_blue:.3})");
    assert!(sim_blue_blue > sim_red_blue,
        "blue image should rank higher for 'blue square' ({sim_blue_blue:.3}) than 'red square' ({sim_red_blue:.3})");
}
```

- [x] **Step 2: Build and test (without models — should skip gracefully)**

```bash
cd /home/pi/nebula/src-tauri && cargo test 2>&1 | tail -10
```
Expected: 29 tests pass (28 remaining + 1 new; new one prints "skipping" and passes vacuously).

- [ ] **Step 3: Run the test with models present**

```bash
NEBULA_TEST_DATA_DIR=/path/to/nebula/data \
  cargo test text_and_image -- --nocapture
```
Expected: passes with non-trivial similarity scores. If it fails, `pooler_output` may not be the shared-space projection for these exports — investigate and open a follow-up.

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "test(vision): cross-modal pooler_output compatibility guard"
```

---

## Verification

Before marking ready for re-review:

1. `cargo check` — clean
2. `cargo test` — 29 tests pass
3. Bench before/after numbers recorded — embed avg ms and images/sec improved after T4
4. `NEBULA_TEST_DATA_DIR=... cargo test text_and_image -- --nocapture` — passes
5. Manual: index ~20 images in the running app, run a text search, confirm relevant results appear at top
