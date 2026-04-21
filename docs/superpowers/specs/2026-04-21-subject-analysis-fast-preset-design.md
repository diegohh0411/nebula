# Subject Analysis "Fast" Preset

## Problem

Subject analysis (face detection + ArcFace embedding) runs entirely on CPU and uses the default `FaceAnalyzer::from_hf().build()` configuration with a 640x640 detector input and all three sub-models (detector, embedder, gender/age). This is slower than necessary for consumer CPUs, especially on large photo libraries.

Additionally, gender/age estimation is computed for every face but never stored or used by Nebula — pure wasted compute.

## Goal

Add a "Fast" preset for subject analysis that mirrors the existing SigLIP `embedding_model` settings pattern, reducing per-image processing time on CPU.

## Design

### Settings

Add a `subject_model` setting to the `settings` table with two values:

| Preset | Value | Detector Input | Gender/Age |
|--------|-------|---------------|------------|
| Standard | `standard` | 640x640 | Computed (current behavior) |
| Fast | `fast` | 640x640 | Skipped |

Default: `standard` (preserves current behavior for existing installs).

### Backend Changes

#### 1. `vision_engine.rs` — Configurable FaceAnalyzer

Replace `face_analyzer: OnceCell<FaceAnalyzer>` with a swap-able store that detects preset changes:

```rust
face_analyzer: std::sync::Mutex<Option<(String, FaceAnalyzer)>>
```

`get_face_analyzer()` reads the current `subject_model` setting, compares to the stored preset, and rebuilds the analyzer if it changed. This mirrors the existing `get_session()` pattern for SigLIP.

For the **Fast** preset, the builder uses:
```rust
FaceAnalyzer::from_hf()
    .detector_input_size((640, 640))
    .detector_model(HfModel {
        id: "RuteNL/SCRFD-face-detection-ONNX".to_string(),
        file: "10g_bnkps.onnx".to_string(),
    })
    .build()
    .await
```

For **Standard**, the builder uses defaults (no custom params).

#### 2. `embedder.rs` — Skip gender/age in Fast mode

`process_subject_one()` reads the `subject_model` setting and branches:

**Standard path** (unchanged):
```rust
analyzer.analyze(&dynamic_img) // runs detection + embedding + gender/age
```

**Fast path** — bypass `analyze()`, call detector + embedder directly:
```rust
// 1. Detect faces
let detections = analyzer.detector.lock().unwrap().detect(&img)?;
// 2. Align each face using landmarks (copy alignment logic from FaceAnalyzer::analyze)
// 3. Compute embeddings via analyzer.embedder.lock().unwrap().compute_embeddings_batch(&crops)
// 4. Return (detection, embedding) pairs — no gender/age
```

The alignment logic (~20 lines) is extracted from `FaceAnalyzer::analyze()` in the `face_id` crate source. It uses `norm_crop(&rgb_img, &landmarks_array, 112)` which is available from `face_id::face_align::norm_crop`.

#### 3. `settings.rs` — Subject model setting

Add `get_available_subject_models()` command:
```rust
vec![
    ModelInfo { id: "standard".into(), name: "Standard".into(), description: "Full accuracy (640x640 detection)".into() },
    ModelInfo { id: "fast".into(), name: "Fast".into(), description: "Optimized for consumer CPUs (smaller detector, no gender/age)".into() },
]
```

In `update_setting()`, handle `subject_model` changes:
- If value changed, call `db::reset_all_subject_data(pool)` to clear faces + subjects + re-queue all images for the subject pipeline
- This matches the `embedding_model` change pattern (reset + re-queue)

#### 4. `db.rs` — Subject reset + default setting

Add a default for `subject_model = "standard"` in the settings initialization.

Add `reset_all_subject_data(pool)` that:
- Deletes all rows from `faces`, `subjects`, `face_corrections`, `merge_suggestions`
- Re-enqueues all images with `pipeline = 'subject'`
- Resets `subject_analysis_done = 0` on all images

#### 5. Frontend — Settings UI

Add a second model picker to the settings component for "Face Analysis" with the same dropdown pattern as the existing "Semantic Search" model picker. When changed, triggers `update_setting("subject_model", value)`.

### What Does NOT Change

- **Clustering** (`clustering.rs`) — untouched, same anchor-centroid + HDBSCAN logic
- **Thumbnails** (`thumbnail.rs`) — untouched, same face crop logic
- **Embedding model** — same `w600k_r50.onnx` ArcFace in both presets (always 512-dim)
- **Database schema** — no schema changes needed

### Performance Expectations

| Metric | Standard | Fast |
|--------|----------|------|
| Detector input | 640x640 | 320x320 |
| Detector compute | ~4x baseline | ~1x |
| Gender/age inference | Yes | No |
| Embedding inference | Same | Same |
| Total per-image speedup | — | ~2-3x |
| Memory | 3 ONNX sessions | 3 sessions (gender/age loaded but unused) |
| Face detection quality | Full | May miss small/distant faces |

### Edge Cases

- **No landmarks**: The fast path still uses `34g_gnkps.onnx` which always produces landmarks. No fallback needed.
- **Zero faces detected**: Both paths return empty vec — handled identically.
- **Preset switch mid-processing**: The subject worker reads the preset per-batch. In-flight images finish with the old analyzer. The next batch uses the new one. After the setting change, `reset_all_subject_data` re-queues everything, so all faces get re-analyzed with the new preset.
