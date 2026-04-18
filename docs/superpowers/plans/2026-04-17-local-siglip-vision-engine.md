# Local SigLIP 2 Vision Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the cloud-based Gemini embedding API with a fully local SigLIP 2 vision engine using ONNX Runtime (ORT), enabling 100% offline visual and natural language search.

**Architecture:** Introduce a unified `VisionEngine` service that manages local ONNX sessions for both Face-ID (ArcFace) and SigLIP 2. Refactor existing background workers to use this engine and trigger a one-time database migration to reset cloud-based embeddings.

**Tech Stack:** Rust, Tauri, ORT (ONNX Runtime), SigLIP 2 (via ONNX), `hf-hub` for model management, `tokenizers` for text encoding.

---

### Task 1: Project Setup & Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add required dependencies**

```toml
[dependencies]
# ... existing dependencies
ort = "2.0.0-rc.12"
hf-hub = "0.4"
tokenizers = "0.21"
ndarray = "0.15"
# ...
```

- [ ] **Step 2: Run cargo check to verify dependencies**

Run: `cargo check` in `src-tauri`
Expected: Success

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml
git commit -m "chore: add dependencies for local SigLIP 2"
```

---

### Task 2: Database Migration - Resetting Embeddings

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add a function to reset all embeddings**

```rust
pub async fn reset_all_embeddings(pool: &SqlitePool) -> Result<()> {
    // 1. Clear image embeddings (Gemini space)
    sqlx::query("UPDATE images SET embedding = NULL, embed_status = 'pending'")
        .execute(pool)
        .await?;

    // 2. Clear embedding queue to prevent retries of old tasks
    sqlx::query("DELETE FROM embedding_queue")
        .execute(pool)
        .await?;

    // 3. Re-enqueue all images for the new local engine
    sqlx::query("INSERT INTO embedding_queue (image_id, attempts, scheduled_at) 
                 SELECT id, 0, ? FROM images WHERE deleted_at IS NULL")
        .bind(chrono::Utc::now().timestamp())
        .execute(pool)
        .await?;

    Ok(())
}
```

- [ ] **Step 2: Run the migration on startup if a specific flag/version is missing**

(For this task, we will just expose the function and call it in `lib.rs` once for the migration).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add embedding reset migration"
```

---

### Task 3: VisionEngine - Core Service Implementation

**Files:**
- Create: `src-tauri/src/vision_engine.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Define the `VisionEngine` structure and initialization**

```rust
use anyhow::Result;
use ort::{Session, Environment};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::path::PathBuf;

pub struct VisionEngine {
    data_dir: PathBuf,
    image_session: Arc<Mutex<Option<Session>>>,
    text_session: Arc<Mutex<Option<Session>>>,
    tokenizer: Arc<Mutex<Option<tokenizers::Tokenizer>>>,
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            image_session: Arc::new(Mutex::new(None)),
            text_session: Arc::new(Mutex::new(None)),
            tokenizer: Arc::new(Mutex::new(None)),
        }
    }
}
```

- [ ] **Step 2: Register `VisionEngine` in `AppState`**

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/vision_engine.rs src-tauri/src/lib.rs
git commit -m "feat: skeleton for VisionEngine service"
```

---

### Task 4: VisionEngine - Model Loading Logic

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`

- [ ] **Step 1: Implement model downloading and loading**

```rust
impl VisionEngine {
    async fn load_session(&self, filename: &str, session_mutex: &Arc<Mutex<Option<Session>>>) -> Result<()> {
        let mut lock = session_mutex.lock().await;
        if lock.is_none() {
            let api = hf_hub::api::sync::Api::new()?;
            let repo = api.model("google/siglip-so400m-patch14-384".to_string());
            let model_path = repo.get(filename)?;
            
            let session = Session::builder()?
                .with_optimization_level(ort::GraphOptimizationLevel::Level3)?
                .with_intra_threads(4)?
                .with_model_from_file(model_path)?;
            *lock = Some(session);
        }
        Ok(())
    }

