mod db;
mod scanner;
mod search;
mod embeddings;

use db::*;
use scanner::*;
use embeddings::Embedder;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum JobStatus {
    Idle,
    Processing,
    Completed,
    Error(String),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EmbeddingJobState {
    pub status: JobStatus,
    pub current_file: Option<String>,
    pub processed: usize,
    pub total: usize,
}

pub struct AppState {
    db: Mutex<rusqlite::Connection>,
    embedder: Embedder,
    job_state: Mutex<EmbeddingJobState>,
}

#[tauri::command]
async fn add_folder(
    app: tauri::AppHandle,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<Folder, String> {
    let conn = state.db.lock().await;
    let folder = db::add_folder(&conn, &path).map_err(|e| e.to_string())?;

    let scanned = scan_directory(std::path::Path::new(&path))?;
    let image_data: Vec<(String, String, Option<i64>)> = scanned
        .iter()
        .map(|img| (img.file_path.clone(), img.file_name.clone(), img.file_size))
        .collect();
    db::add_images(&conn, folder.id, &image_data).map_err(|e| e.to_string())?;
    
    // Release the lock before starting the background job
    drop(conn);

    // Automatically trigger embedding job
    let _ = internal_start_embedding_job(app, state).await;

    Ok(folder)
}

#[tauri::command]
async fn remove_folder(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().await;
    db::remove_folder(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_folders(state: tauri::State<'_, AppState>) -> Result<Vec<Folder>, String> {
    let conn = state.db.lock().await;
    db::list_folders(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_indexing_status(state: tauri::State<'_, AppState>) -> Result<IndexingStatus, String> {
    let conn = state.db.lock().await;
    db::get_indexing_status(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_images(
    offset: i64,
    limit: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ImageRecord>, String> {
    let conn = state.db.lock().await;
    db::get_images_paginated(&conn, offset, limit).map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_sidecar() -> Result<(), String> {
    Ok(())
}

#[tauri::command]
async fn stop_sidecar() -> Result<(), String> {
    Ok(())
}

#[tauri::command]
async fn sidecar_health() -> Result<bool, String> {
    Ok(true)
}

#[tauri::command]
async fn get_job_state(state: tauri::State<'_, AppState>) -> Result<EmbeddingJobState, String> {
    Ok(state.job_state.lock().await.clone())
}

async fn internal_start_embedding_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut job_state = state.job_state.lock().await;
    if let JobStatus::Processing = job_state.status {
        return Ok(()); // Already running
    }

    let images = {
        let conn = state.db.lock().await;
        db::get_unembedded_images(&conn).map_err(|e| e.to_string())?
    };

    if images.is_empty() {
        job_state.status = JobStatus::Idle;
        return Ok(());
    }

    job_state.status = JobStatus::Processing;
    job_state.total = images.len();
    job_state.processed = 0;
    job_state.current_file = None;
    drop(job_state);

    let app_handle = app.clone();
    tokio::spawn(async move {
        let state: tauri::State<'_, AppState> = app_handle.state();

        let total = images.len();
        for (i, image) in images.iter().enumerate() {
            {
                let mut js = state.job_state.lock().await;
                js.current_file = Some(image.file_name.clone());
                js.processed = i;
            }

            let result = state.embedder.embed_image(std::path::Path::new(&image.file_path)).await;
            
            match result {
                Ok(embedding) => {
                    let bytes = embedding_to_bytes(&embedding);
                    let conn = state.db.lock().await;
                    let _ = db::store_embedding(&conn, image.id, &bytes);
                    let _ = db::mark_embedded(&conn, image.id);
                }
                Err(e) => {
                    eprintln!("Error embedding image {}: {}", image.file_path, e);
                    // Continue with next image
                }
            }

            let current_js = state.job_state.lock().await.clone();
            let _ = app_handle.emit("embedding-progress", current_js);
        }

        let mut final_js = state.job_state.lock().await;
        final_js.status = JobStatus::Completed;
        final_js.current_file = None;
        final_js.processed = total;
        let final_js_clone = final_js.clone();
        drop(final_js);

        let _ = app_handle.emit("embedding-complete", final_js_clone);
    });

    Ok(())
}

#[tauri::command]
async fn start_embedding_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    internal_start_embedding_job(app, state).await
}

#[tauri::command]
async fn search_images(
    query: String,
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<db::SearchResult>, String> {
    let query_embedding = state.embedder.embed_text(&query).await.map_err(|e| e.to_string())?;

    let conn = state.db.lock().await;
    search::search_images(&conn, &query_embedding, limit).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let api_key = std::env::var("GOOGLE_API_KEY").unwrap_or_else(|_| "YOUR_API_KEY".to_string());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            add_folder,
            remove_folder,
            list_folders,
            get_indexing_status,
            get_images,
            start_sidecar,
            stop_sidecar,
            sidecar_health,
            start_embedding_job,
            get_job_state,
            search_images,
        ])
        .setup(move |app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");
            std::fs::create_dir_all(&app_data_dir).expect("Failed to create app data directory");

            let db_path = app_data_dir.join("nebula.db");
            let db_conn =
                rusqlite::Connection::open(&db_path).expect("Failed to open database");
            db::init_db(&db_conn).expect("Failed to initialize database");

            app.manage(AppState {
                db: Mutex::new(db_conn),
                embedder: Embedder::new(api_key),
                job_state: Mutex::new(EmbeddingJobState {
                    status: JobStatus::Idle,
                    current_file: None,
                    processed: 0,
                    total: 0,
                }),
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
