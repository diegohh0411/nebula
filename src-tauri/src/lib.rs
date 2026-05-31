mod clustering;
mod commands;
mod db;
mod embedder;
pub mod models;
pub mod pipeline;
mod preprocess;
mod preview;
mod search;
mod thumbnail;
pub mod vision_engine;
mod indexer;
mod vector_index;
mod watcher;
mod settings;

use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub indexer: Arc<indexer::Indexer>,
    pub vision_engine: Arc<vision_engine::VisionEngine>,
    pub model_manager: Arc<crate::models::ModelManager>,
    pub index: vector_index::IndexStore,
    pub preview: preview::PreviewHandle,
    pub throughput_ema: std::sync::atomic::AtomicU32,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(thumbnail::thumbnail_cache_dir(&data_dir))?;
            std::fs::create_dir_all(thumbnail::face_crop_cache_dir(&data_dir))?;

            let pool = tauri::async_runtime::block_on(db::init_db(&data_dir))?;

            let flat_index = tauri::async_runtime::block_on(
                vector_index::FlatIndex::load_or_rebuild(&data_dir, &pool)
            )?;
            let index: vector_index::IndexStore = Arc::new(std::sync::RwLock::new(Box::new(flat_index)));

            let pipeline_config = pipeline::PipelineConfig::default();
            let vision_engine = Arc::new(vision_engine::VisionEngine::new(
                data_dir.clone(),
                pipeline_config.placement,
            ));
            let model_manager = Arc::new(crate::models::ModelManager::new(data_dir.clone()));

            let preview_handle = preview::PreviewService::start(
                pool.clone(),
                app.handle().clone(),
                data_dir.clone(),
            );

            let indexer = tauri::async_runtime::block_on(
                indexer::Indexer::init(pool.clone(), data_dir.clone(), app.handle().clone(), preview_handle.clone())
            )?;

            app.manage(AppState {
                pool: pool.clone(),
                data_dir: data_dir.clone(),
                indexer,
                vision_engine: vision_engine.clone(),
                model_manager: model_manager.clone(),
                index: index.clone(),
                preview: preview_handle.clone(),
                throughput_ema: std::sync::atomic::AtomicU32::new(0.0f32.to_bits()),
            });

            let indexer_rescan = app.state::<AppState>().indexer.clone();
            tauri::async_runtime::spawn(async move {
                indexer_rescan.start_rescan().await;
            });

            let pool_pipe = pool.clone();
            let app_pipe = app.handle().clone();
            let ve_pipe = Arc::clone(&vision_engine);
            let mm_pipe = Arc::clone(&model_manager);
            let index_pipe = index.clone();
            let data_dir_pipe = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                // Resolve the user's chosen embedding model; default to SIGLIP_BASE.
                let model_id = db::get_setting(&pool_pipe, "embedding_model")
                    .await
                    .unwrap_or(None)
                    .unwrap_or_else(|| crate::models::registry::SIGLIP_BASE.id.to_string());
                let spec = crate::models::registry::ModelSpec::find_by_id(&model_id)
                    .unwrap_or(&crate::models::registry::SIGLIP_BASE);

                if let Err(e) = mm_pipe.ensure_ready(&app_pipe, spec).await {
                    eprintln!("Model setup failed: {}", e);
                }

                pipeline::run_pipeline(
                    pool_pipe, app_pipe, ve_pipe, mm_pipe, index_pipe, data_dir_pipe,
                    pipeline_config,
                    spec,
                ).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_folder,
            commands::remove_folder,
            commands::list_folders,
            commands::list_images,
            commands::prioritize_previews,
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