    pub async fn get_image_session(&self) -> Result<Arc<Mutex<Option<Session>>>> {
        self.load_session("model.onnx", &self.image_session).await?;
        Ok(self.image_session.clone())
    }
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Success

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat: implement model loading in VisionEngine"
```

---

### Task 5: VisionEngine - Image Inference (SigLIP)

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`

- [ ] **Step 1: Implement image embedding logic**

```rust
use ndarray::Array4;

impl VisionEngine {
    pub async fn embed_image(&self, img: &image::DynamicImage) -> Result<Vec<f32>> {
        let session_arc = self.get_image_session().await?;
        let lock = session_arc.lock().await;
        let session = lock.as_ref().unwrap();

        // Preprocess: Resize to 384x384 for so400m model
        let resized = img.resize_exact(384, 384, image::imageops::FilterType::Lanczos3);
        let rgb = resized.to_rgb8();
        
        let mut input = Array4::<f32>::zeros((1, 3, 384, 384));
        for (x, y, pixel) in rgb.enumerate_pixels() {
            for c in 0..3 {
                // Normalization: (x - mean) / std
                let val = pixel[c] as f32 / 255.0;
                input[[0, c, y as usize, x as usize]] = (val - 0.5) / 0.5;
            }
        }

        let outputs = session.run(ort::inputs![input]?)?;
        let output = outputs[0].try_extract_tensor::<f32>()?;
        Ok(output.view().to_slice().unwrap().to_vec())
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat: implement image embedding in VisionEngine"
```

---

### Task 6: VisionEngine - Text Inference (SigLIP)

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`

- [ ] **Step 1: Implement text embedding logic**

```rust
impl VisionEngine {
    pub async fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let mut tok_lock = self.tokenizer.lock().await;
        if tok_lock.is_none() {
            let api = hf_hub::api::sync::Api::new()?;
            let repo = api.model("google/siglip-so400m-patch14-384".to_string());
            let tok_path = repo.get("tokenizer.json")?;
            *tok_lock = Some(tokenizers::Tokenizer::from_file(tok_path).map_err(|e| anyhow::anyhow!(e))?);
        }

        self.load_session("text_model.onnx", &self.text_session).await?;
        let session_lock = self.text_session.lock().await;
        let session = session_lock.as_ref().unwrap();
        
        let tokenizer = tok_lock.as_ref().unwrap();
        let encoding = tokenizer.encode(text, true).map_err(|e| anyhow::anyhow!(e))?;
        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let input_ids_tensor = ndarray::Array2::from_shape_vec((1, input_ids.len()), input_ids)?;

        let outputs = session.run(ort::inputs![input_ids_tensor]?)?;
        let output = outputs[0].try_extract_tensor::<f32>()?;
        Ok(output.view().to_slice().unwrap().to_vec())
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat: implement text embedding in VisionEngine"
```

---

### Task 7: Refactor Background Worker

**Files:**
- Modify: `src-tauri/src/embedder.rs`

- [ ] **Step 1: Update `process_one` to use `VisionEngine` instead of Gemini API**

- [ ] **Step 2: Remove Gemini-specific code and `api_key` requirements for global search**

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/embedder.rs
git commit -m "refactor: use local VisionEngine in background worker"
```

---

### Task 8: Refactor Search Command

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Update `search` command to use `VisionEngine` for text queries**

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "refactor: update search command for local embeddings"
```

---

### Task 9: Final Migration and Cleanup

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [x] **Step 1: Trigger `db::reset_all_embeddings` if detected needed (one-time)** (Skipped by user request)

- [x] **Step 2: Remove redundant `face_detector.rs` if unified into `VisionEngine`** (Not unified, kept separate)

- [x] **Step 3: Final verification run**

- [x] **Step 4: Commit**

```bash
git commit -m "feat: final migration to local SigLIP 2"
```
