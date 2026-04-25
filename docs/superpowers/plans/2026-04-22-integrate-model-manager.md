# Integrate ModelManager into AppState and Workers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate the new `ModelManager` into the `AppState` and update all `VisionEngine` call sites to use it, ensuring models are downloaded and ready before use.

**Architecture:** 
- Add `ModelManager` to `AppState` for global access.
- Initialize `ModelManager` during app startup and ensure the default model is ready.
- Update background workers (`run_semantic_worker`, `run_subject_worker`) to take `ModelManager` and use appropriate `ModelSpec`.
- Update commands to use `ModelManager` for model lifecycle management.

**Tech Stack:** Rust, Tauri, SQLx, ONNX Runtime.

---

### Task 1: Update AppState and Initialization in lib.rs

**Files:**
- Modify: `.worktrees/model-management-system/src-tauri/src/lib.rs`

- [ ] **Step 1: Update `AppState` definition**
Add `model_manager: Arc<crate::models::ModelManager>` to `AppState`.

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
Initialize `ModelManager` and include it in `AppState`. Update the startup task to use `model_manager.ensure_ready`.

```rust
            let vision_engine = Arc::new(vision_engine::VisionEngine::new(data_dir.clone()));
            let model_manager = Arc::new(crate::models::ModelManager::new(data_dir.clone())); // Added

            let indexer = tauri::async_runtime::block_on(
                indexer::Indexer::init(pool.clone(), data_dir.clone(), app.handle().clone())
            )?;

            app.manage(AppState {
                pool: pool.clone(),
                data_dir: data_dir.clone(),
                indexer,
                vision_engine: vision_engine.clone(),
                model_manager: model_manager.clone(), // Added
                index: index.clone(),
            });

            // ... update startup task ...
            let model_manager_startup = Arc::clone(&model_manager);
            let app_handle_model = app.handle().clone();
            let pool_model = pool.clone();
            tauri::async_runtime::spawn(async move {
                let model_id = db::get_setting(&pool_model, "embedding_model")
                    .await
                    .unwrap_or(None)
                    .unwrap_or_else(|| "diegohh/siglip2-base-patch16-224".to_string());

                let spec = crate::models::registry::ModelSpec::find_by_id(&model_id)
                    .unwrap_or(&crate::models::registry::SIGLIP_BASE);

                if let Err(e) = model_manager_startup.ensure_ready(&app_handle_model, spec).await {
                    eprintln!("Model setup failed: {}", e);
                }
            });
```

- [ ] **Step 3: Update worker spawns in `run()`**
Update `run_semantic_worker` and `run_subject_worker` calls to include `model_manager`.

```rust
            let pool_semantic = pool.clone();
            let app_handle_semantic = app.handle().clone();
            let vision_engine_semantic = Arc::clone(&vision_engine);
            let model_manager_semantic = Arc::clone(&model_manager); // Added
            let index_semantic = index.clone();
            let data_dir_semantic = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                embedder::run_semantic_worker(
                    pool_semantic,
                    app_handle_semantic,
                    vision_engine_semantic,
                    model_manager_semantic, // Added
                    index_semantic,
                    data_dir_semantic,
                ).await;
            });

            let pool_subject = pool.clone();
            let app_handle_subject = app.handle().clone();
            let vision_engine_subject = Arc::clone(&vision_engine);
            let model_manager_subject = Arc::clone(&model_manager); // Added
            tauri::async_runtime::spawn(async move {
                embedder::run_subject_worker(
                    pool_subject, 
                    app_handle_subject, 
                    vision_engine_subject,
                    model_manager_subject // Added
                ).await;
            });
```

### Task 2: Update embedder.rs

**Files:**
- Modify: `.worktrees/model-management-system/src-tauri/src/embedder.rs`

- [ ] **Step 1: Update `run_semantic_worker` signature and logic**
Accept `model_manager` and use it to ensure the model is ready before embedding.

- [ ] **Step 2: Update `run_subject_worker` signature and logic**
Accept `model_manager` and use it with `get_face_analyzer`.

### Task 3: Update commands.rs

**Files:**
- Modify: `.worktrees/model-management-system/src-tauri/src/commands.rs`

- [ ] **Step 1: Update `search` command**
Use `model_manager` and `spec` when calling `embed_text`.

### Task 4: Update settings.rs

**Files:**
- Modify: `.worktrees/model-management-system/src-tauri/src/settings.rs`

- [ ] **Step 1: Update `get_available_models`**
- [ ] **Step 2: Update `get_available_subject_models`**

### Task 5: Final Verification

- [ ] **Step 1: Run `cargo build` in `src-tauri` directory.**
Run: `cd .worktrees/model-management-system/src-tauri && cargo build`
Expected: PASS
