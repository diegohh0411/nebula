mod clustering;
mod commands;
mod db;
mod embedder;
mod models;
mod search;
mod thumbnail;
mod vision_engine;
mod indexer;
mod vector_index;
mod watcher;
mod settings;

use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager};

pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub indexer: Arc<indexer::Indexer>,
    pub vision_engine: Arc<vision_engine::VisionEngine>,
    pub index: vector_index::IndexStore,
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

            let vision_engine = Arc::new(vision_engine::VisionEngine::new(data_dir.clone()));

            let indexer = tauri::async_runtime::block_on(
                indexer::Indexer::init(pool.clone(), data_dir.clone(), app.handle().clone())
            )?;

            app.manage(AppState {
                pool: pool.clone(),
                data_dir: data_dir.clone(),
                indexer,
                vision_engine: vision_engine.clone(),
                index: index.clone(),
            });

            let indexer_rescan = app.state::<AppState>().indexer.clone();
            tauri::async_runtime::spawn(async move {
                indexer_rescan.start_rescan().await;
            });

            let vision_engine_model = Arc::clone(&vision_engine);
            let app_handle_model = app.handle().clone();
            let pool_model = pool.clone();
            tauri::async_runtime::spawn(async move {
                let model_id = db::get_setting(&pool_model, "embedding_model")
                    .await
                    .unwrap_or(None)
                    .unwrap_or_else(|| "diegohh/siglip2-base-patch16-224".to_string());

                if let Err(e) = vision_engine_model.ensure_model_ready(&app_handle_model, &model_id).await {
                    eprintln!("Model setup failed: {}", e);
                    let _ = app_handle_model.emit(
                        "model_download_progress",
                        crate::models::ModelDownloadPayload {
                            file: String::new(),
                            bytes_done: 0,
                            bytes_total: None,
                            done: false,
                            error: Some(e.to_string()),
                        },
                    );
                }
            });

            let pool_semantic = pool.clone();
            let app_handle_semantic = app.handle().clone();
            let vision_engine_semantic = Arc::clone(&vision_engine);
            let index_semantic = index.clone();
            let data_dir_semantic = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                embedder::run_semantic_worker(
                    pool_semantic,
                    app_handle_semantic,
                    vision_engine_semantic,
                    index_semantic,
                    data_dir_semantic,
                ).await;
            });

            let pool_subject = pool.clone();
            let app_handle_subject = app.handle().clone();
            let vision_engine_subject = Arc::clone(&vision_engine);
            tauri::async_runtime::spawn(async move {
                embedder::run_subject_worker(pool_subject, app_handle_subject, vision_engine_subject).await;
            });

            Ok(())
        })
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
            settings::get_setting,
            settings::update_setting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
