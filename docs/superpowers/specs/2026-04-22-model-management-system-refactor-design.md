# Design Spec: Model Management System Refactor

This document outlines the refactoring of the model management system in the Nebula project. The goal is to move from ad-hoc model handling in `vision_engine.rs` to a centralized, spec-driven architecture using the `models` module.

## 1. Problem Statement

Currently, `vision_engine.rs` contains hardcoded HuggingFace repository IDs, file paths, and download logic. This makes it difficult to add new models, leads to code duplication, and mixes high-level inference logic with low-level file management. The new `models` module provides a `ModelManager` and `ModelSpec` registry, but it hasn't been integrated into the rest of the application.

## 2. Proposed Architecture

We will adopt a decoupled architecture where:
1.  **Registry (`registry.rs`)** acts as the single source of truth for model identity, files, and metadata.
2.  **ModelManager (`manager.rs`)** handles all file system operations, HuggingFace downloads, and acts as the **authority for path resolution**.
3.  **VisionEngine (`vision_engine.rs`)** becomes a "pure" inference engine that executes models based on provided `ModelSpec`s and `FaceIdPreset`s.
4.  **AppState (`lib.rs`)** orchestrates the flow: downloading models via the `ModelManager` before passing them to the `VisionEngine` for execution.

## 3. Component Details

### 3.1 Registry & Spec Enhancements (`src-tauri/src/models/registry.rs`)

`ModelSpec` will be updated to explicitly define its primary files.

```rust
pub struct ModelSpec {
    pub id: &'static str,
    pub hf_repo: &'static str,
    pub model_type: ModelType,
    pub cache_dir: &'static str,
    pub model_file: ModelFile,
    pub tokenizer_file: Option<ModelFile>,
    pub display_name: &'static str,
    pub display_description: &'static str,
}

pub struct FaceIdPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub detector: &'static ModelSpec,
    pub embedder: &'static ModelSpec,
    pub detector_input_size: (u32, u32),
}
```

**Key methods:**
- `ModelSpec::all_files()`: Returns a list of all required files for the manager.

### 3.2 ModelManager (`src-tauri/src/models/manager.rs`)

Renamed from `downloader.rs`. The module structure will be updated in `src-tauri/src/models/mod.rs` to export `manager`. It now provides the absolute paths required for model loading.

**Key methods:**
- `ensure_ready(spec)`: Downloads files if missing.
- `onnx_path(spec)`: Returns the absolute path to the `.onnx` model file.
- `tokenizer_path(spec)`: Returns the absolute path to the `tokenizer.json` file.

### 3.3 VisionEngine Refactoring (`src-tauri/src/vision_engine.rs`)

`VisionEngine` will be simplified by removing all download-related code.

- **`get_face_analyzer(&self, manager: &ModelManager, preset: &FaceIdPreset)`**: Uses `FaceAnalyzer::builder` with local paths provided by the `ModelManager`.
- **`embed_image(&self, manager: &ModelManager, img, spec: &ModelSpec)`**: Uses `manager.onnx_path(spec)` to load the session.
- **`embed_text(&self, manager: &ModelManager, text, spec: &ModelSpec)`**: Uses `manager.onnx_path(spec)` and `manager.tokenizer_path(spec)`.

### 3.4 AppState Orchestration (`src-tauri/src/lib.rs`)

`AppState` will include both `VisionEngine` and `ModelManager`.

```rust
pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub indexer: Arc<indexer::Indexer>,
    pub vision_engine: Arc<vision_engine::VisionEngine>,
    pub model_manager: Arc<models::ModelManager>, // Added
    pub index: vector_index::IndexStore,
}
```

## 4. Implementation Plan

1.  **Registry Update:** Refactor `ModelSpec` and define `BUFFALO_S` specs and preset in `registry.rs`.
2.  **ModelManager Refactor:** 
    - Rename `src-tauri/src/models/downloader.rs` to `src-tauri/src/models/manager.rs`.
    - Update `src-tauri/src/models/mod.rs` to export `pub mod manager`.
    - Update `ModelManager` to use the new `all_files()` method and provide path resolution.
3.  **VisionEngine Update:** Rewrite `VisionEngine` to use `ModelManager` for paths.
4.  **AppState Integration:** Wire the `ModelManager` into `AppState` and update workers/commands.
5.  **Settings Cleanup:** Update `settings.rs` to pull available models from the registry.

## 5. Verification Plan

- **Automated Tests:** Verify `ModelManager::onnx_path` returns expected paths.
- **Manual Verification:** 
    - Trigger a model change in settings and verify the download progress is emitted.
    - Verify face detection and semantic search still work correctly after the refactor.
