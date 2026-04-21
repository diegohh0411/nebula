# Subject Analysis "Fast" Preset Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a configurable "Fast" preset for subject analysis that reduces per-image processing time by using a smaller detector input size and skipping gender/age inference.

**Architecture:** Replace the `OnceCell<FaceAnalyzer>` with a `Mutex<Option<(String, Arc<FaceAnalyzer>)>>` store that detects preset changes and rebuilds. For the "fast" preset, bypass `FaceAnalyzer::analyze()` and call detector + embedder directly, skipping gender/age. Add a `subject_model` setting and frontend picker mirroring the existing SigLIP model picker.

**Tech Stack:** Rust (Tauri backend), Angular (frontend), `face_id` 0.4.1 crate

---

### Task 1: Database — Add `subject_model` default setting + reset function

**Files:**
- Modify: `src-tauri/src/db.rs:154` (default setting insertion)
- Modify: `src-tauri/src/db.rs:1186-1219` (add new reset function after `reset_all_embeddings`)

- [ ] **Step 1: Add `subject_model` default setting in `init_db`**

In `src-tauri/src/db.rs`, after the existing `embedding_model` INSERT at line 154-156, add a second INSERT:

```rust
    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('embedding_model', 'diegohh/siglip2-base-patch16-224')")
        .execute(&pool)
        .await?;

    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('subject_model', 'standard')")
        .execute(&pool)
        .await?;
```

- [ ] **Step 2: Add `reset_all_subject_data` function**

Add after `reset_all_embeddings` (after line 1219):

```rust
pub async fn reset_all_subject_data(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM face_corrections")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM faces")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM subjects")
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE images SET subject_analysis_done = 0 WHERE deleted_at IS NULL")
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM embedding_queue WHERE pipeline = 'subject'")
        .execute(&mut *tx)
        .await?;

    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at)
         SELECT id, 'subject', 0, ? FROM images WHERE deleted_at IS NULL",
    )
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 3: Verify compilation**

Run: `cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -5`
Expected: `Finished` with no errors

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(db): add subject_model default setting and reset_all_subject_data"
```

---

### Task 2: Vision Engine — Replace `OnceCell` with Arc-based swap-able store

**Files:**
- Modify: `src-tauri/src/vision_engine.rs:1-10` (add imports)
- Modify: `src-tauri/src/vision_engine.rs:22-56` (struct + new)
- Modify: `src-tauri/src/vision_engine.rs:173-182` (replace `get_face_analyzer`)

- [ ] **Step 1: Update imports**

At the top of `src-tauri/src/vision_engine.rs`, add `Arc` and `face_id` imports. Change line 1 to:

```rust
use anyhow::Result;
use face_id::face_align::norm_crop;
use futures::StreamExt;
use ndarray::{Array2, Array4};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

use crate::models::ModelDownloadPayload;
```

- [ ] **Step 2: Replace `face_analyzer` field in `VisionEngine` struct**

In the `VisionEngine` struct (line 22-29), replace `face_analyzer: tokio::sync::OnceCell<face_id::analyzer::FaceAnalyzer>` with `face_analyzer: std::sync::Mutex<Option<(String, Arc<face_id::analyzer::FaceAnalyzer>)>>`:

```rust
pub struct VisionEngine {
    pub data_dir: PathBuf,
    session: std::sync::Mutex<Option<(String, Session)>>,
    tokenizer: std::sync::Mutex<Option<(String, tokenizers::Tokenizer)>>,
    face_analyzer: std::sync::Mutex<Option<(String, Arc<face_id::analyzer::FaceAnalyzer>)>>,
    model_ready_tx: tokio::sync::watch::Sender<bool>,
    model_ready_rx: tokio::sync::watch::Receiver<bool>,
}
```

- [ ] **Step 3: Update `VisionEngine::new`**

