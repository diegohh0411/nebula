mod db;
mod scanner;

use db::*;
use scanner::*;
use search::*;
use sidecar::*;
use serde_json::json;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub struct AppState {
    db: Mutex<rusqlite::Connection>,
    sidecar: Mutex<SidecarManager>,
}

#[tauri::command]
fn add_folder(path: String, state: tauri::State<'_, AppState>) -> Result<Folder, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
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
fn remove_folder(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::remove_folder(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_folders(state: tauri::State<'_, AppState>) -> Result<Vec<Folder>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::list_folders(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_indexing_status(state: tauri::State<'_, AppState>) -> Result<IndexingStatus, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_indexing_status(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_images(
    offset: i64,
    limit: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ImageRecord>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_images_paginated(&conn, offset, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn start_sidecar(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    sidecar.start()
}

#[tauri::command]
fn sidecar_health(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    if !sidecar.is_ready() {
        return Ok(false);
    }
    drop(sidecar);
    let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    match sidecar.send_request(&json!({"action": "health_check"})) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
fn stop_sidecar(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    sidecar.shutdown()
}

#[tauri::command]
fn start_embedding_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let images = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get_unembedded_images(&conn).map_err(|e| e.to_string())?
    };

    if images.is_empty() {
        return Ok(());
    }

    let total = images.len();
    let app_handle = app.clone();

    std::thread::spawn(move || {
        let state: tauri::State<'_, AppState> = app_handle.state();

        for (i, image) in images.iter().enumerate() {
            let request = json!({
                "action": "embed_image",
                "image_path": image.file_path
            });

            let response = {
                let mut sidecar = match state.sidecar.lock() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                sidecar.send_request(&request)
            };

            match response {
                Ok(resp) => {
                    if let Some(embedding_arr) =
                        resp.get("embedding").and_then(|e| e.as_array())
                    {
                        let embedding: Vec<f32> = embedding_arr
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();

                        let bytes = embedding_to_bytes(&embedding);

                        let conn = match state.db.lock() {
                            Ok(c) => c,
                            Err(_) => break,
                        };
                        let _ = db::store_embedding(&conn, image.id, &bytes);
                        let _ = db::mark_embedded(&conn, image.id);
                    }
                }
                Err(_) => continue,
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
fn search_images(
    query: String,
    limit: usize,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let text_request = json!({
        "action": "embed_text",
        "text": query
    });

    let query_embedding = {
        let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
        let response = sidecar.send_request(&text_request)?;

        let embedding_arr = response
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or("No embedding in response")?;

        embedding_arr
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect::<Vec<f32>>()
    };

    let all_embeddings = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get_all_embeddings(&conn).map_err(|e| e.to_string())?
    };

    Ok(search_embeddings(&query_embedding, &all_embeddings, limit))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
        .setup(|app| {
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
                sidecar: Mutex::new(SidecarManager::new()),
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
