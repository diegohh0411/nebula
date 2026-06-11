//! Tauri application wiring: Builder, setup, command registry.
pub mod state;
pub use state::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            crate::platform::logger::init(&data_dir);
            log::info!("Nebula backend initializing. Data directory: {:?}", data_dir);
            std::fs::create_dir_all(crate::thumbnail::thumbnail_cache_dir(&data_dir))?;
            std::fs::create_dir_all(crate::thumbnail::face_crop_cache_dir(&data_dir))?;

            let pool = tauri::async_runtime::block_on(crate::db::init_db(&data_dir))?;

            let flat_index = tauri::async_runtime::block_on(
                crate::vector_index::FlatIndex::load_or_rebuild(&data_dir, &pool)
            )?;
            let index: crate::vector_index::IndexStore = Arc::new(std::sync::RwLock::new(Box::new(flat_index)));

            let pipeline_config = crate::pipeline::PipelineConfig::default();
            let vision_engine = Arc::new(crate::vision_engine::VisionEngine::new(
                data_dir.clone(),
                pipeline_config.placement,
            ));
            let model_manager = Arc::new(crate::models::ModelManager::new(data_dir.clone()));

            let preview_handle = crate::preview::PreviewService::start(
                pool.clone(),
                app.handle().clone(),
                data_dir.clone(),
            );

            let indexer = tauri::async_runtime::block_on(
                crate::library::indexer::Indexer::init(pool.clone(), data_dir.clone(), app.handle().clone(), preview_handle.clone())
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
                let model_id = crate::db::get_setting(&pool_pipe, "embedding_model")
                    .await
                    .unwrap_or(None)
                    .unwrap_or_else(|| crate::models::registry::SIGLIP_BASE.id.to_string());
                let spec = crate::models::registry::ModelSpec::find_by_id(&model_id)
                    .unwrap_or(&crate::models::registry::SIGLIP_BASE);

                if let Err(e) = mm_pipe.ensure_ready(&app_pipe, spec).await {
                    eprintln!("Model setup failed: {}", e);
                }

                crate::pipeline::run_pipeline(
                    pool_pipe, app_pipe, ve_pipe, mm_pipe, index_pipe, data_dir_pipe,
                    pipeline_config,
                    spec,
                ).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::library::commands::add_folder,
            crate::library::commands::remove_folder,
            crate::library::commands::list_folders,
            crate::library::commands::list_images,
            crate::commands::prioritize_previews,
            crate::commands::search,
            crate::commands::get_processing_status,
            crate::commands::list_subjects,
            crate::commands::name_subject,
            crate::commands::list_faces,
            crate::commands::list_faces_for_image,
            crate::commands::get_face_crop,
            crate::commands::set_subject_thumbnail,
            crate::commands::get_subject_photos,
            crate::commands::get_subject_detail,
            crate::commands::get_merge_suggestions,
            crate::commands::merge_subjects,
            crate::commands::dismiss_merge_suggestion,
            crate::commands::assign_face_to_subject,
            crate::commands::create_subject_for_face,
            crate::commands::unassign_face,
            crate::commands::search_subjects,
            crate::commands::create_tag,
            crate::commands::add_subject_tag,
            crate::commands::remove_subject_tag,
            crate::commands::get_subject_tags,
            crate::commands::list_tags,
            crate::commands::rename_tag,
            crate::commands::delete_tag,
            crate::commands::get_tag_subjects,
            crate::settings::commands::get_available_models,
            crate::settings::commands::get_available_subject_models,
            crate::settings::commands::get_setting,
            crate::settings::commands::update_setting,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
