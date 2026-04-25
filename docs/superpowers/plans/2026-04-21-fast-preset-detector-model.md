# FAST Preset: Use Smaller Detector Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce the face detection bottleneck in the FAST preset by switching to a significantly smaller detector model (16.9 MB vs 39.4 MB) that still provides the necessary keypoints for alignment.

**Architecture:** Update the `SubjectPreset` struct to include optional Hugging Face model overrides for the detector. Modify `build_face_analyzer` to apply these overrides when present.

**Tech Stack:** Rust, `face_id` crate.

---

### Task 1: Update `SubjectPreset` Struct and Constants

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`

- [ ] **Step 1: Add new fields to `SubjectPreset`**

Update the struct definition to include `detector_hf_id` and `detector_hf_file`.

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

- [ ] **Step 2: Update `STANDARD` constant**

Set empty strings for the new fields in the `STANDARD` preset.

```rust
    pub const STANDARD: SubjectPreset = SubjectPreset {
        id: "standard",
        name: "Standard",
        description: "Full accuracy (640\u{00d7}640 detection)",
        detector_input_size: (640, 640),
        skip_gender_age: false,
        detector_hf_id: "",
        detector_hf_file: "",
    };
```

- [ ] **Step 3: Update `FAST` constant**

Configure the `FAST` preset to use the smaller detector and update its description.

```rust
    pub const FAST: SubjectPreset = SubjectPreset {
        id: "fast",
        name: "Fast",
        description: "Uses smaller detector and skips gender/age for maximum speed",
        detector_input_size: (640, 640),
        skip_gender_age: true,
        detector_hf_id: "RuteNL/SCRFD-face-detection-ONNX",
        detector_hf_file: "10g_bnkps.onnx",
    };
```

- [ ] **Step 4: Commit changes**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat(vision): add detector model overrides to SubjectPreset"
```

---

### Task 2: Update `build_face_analyzer` to use overrides

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`

- [ ] **Step 1: Update `build_face_analyzer` implementation**

Modify the method to check `detector_hf_id` and apply the `detector_model` override if it's not empty.

```rust
    async fn build_face_analyzer(preset: &SubjectPreset) -> Result<face_id::analyzer::FaceAnalyzer> {
        let mut builder = face_id::analyzer::FaceAnalyzer::from_hf()
            .detector_input_size(preset.detector_input_size);
        
        if !preset.detector_hf_id.is_empty() {
            builder = builder.detector_model(face_id::model_manager::HfModel {
                id: preset.detector_hf_id.to_string(),
                file: preset.detector_hf_file.to_string(),
            });
        }

        builder.build()
            .await
            .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check` in `src-tauri` directory.
Expected: Success.

- [ ] **Step 3: Commit changes**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat(vision): use detector model override in face analyzer builder"
```

---

### Task 3: Verification

**Files:**
- None (manual verification)

- [ ] **Step 1: Manual test (optional but recommended)**

If possible, switch the app to "Fast" preset in settings and ensure face detection still works (though it will download a new model on first use).

- [ ] **Step 2: Final Check**

Ensure all `SubjectPreset` usage is consistent with the new struct definition.

```bash
grep -r "SubjectPreset" src-tauri/src
```
