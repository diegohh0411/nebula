# Model Management System Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the model management system to use a centralized `ModelManager` and `ModelSpec` registry, decoupling download logic from the `VisionEngine`.

**Architecture:** Use a spec-driven approach where `ModelSpec` defines model metadata and `ModelManager` acts as the authority for file downloading and path resolution. `VisionEngine` is refactored into a pure inference engine that accepts specs and uses `ModelManager` to resolve local paths.

**Tech Stack:** Rust (Tauri, ONNX Runtime, face-id, tokenizers).

---

### Task 1: Update Model Registry and Specs

**Files:**
- Modify: `src-tauri/src/models/registry.rs`

- [ ] **Step 1: Refactor `ModelSpec` and `FaceIdPreset`**

Update the structs to include explicit file definitions and the new `FaceIdPreset` structure.

```rust
pub enum ModelType {
  TextImageEmbedding,
  FaceEmbedding,
  FaceDetection,
}

pub struct ModelFile {
  pub filename: &'static str,
  pub remote_path: Option<&'static str>,
}

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

impl ModelSpec {
  pub fn all_files(&self) -> Vec<&ModelFile> {
    let mut f = vec![&self.model_file];
    if let Some(ref t) = self.tokenizer_file {
      f.push(t);
    }
    f
  }
}
```

- [ ] **Step 2: Define `BUFFALO_S` Specs and Preset**

Replace the existing hardcoded models with the new `ModelSpec` and `FaceIdPreset` instances.

```rust
pub const SIGLIP_BASE: ModelSpec = ModelSpec {
  id: "diegohh/siglip2-base-patch16-224",
  hf_repo: "diegohh/siglip2-base-patch16-224",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-224",
  model_file: ModelFile { filename: "model.onnx", remote_path: None },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Standard",
  display_description: "Balanced quality and speed (86M params)",
};

pub const SIGLIP_FAST: ModelSpec = ModelSpec {
  id: "onnx-community/siglip2-base-patch32-256-ONNX",
  hf_repo: "onnx-community/siglip2-base-patch32-256-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-256",
  model_file: ModelFile { filename: "model_fp16.onnx", remote_path: Some("onnx/model_fp16.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Fast",
  display_description: "Optimized for consumer CPUs with larger patches",
};

pub const BUFFALO_S_RECOGNITION: ModelSpec = ModelSpec {
  id: "buffalo_s_recognition",
  hf_repo: "immich-app/buffalo_s",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "buffalo_s",
  model_file: ModelFile { filename: "recognition.onnx", remote_path: Some("recognition/model.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo S Recognition",
  display_description: "Lightweight face recognition model",
};

pub const BUFFALO_S_DETECTION: ModelSpec = ModelSpec {
  id: "buffalo_s_detection",
  hf_repo: "immich-app/buffalo_s",
  model_type: ModelType::FaceDetection,
  cache_dir: "buffalo_s",
  model_file: ModelFile { filename: "detection.onnx", remote_path: Some("detection/model.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo S Detection",
  display_description: "Lightweight face detection model",
};

pub const BUFFALO_S_PRESET: FaceIdPreset = FaceIdPreset {
    id: "blitz",
    name: "Blitz",
    description: "Maximum inference speed, for bulk processing",
    detector: &BUFFALO_S_DETECTION,
    embedder: &BUFFALO_S_RECOGNITION,
    detector_input_size: (640, 640),
};

pub const ALL_MODELS: &[&ModelSpec] = &[&SIGLIP_BASE, &SIGLIP_FAST, &BUFFALO_S_RECOGNITION, &BUFFALO_S_DETECTION];
pub const ALL_PRESETS: &[&FaceIdPreset] = &[&BUFFALO_S_PRESET];
```

- [ ] **Step 3: Add `find_by_id` and `find_preset_by_id` helpers**

```rust
impl ModelSpec {
  pub fn find_by_id(id: &str) -> Option<&'static ModelSpec> {
    ALL_MODELS.iter().find(|m| m.id == id).copied()
  }
}

impl FaceIdPreset {
  pub fn find_by_id(id: &str) -> Option<&'static FaceIdPreset> {
    ALL_PRESETS.iter().find(|p| p.id == id).copied()
  }
}
```

- [ ] **Step 4: Verify Compilation**

Run: `cargo check`
Expected: PASS (with some dead code warnings)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/models/registry.rs
git commit -m "feat: refactor model registry and specs"
```

---

### Task 2: Refactor `ModelManager`

**Files:**
- Rename: `src-tauri/src/models/downloader.rs` -> `src-tauri/src/models/manager.rs`
- Modify: `src-tauri/src/models/mod.rs`
- Modify: `src-tauri/src/models/manager.rs`

- [ ] **Step 1: Rename the file and update module**

```bash
mv src-tauri/src/models/downloader.rs src-tauri/src/models/manager.rs
```

Update `src-tauri/src/models/mod.rs`:
```rust
pub mod registry;
pub mod manager;

pub use manager::{ModelManager, ModelDownloadPayload};
```

- [ ] **Step 2: Update `ModelManager` implementation**

Rename the struct and update `ensure_ready` to use `spec.all_files()`. Add path resolution methods.

```rust
pub struct ModelManager {
  data_dir: PathBuf,
  readiness: std::sync::Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
}

impl ModelManager {
  pub fn new(data_dir: PathBuf) -> Self {
    Self {
      data_dir,
      readiness: std::sync::Mutex::new(HashMap::new()),
    }
  }

  pub fn model_dir(&self, spec: &ModelSpec) -> PathBuf {
    self.data_dir.join("models").join(spec.cache_dir)
  }

  pub fn onnx_path(&self, spec: &ModelSpec) -> PathBuf {
    self.model_dir(spec).join(spec.model_file.filename)
  }

