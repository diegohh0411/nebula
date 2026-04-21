# FAST Preset: Use Smaller Detector Model

## Problem

The FAST preset currently only skips gender/age inference (~5-10% savings). The real bottleneck is the 39.4 MB SCRFD detector model running on every image. We need an actually faster detector.

## Solution

FAST preset uses the `10g_bnkps.onnx` detector (16.9 MB, ~2.3x smaller) from `RuteNL/SCRFD-face-detection-ONNX`, which still provides keypoints for face alignment. Combined with skipping gender/age, this gives meaningful per-image speedup.

## Architecture

Add two `&'static str` fields to `SubjectPreset` (can't use `HfModel` directly since it contains `String`):

```rust
pub struct SubjectPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub detector_input_size: (u32, u32),
    pub skip_gender_age: bool,
    pub detector_hf_id: &'static str,
    pub detector_hf_file: &'static str,
}
```

- **STANDARD**: `detector_hf_id = ""`, `detector_hf_file = ""` — empty strings mean "use face_id defaults"
- **FAST**: `detector_hf_id = "RuteNL/SCRFD-face-detection-ONNX"`, `detector_hf_file = "10g_bnkps.onnx"`, `skip_gender_age = true`

`build_face_analyzer` constructs `HfModel` at runtime from these fields and passes `.detector_model()` to the builder only when non-empty.

## Files Changed

- `src-tauri/src/vision_engine.rs` — SubjectPreset struct + build_face_analyzer method
- Frontend description text update (if desired)