In `new()` (line 46-56), change `face_analyzer: tokio::sync::OnceCell::new()` to `face_analyzer: std::sync::Mutex::new(None)`:

```rust
    pub fn new(data_dir: PathBuf) -> Self {
        let (tx, rx) = tokio::sync::watch::channel(false);
        Self {
            data_dir,
            session: std::sync::Mutex::new(None),
            tokenizer: std::sync::Mutex::new(None),
            face_analyzer: std::sync::Mutex::new(None),
            model_ready_tx: tx,
            model_ready_rx: rx,
        }
    }
```

- [ ] **Step 4: Replace `get_face_analyzer` method**

Replace the entire `get_face_analyzer` method (lines 173-182) with three new methods:

```rust
    async fn build_face_analyzer(&self, preset: &str) -> Result<face_id::analyzer::FaceAnalyzer> {
        let mut builder = face_id::analyzer::FaceAnalyzer::from_hf();
        if preset == "fast" {
            builder = builder.detector_input_size((320, 320));
        }
        builder
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))
    }

    pub async fn get_face_analyzer(&self, preset: &str) -> Result<Arc<face_id::analyzer::FaceAnalyzer>> {
        {
            let guard = self.face_analyzer.lock()
                .map_err(|e| anyhow::anyhow!("face analyzer mutex poisoned: {e}"))?;
            if let Some((current, analyzer)) = guard.as_ref() {
                if current == preset {
                    return Ok(Arc::clone(analyzer));
                }
            }
        }

        let analyzer = Arc::new(self.build_face_analyzer(preset).await?);
        {
            let mut guard = self.face_analyzer.lock()
                .map_err(|e| anyhow::anyhow!("face analyzer mutex poisoned: {e}"))?;
            *guard = Some((preset.to_string(), Arc::clone(&analyzer)));
        }
        Ok(analyzer)
    }

    pub fn analyze_faces_fast(
        analyzer: &face_id::analyzer::FaceAnalyzer,
        img: &image::DynamicImage,
    ) -> Result<Vec<(face_id::detector::DetectedFace, Vec<f32>)>> {
        let rgb_img = img.to_rgb8();

        let detections = {
            let mut detector = analyzer.detector.lock()
                .map_err(|e| anyhow::anyhow!("detector mutex poisoned: {e}"))?;
            detector.detect(img)?
        };

        if detections.is_empty() {
            return Ok(vec![]);
        }

        let embed_crops: Vec<_> = detections
            .iter()
            .map(|res| {
                let landmarks = res.landmarks.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("face missing landmarks for embedding"))?;
                let lms_array: [(f32, f32); 5] = landmarks
                    .iter()
                    .map(|&(x, y)| (x * rgb_img.width() as f32, y * rgb_img.height() as f32))
                    .collect::<Vec<_>>()
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("landmarks were not 5-point keypoints"))?;
                Ok(norm_crop(&rgb_img, &lms_array, 112))
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;

        let embeddings = {
            let mut embedder = analyzer.embedder.lock()
                .map_err(|e| anyhow::anyhow!("embedder mutex poisoned: {e}"))?;
            embedder.compute_embeddings_batch(&embed_crops)
                .map_err(|e| anyhow::anyhow!("batch embedding failed: {e}"))?
        };

        Ok(detections.into_iter().zip(embeddings).collect())
    }
```

- [ ] **Step 5: Verify compilation**