  pub fn tokenizer_path(&self, spec: &ModelSpec) -> Option<PathBuf> {
    spec.tokenizer_file.as_ref().map(|f| self.model_dir(spec).join(f.filename))
  }

  pub async fn ensure_ready(&self, app: &AppHandle, spec: &ModelSpec) -> Result<()> {
    let dir = self.model_dir(spec);

    // ... (readiness channel logic same as before) ...

    // Fast path: use all_files()
    if spec.all_files().iter().all(|f| dir.join(f.filename).exists()) {
      self.signal_ready(spec.id);
      return Ok(());
    }

    tokio::fs::create_dir_all(&dir).await?;
    let client = reqwest::Client::new();

    for file in spec.all_files() {
      // ... (download logic same as before, using file.filename and file.remote_path) ...
    }

    self.signal_ready(spec.id);
    Ok(())
  }
}
```

- [ ] **Step 3: Verify Compilation**

Run: `cargo check`
Expected: PASS (except for `vision_engine.rs` which will now be broken)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/models/
git commit -m "feat: rename downloader to ModelManager and add path resolution"
```

---

### Task 3: Refactor `VisionEngine`

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`

- [ ] **Step 1: Simplify `VisionEngine` Struct and Imports**

Remove download-related fields and methods. Use `FaceAnalyzer::builder`.

```rust
use crate::models::manager::ModelManager;
use crate::models::registry::{ModelSpec, FaceIdPreset};
use face_id::analyzer::FaceAnalyzer;

pub struct VisionEngine {
    pub data_dir: PathBuf,
    session: std::sync::Mutex<Option<(String, Session)>>,
    tokenizer: std::sync::Mutex<Option<(String, tokenizers::Tokenizer)>>,
    face_analyzer: std::sync::Mutex<Option<(String, Arc<FaceAnalyzer>)>>,
}
```

- [ ] **Step 2: Update `get_face_analyzer`**

```rust
pub async fn get_face_analyzer(
    &self, 
    manager: &ModelManager, 
    preset: &FaceIdPreset
) -> Result<Arc<FaceAnalyzer>> {
    {
        let guard = self.face_analyzer.lock().unwrap();
        if let Some((current_id, analyzer)) = guard.as_ref() {
            if current_id == preset.id {
                return Ok(Arc::clone(analyzer));
            }
        }
    }

    let det_path = manager.onnx_path(preset.detector);
    let rec_path = manager.onnx_path(preset.embedder);

    let analyzer = FaceAnalyzer::builder(det_path, rec_path, None)
        .detector_input_size(preset.detector_input_size)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))?;

    let analyzer = Arc::new(analyzer);
    {
        let mut guard = self.face_analyzer.lock().unwrap();
        *guard = Some((preset.id.to_string(), Arc::clone(&analyzer)));
    }
    Ok(analyzer)
}
```

- [ ] **Step 3: Update `embed_image` and `embed_text`**

Update these to accept `&ModelSpec` and use `manager.onnx_path(spec)`.

```rust
pub fn embed_image(&self, manager: &ModelManager, img: &image::DynamicImage, spec: &ModelSpec) -> Result<Vec<f32>> {
    // ... preprocessing ...
    let model_path = manager.onnx_path(spec);
    // ... load session if needed using model_path and spec.id ...
}

pub fn embed_text(&self, manager: &ModelManager, text: &str, spec: &ModelSpec) -> Result<Vec<f32>> {
    let tok_path = manager.tokenizer_path(spec).ok_or_else(|| anyhow!("Model has no tokenizer"))?;
    // ... load tokenizer if needed using tok_path and spec.id ...
    
    let model_path = manager.onnx_path(spec);
    // ... load session if needed using model_path and spec.id ...
}
```

- [ ] **Step 4: Verify Compilation**

Run: `cargo check`
Expected: PASS (except for `lib.rs` and `settings.rs` callers)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat: refactor VisionEngine to use ModelManager and specs"
```

---

### Task 4: Integrate into `AppState` and Workers

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/embedder.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/settings.rs`

- [ ] **Step 1: Update `AppState` definition**

```rust
pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub indexer: Arc<indexer::Indexer>,
    pub vision_engine: Arc<vision_engine::VisionEngine>,
    pub model_manager: Arc<crate::models::ModelManager>, // Added
    pub index: vector_index::IndexStore,
}
```

- [ ] **Step 2: Update `run()` initialization**

Initialize `ModelManager` and use it in the startup background task.

```rust
let model_manager = Arc::new(crate::models::ModelManager::new(data_dir.clone()));

// In setup:
app.manage(AppState {
    // ...
    model_manager: model_manager.clone(),
});

// Startup task:
let model_id = ...;
let spec = ModelSpec::find_by_id(&model_id).unwrap_or(&crate::models::registry::SIGLIP_BASE);
model_manager.ensure_ready(&app_handle, spec).await?;
```

- [ ] **Step 3: Update Workers and Commands**

Update all calls to `vision_engine.embed_image`, `embed_text`, and `get_face_analyzer` to pass `&state.model_manager`.

- [ ] **Step 4: Update `settings.rs`**

Update `get_available_models` and `get_available_subject_models` to pull from the registry.

```rust
#[command]
pub fn get_available_models() -> Vec<ModelInfo> {
    crate::models::registry::ALL_MODELS
        .iter()
        .filter(|m| matches!(m.model_type, crate::models::registry::ModelType::TextImageEmbedding))
        .map(|m| ModelInfo {
            id: m.id.to_string(),
            name: m.display_name.to_string(),
            description: m.display_description.to_string(),
        })
        .collect()
}
```

- [ ] **Step 5: Final Verification**

Run: `cargo build`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/
git commit -m "feat: integrate ModelManager into AppState and workers"
```
