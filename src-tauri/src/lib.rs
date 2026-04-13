mod db;
mod scanner;
mod search;
mod sidecar;

use db::*;
use scanner::*;
use sidecar::Embedder;
use serde_json::json;
use tokio::sync::Mutex;
use tauri::{Emitter, Manager};

pub struct AppState {
    db: Mutex<rusqlite::Connection>,
    embedder: Embedder,
}

#[tauri::command]
async fn add_folder(path: String, state: tauri::State<'_, AppState>) -> Result<Folder, String> {
    let conn = state.db.lock().await;
    let folder = db::add_folder(&conn, &path).map_err(|e| e.to_string())?;

    let scanned = scan_directory(std::path::Path::new(&path))?;
    let image_data: Vec<(String, String, Option<i64>)> = scanned
        .iter()
        .map(|img| (img.file_path.clone(), img.file_name.clone(), img.file_size))
        .collect();
    db::add_images(&conn, folder.id, &image_data).map_err(|e| e.to_string())?;

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
async fn start_embedding_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let images = {
        let conn = state.db.lock().await;
        db::get_unembedded_images(&conn).map_err(|e| e.to_string())?
    };

    if images.is_empty() {
        return Ok(());
    }

    let total = images.len();
    let app_handle = app.clone();

    tokio::spawn(async move {
        let state: tauri::State<'_, AppState> = app_handle.state();

        for (i, image) in images.iter().enumerate() {
            match state.embedder.embed_image(std::path::Path::new(&image.file_path)).await {
                Ok(embedding) => {
                    let bytes = embedding_to_bytes(&embedding);
                    let conn = state.db.lock().await;
                    let _ = db::store_embedding(&conn, image.id, &bytes);
                    let _ = db::mark_embedded(&conn, image.id);
                }
                Err(e) => {
                    eprintln!("Error embedding image {}: {}", image.file_path, e);
                    continue;
                }
            }

            let _ = app_handle.emit(
                "embedding-progress",
                json!({"current": i + 1, "total": total}),
            );
        }

        let _ = app_handle.emit("embedding-complete", json!({}));
    });

    Ok(())
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
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