Run: `cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -5`
Expected: May have errors in `embedder.rs` because `get_face_analyzer()` signature changed. That's OK — Task 3 fixes it.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "feat(vision): replace OnceCell with Arc-based swap-able face analyzer store"
```

---

### Task 3: Embedder — Add fast path + update subject worker

**Files:**
- Modify: `src-tauri/src/embedder.rs:121-202` (`process_subject_one`)
- Modify: `src-tauri/src/embedder.rs:270-331` (`run_subject_worker`)

- [ ] **Step 1: Update `process_subject_one` to accept preset and use fast path**

Replace the entire `process_subject_one` function (lines 121-202):

```rust
async fn process_subject_one(
    pool: &SqlitePool,
    app: &AppHandle,
    vision_engine: &crate::vision_engine::VisionEngine,
    queue_id: i64,
    image_id: i64,
    attempts: i32,
    subject_preset: &str,
) {
    let image = match db::get_image_by_id(pool, image_id).await {
        Ok(Some(img)) => img,
        _ => return,
    };

    let analyzer = match vision_engine.get_face_analyzer(subject_preset).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Face analyzer unavailable for image {}: {}", image_id, e);
            if db::mark_failed(pool, queue_id, attempts, &e.to_string()).await.is_ok() {
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
            emit_progress(pool, app).await;
            return;
        }
    };

    let img_res = tokio::task::spawn_blocking({
        let path = image.path.clone();
        move || image::open(path)
    }).await;

    let open_result = match img_res {
        Ok(Ok(dynamic_img)) => Ok(dynamic_img),
        Ok(Err(e)) => Err(anyhow::anyhow!("failed to open image: {}", e)),
        Err(e) => Err(anyhow::anyhow!("spawn_blocking panicked: {}", e)),
    };

    match open_result {
        Ok(dynamic_img) => {
            let faces_result = if subject_preset == "fast" {
                crate::vision_engine::VisionEngine::analyze_faces_fast(&analyzer, &dynamic_img)
                    .map(|pairs| pairs.into_iter().map(|(det, emb)| (det.bbox, emb)).collect::<Vec<_>>())
            } else {
                analyzer.analyze(&dynamic_img)
                    .map(|faces| faces.into_iter().map(|f| (f.detection.bbox, f.embedding)).collect::<Vec<_>>())
                    .map_err(|e| anyhow::anyhow!("{}", e))
            };

            match faces_result {
                Ok(faces) => {
                    for (bbox, face_emb) in faces {
                        let face_blob = f32_slice_to_bytes(&face_emb);
                        let _ = db::insert_face(
                            pool,
                            image_id,
                            None,
                            (
                                bbox.x1 as f64,
                                bbox.y1 as f64,
                                (bbox.x2 - bbox.x1) as f64,
                                (bbox.y2 - bbox.y1) as f64,
                            ),
                            Some(&face_blob),
                        ).await;
                    }
                    if db::mark_subject_analysis_done(pool, image_id).await.is_ok() {
                        let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    eprintln!("Face analysis failed for image {}: {}", image_id, err_str);
                    if db::mark_failed(pool, queue_id, attempts, &err_str).await.is_ok() {
                        let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
                    }
                }
            }
        }
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Subject analysis failed for image {}: {}", image_id, err_str);
            if db::mark_failed(pool, queue_id, attempts, &err_str).await.is_ok() {
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
        }
    }

    emit_progress(pool, app).await;
}
```

- [ ] **Step 2: Update `run_subject_worker` to read preset and pass it through**

Replace the entire `run_subject_worker` function (lines 271-332):

```rust
pub async fn run_subject_worker(
    pool: SqlitePool,
    app: AppHandle,
    vision_engine: Arc<crate::vision_engine::VisionEngine>,
) {
    vision_engine.wait_until_ready().await;

    let preset = db::get_setting(&pool, "subject_model")
        .await
        .unwrap_or(None)
        .unwrap_or_else(|| "standard".to_string());
    if let Err(e) = vision_engine.get_face_analyzer(&preset).await {
        eprintln!("[subject-worker] Failed to initialize face analyzer: {}", e);
    }

    let semaphore = Arc::new(Semaphore::new(CONCURRENT_WORKERS));

    loop {
        let batch = match db::get_queue_batch(&pool, "subject", (CONCURRENT_WORKERS * 2) as i64).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[subject-worker] Failed to fetch batch: {}", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        if batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let subject_preset = db::get_setting(&pool, "subject_model")
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| "standard".to_string());

        let had_items = !batch.is_empty();
        let mut handles = vec![];
        for (queue_id, image_id, attempts) in batch {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let pool_c = pool.clone();
            let app_c = app.clone();
            let ve_c = Arc::clone(&vision_engine);
            let sp_c = subject_preset.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                process_subject_one(&pool_c, &app_c, ve_c.as_ref(), queue_id, image_id, attempts, &sp_c).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        if had_items {
            eprintln!("[subject-worker] Batch complete, running auto-recluster...");
            match crate::clustering::cluster_unassigned_faces(&pool).await {
                Ok(result) => {
                    eprintln!(
                        "[subject-worker] Recluster done: {} clusters, {} noise, {} merged, {} deleted",
                        result.clusters, result.noise, result.merged, result.deleted
                    );
                    let _ = app.emit("subjects_updated", ());
                }
                Err(e) => {
                    eprintln!("[subject-worker] Auto-recluster failed: {}", e);
                }
            }
        }
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -5`
Expected: `Finished` with no errors

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embedder.rs
git commit -m "feat(embedder): add fast path for subject analysis with gender/age skip"
```

---

### Task 4: Settings backend — New commands + handle setting change

**Files:**
- Modify: `src-tauri/src/settings.rs:13-27` (add `get_available_subject_models`)
- Modify: `src-tauri/src/settings.rs:42-73` (handle `subject_model` in `update_setting`)

- [ ] **Step 1: Add `get_available_subject_models` command**

After `get_available_models` (after line 27), add:

```rust
#[command]
pub fn get_available_subject_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "standard".into(),
            name: "Standard".into(),
            description: "Full accuracy (640\u{00d7}640 detection)".into(),
        },
        ModelInfo {
            id: "fast".into(),
            name: "Fast".into(),
            description: "Optimized for consumer CPUs (320\u{00d7}320 detection)".into(),
        },
    ]
}
```

- [ ] **Step 2: Handle `subject_model` in `update_setting`**

In `update_setting`, after the `if key == "embedding_model"` block (after line 63), add:

```rust
    if key == "subject_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            crate::db::reset_all_subject_data(pool).await.map_err(|e| e.to_string())?;
        }
    }
```

The full `update_setting` function should now look like:

```rust
#[command]
pub async fn update_setting(app: tauri::AppHandle, state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let pool = &state.pool;

    if key == "embedding_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            state.vision_engine.ensure_model_ready(&app, &value).await.map_err(|e| e.to_string())?;
            crate::db::reset_all_embeddings(pool).await.map_err(|e| e.to_string())?;
            if let Ok(mut idx) = state.index.write() {
                *idx = Box::new(crate::vector_index::FlatIndex::new(768));
            }
            let idx_path = state.data_dir.join("nebula.idx");
            let _ = std::fs::remove_file(idx_path);
        }
    }

    if key == "subject_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            crate::db::reset_all_subject_data(pool).await.map_err(|e| e.to_string())?;
        }
    }

    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(&key)
        .bind(&value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}
```

- [ ] **Step 3: Verify compilation**

Run: `cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -5`
Expected: `Finished` with no errors

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(settings): add subject_model setting with reset on change"
```

---

### Task 5: Wire up new command in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs:111-136` (invoke handler)

- [ ] **Step 1: Register `get_available_subject_models`**

In `lib.rs`, add `settings::get_available_subject_models` to the `invoke_handler` macro. Add it after `settings::get_available_models` on line 133:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::add_folder,
            commands::remove_folder,
            commands::list_folders,
            commands::list_images,
            commands::search,
            commands::get_processing_status,
            commands::list_subjects,
            commands::name_subject,
            commands::list_faces,
            commands::list_faces_for_image,
            commands::get_face_crop,
            commands::set_subject_thumbnail,
            commands::get_subject_photos,
            commands::get_subject_detail,
            commands::recluster_faces,
            commands::get_merge_suggestions,
            commands::merge_subjects,
            commands::dismiss_merge_suggestion,
            commands::assign_face_to_subject,
            commands::create_subject_for_face,
            commands::unassign_face,
            settings::get_available_models,
            settings::get_available_subject_models,
            settings::get_setting,
            settings::update_setting,
        ])
