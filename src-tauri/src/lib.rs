mod clustering;
mod commands;
mod config;
mod db;
mod embedder;
mod models;
mod search;
mod thumbnail;
mod vision_engine;
mod watcher;

use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;


use watcher::FolderWatcher;

pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub api_key: Arc<Mutex<Option<String>>>,
    pub watcher: Arc<Mutex<FolderWatcher>>,
    pub vision_engine: Arc<vision_engine::VisionEngine>,
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

            // Initialize DB
            let pool = tauri::async_runtime::block_on(db::init_db(&data_dir))?;

            // Load API key from config
            let api_key = config::read_api_key(&data_dir);
            let api_key = Arc::new(Mutex::new(api_key));

            // Set up the file watcher channel
            let (watcher_tx, watcher_rx) = tokio::sync::mpsc::unbounded_channel();
            let folder_watcher = FolderWatcher::new(watcher_tx)?;
            let watcher_arc = Arc::new(Mutex::new(folder_watcher));

            // Re-register watchers for already-stored folders
            {
                let pool_init = pool.clone();
                let watcher_init = watcher_arc.clone();
                tauri::async_runtime::block_on(async move {
                    if let Ok(folders) = db::list_all_folders(&pool_init).await {
                        let mut w = watcher_init.lock().await;
                        for folder in folders {
                            let path = PathBuf::from(&folder.path);
                            if path.exists() {
                                let _ = w.watch(path, folder.id);
                            }
                        }
                    }
                });
            }

            // Create VisionEngine
            let vision_engine = Arc::new(vision_engine::VisionEngine::new(data_dir.clone()));

            // Register app state
            app.manage(AppState {
                pool: pool.clone(),
                data_dir: data_dir.clone(),
                api_key: api_key.clone(),
                watcher: watcher_arc,
                vision_engine,
            });

            // Spawn watcher event consumer
            let pool_watcher = pool.clone();
            let app_handle_watcher = app.handle().clone();
            let data_dir_watcher = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                watcher::run_event_consumer(
                    watcher_rx,
                    pool_watcher,
                    app_handle_watcher,
                    data_dir_watcher,
                )
                .await;
            });

            // Startup rescan: pick up images added while the app was offline
            let pool_rescan = pool.clone();
            let app_handle_rescan = app.handle().clone();
            let data_dir_rescan = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(folders) = db::list_all_folders(&pool_rescan).await {
                    for folder in folders {
                        let path = PathBuf::from(&folder.path);
                        if path.exists() {
                            if let Err(e) = watcher::scan_folder(
                                &pool_rescan,
                                &app_handle_rescan,
                                folder.id,
                                &path,
                                &data_dir_rescan,
                            )
                            .await
                            {
                                eprintln!("Startup rescan failed for {}: {}", folder.path, e);
                            }
                        }
                    }
                }
            });

            // Ensure model files are present (downloads from HF on first run).
            // The embedding worker waits on this before processing any images.
            let vision_engine_model = Arc::clone(&app.state::<AppState>().vision_engine);
            let app_handle_model = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = vision_engine_model.ensure_model_ready(&app_handle_model).await {
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

            // Spawn semantic (SigLIP) embedding worker
            let pool_semantic = pool.clone();
            let app_handle_semantic = app.handle().clone();
            let vision_engine_semantic = Arc::clone(&app.state::<AppState>().vision_engine);
            tauri::async_runtime::spawn(async move {
                embedder::run_semantic_worker(pool_semantic, app_handle_semantic, vision_engine_semantic).await;
            });

            // Spawn subject (ArcFace + clustering) worker
            let pool_subject = pool.clone();
            let app_handle_subject = app.handle().clone();
            let vision_engine_subject = Arc::clone(&app.state::<AppState>().vision_engine);
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
            commands::set_api_key,
            commands::get_api_key,
            commands::regenerate_all_thumbnails,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