```

- [ ] **Step 2: Verify compilation**

Run: `cd /home/pi/nebula/src-tauri && cargo check 2>&1 | tail -5`
Expected: `Finished` with no errors

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: register get_available_subject_models command"
```

---

### Task 6: Frontend — Add subject model picker to settings

**Files:**
- Modify: `src/app/components/settings/settings.component.ts`
- Modify: `src/app/components/settings/settings.component.html`

- [ ] **Step 1: Add subject model signals and loading to TS**

Add after the existing model signals (after line 46), and add new loading/selection methods. The full updated component:

```typescript
import { Component, OnInit, signal, inject, OnDestroy } from '@angular/core';
import { CommonModule } from '@angular/common';
import { invoke } from '@tauri-apps/api/core';
import { LucideAngularModule } from 'lucide-angular';
import { Subscription } from 'rxjs';
import {
  HlmCard,
  HlmCardHeader,
  HlmCardTitle,
  HlmCardDescription,
  HlmCardContent,
  HlmCardFooter,
} from '../../libs/ui/card/src';
import { HlmButton } from '../../libs/ui/button/src';
import { TauriEventsService } from '../../services/tauri-events.service';
import { ModelDownloadEvent } from '../../models/models';

interface ModelInfo {
  id: string;
  name: string;
  description: string;
}

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [
    CommonModule,
    LucideAngularModule,
    HlmCard,
    HlmCardHeader,
    HlmCardTitle,
    HlmCardDescription,
    HlmCardContent,
    HlmCardFooter,
    HlmButton,
  ],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.css'
})
export class SettingsComponent implements OnInit, OnDestroy {
  private events = inject(TauriEventsService);
  private sub = new Subscription();

  models = signal<ModelInfo[]>([]);
  currentModel = signal<string | null>(null);

  subjectModels = signal<ModelInfo[]>([]);
  currentSubjectModel = signal<string | null>(null);

  isConfirming = signal(false);
  pendingModelId = signal<string | null>(null);
  pendingSection = signal<'vision' | 'subject'>('vision');
  confirmInputValue = signal('');
  isProcessing = signal(false);

  processingPhase = signal<'downloading' | 'reindexing'>('downloading');
  downloadProgress = signal<number | null>(null);
  currentDownloadFile = signal<string | null>(null);

  async ngOnInit() {
    await this.loadModels();
    await this.loadSettings();

    this.sub.add(
      this.events.modelDownloadProgress$.subscribe((ev: ModelDownloadEvent) => {
        if (ev.done) {
          this.processingPhase.set('reindexing');
          this.downloadProgress.set(100);
          return;
        }
        this.processingPhase.set('downloading');
        this.currentDownloadFile.set(ev.file);
        if (ev.bytes_total) {
          this.downloadProgress.set((ev.bytes_done / ev.bytes_total) * 100);
        } else {
          this.downloadProgress.set(null);
        }
      })
    );
  }

  ngOnDestroy() {
    this.sub.unsubscribe();
  }

  async loadModels() {
    try {
      const availableModels = await invoke<ModelInfo[]>('get_available_models');
      this.models.set(availableModels);
    } catch (e) {
      console.error('Failed to load models:', e);
    }
    try {
      const subjectModels = await invoke<ModelInfo[]>('get_available_subject_models');
      this.subjectModels.set(subjectModels);
    } catch (e) {
      console.error('Failed to load subject models:', e);
    }
  }

  async loadSettings() {
    try {
      const model = await invoke<string | null>('get_setting', { key: 'embedding_model' });
      this.currentModel.set(model || 'diegohh/siglip2-base-patch16-224');
    } catch (e) {
      console.error('Failed to load settings:', e);
    }
    try {
      const subjectModel = await invoke<string | null>('get_setting', { key: 'subject_model' });
      this.currentSubjectModel.set(subjectModel || 'standard');
    } catch (e) {
      console.error('Failed to load subject model setting:', e);
    }
  }

  selectVisionModel(modelId: string) {
    if (modelId === this.currentModel() || this.isProcessing()) return;
    this.pendingModelId.set(modelId);
    this.pendingSection.set('vision');
    this.confirmInputValue.set('');
    this.isConfirming.set(true);
  }

  selectSubjectModel(modelId: string) {
    if (modelId === this.currentSubjectModel() || this.isProcessing()) return;
    this.pendingModelId.set(modelId);
    this.pendingSection.set('subject');
    this.confirmInputValue.set('');
    this.isConfirming.set(true);
  }

  cancelSelection() {
    if (this.isProcessing()) return;
    this.isConfirming.set(false);
    this.pendingModelId.set(null);
  }

  async confirmSelection() {
    const modelId = this.pendingModelId();
    const section = this.pendingSection();
    if (modelId && this.confirmInputValue() === 'REINDEX' && !this.isProcessing()) {
      this.isProcessing.set(true);
      this.processingPhase.set('reindexing');
      this.downloadProgress.set(0);
      try {
        const key = section === 'vision' ? 'embedding_model' : 'subject_model';
        await invoke('update_setting', { key, value: modelId });
        if (section === 'vision') {
          this.currentModel.set(modelId);
        } else {
          this.currentSubjectModel.set(modelId);
        }
        this.isConfirming.set(false);
        this.pendingModelId.set(null);
      } catch (e) {
        console.error('Failed to update model:', e);
      } finally {
        this.isProcessing.set(false);
        this.downloadProgress.set(null);
        this.currentDownloadFile.set(null);
      }
    }
  }
}
```

- [ ] **Step 2: Update HTML template to add subject model section**

Replace the entire `settings.component.html`:

```html
<div class="settings-container">
  <header class="settings-header">
    <lucide-icon name="settings" class="header-icon"></lucide-icon>
    <h1>Settings</h1>
  </header>

  <div class="settings-content">
    <section class="settings-section">
      <div class="section-title">
        <lucide-icon name="search" class="section-icon"></lucide-icon>
        <h2>Vision Model</h2>
      </div>
      <p class="section-description">Select the embedding model used for image search and semantic analysis.</p>

      <div class="model-list">
        @for (model of models(); track model.id) {
          <div
            hlmCard
            class="cursor-pointer transition-all hover:border-ring"
            [class.active-card]="currentModel() === model.id"
            (click)="selectVisionModel(model.id)"
          >
            <div hlmCardHeader>
              <div class="flex items-center justify-between">
                <h3 hlmCardTitle>{{ model.name }}</h3>
                @if (currentModel() === model.id) {
                  <span class="text-xs font-medium px-2 py-0.5 rounded-full border border-border bg-muted text-muted-foreground">Active</span>
                }
              </div>
              <p hlmCardDescription>{{ model.description }}</p>
            </div>
            <div hlmCardContent>
              <p class="model-id">{{ model.id }}</p>
            </div>
          </div>
        }
      </div>
    </section>

    <section class="settings-section">
      <div class="section-title">
        <lucide-icon name="scan-face" class="section-icon"></lucide-icon>
        <h2>Face Analysis</h2>
      </div>
      <p class="section-description">Select the model used for face detection and subject labeling.</p>

      <div class="model-list">
        @for (model of subjectModels(); track model.id) {
          <div
            hlmCard
            class="cursor-pointer transition-all hover:border-ring"
            [class.active-card]="currentSubjectModel() === model.id"
            (click)="selectSubjectModel(model.id)"
          >
            <div hlmCardHeader>
              <div class="flex items-center justify-between">
                <h3 hlmCardTitle>{{ model.name }}</h3>
                @if (currentSubjectModel() === model.id) {
                  <span class="text-xs font-medium px-2 py-0.5 rounded-full border border-border bg-muted text-muted-foreground">Active</span>
                }
              </div>
              <p hlmCardDescription>{{ model.description }}</p>
            </div>
          </div>
        }
      </div>
    </section>
  </div>

  @if (isConfirming()) {
    <div class="modal-backdrop" (click)="cancelSelection()">
      <div hlmCard class="modal-content" (click)="$event.stopPropagation()">
        <div hlmCardHeader>
          <div class="flex items-center gap-2 text-destructive">
            <lucide-icon name="alert-triangle" size="20"></lucide-icon>
            <h2 hlmCardTitle class="text-destructive">Change {{ pendingSection() === 'vision' ? 'Vision' : 'Face Analysis' }} Model?</h2>
          </div>
          <p hlmCardDescription>This will trigger a full reindex of your library.</p>
        </div>

        <div hlmCardContent class="space-y-4">
          @if (isProcessing()) {
            <div class="space-y-3">
              <div class="flex justify-between text-xs font-medium">
                <span>{{ processingPhase() === 'downloading' ? 'Downloading model...' : 'Reindexing library...' }}</span>
                @if (processingPhase() === 'downloading') {
                  <span>{{ downloadProgress() | number:'1.0-0' }}%</span>
                }
              </div>
              @if (processingPhase() === 'downloading') {
                <div class="progress-bar-container">
                  <div class="progress-bar-fill" [style.width.%]="downloadProgress() ?? 0"></div>
                </div>
                <p class="text-[10px] text-muted-foreground truncate">{{ currentDownloadFile() }}</p>
              }
            </div>
          } @else {
            <div class="bg-muted/50 p-3 rounded text-sm space-y-2 border border-border">
              <p>&#8226; All face and subject data will be wiped and rebuilt.</p>
              <p>&#8226; Large libraries may take a long time to re-process.</p>
            </div>

            <div class="space-y-2">
              <p class="text-xs font-medium">Type <span class="font-bold">REINDEX</span> to confirm:</p>
              <input
                type="text"
                [value]="confirmInputValue()"
                (input)="confirmInputValue.set($any($event.target).value)"
                placeholder="REINDEX"
                class="confirm-input"
                autofocus
              >
            </div>
          }
        </div>

        <div hlmCardFooter class="flex justify-end gap-2">
          <button hlmBtn variant="secondary" (click)="cancelSelection()" [disabled]="isProcessing()">
            Cancel
          </button>
          <button
            hlmBtn
            variant="destructive"
            [disabled]="confirmInputValue() !== 'REINDEX' || isProcessing()"
            (click)="confirmSelection()"
          >
            Confirm Reindex
          </button>
        </div>
      </div>
    </div>
  }
</div>
```

- [ ] **Step 3: Verify frontend builds**

Run: `cd /home/pi/nebula && npx ng build 2>&1 | tail -10`
Expected: Build succeeds with no errors

- [ ] **Step 4: Commit**

```bash
git add src/app/components/settings/
git commit -m "feat(ui): add face analysis model picker to settings"
```

---

### Task 7: Full build verification

**Files:** None — verification only

- [ ] **Step 1: Full Rust build**

Run: `cd /home/pi/nebula/src-tauri && cargo build 2>&1 | tail -10`
Expected: `Finished` with no errors

- [ ] **Step 2: Run existing Rust tests**

Run: `cd /home/pi/nebula/src-tauri && cargo test 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 3: Full frontend build**

Run: `cd /home/pi/nebula && npx ng build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 4: Final commit (if any lint/format fixes needed)**

```bash
cargo fmt --check
npx ng lint
```
