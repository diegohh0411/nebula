use reqwest::Client;
use tauri::AppHandle;

use crate::{
    config, db,
    models::{EmbedStatus, FolderWithCount, Image, SearchResult},
    search, thumbnail, watcher, AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn add_folder(
    path: String,
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<FolderWithCount, String> {
    let pool = &state.pool;
    let data_dir = &state.data_dir;

    // Insert folder into DB
    let folder_id = db::insert_folder(pool, &path).await.map_err(map_err)?;

    // Scan folder for existing images
    let folder_path = std::path::PathBuf::from(&path);
    watcher::scan_folder(pool, &app, folder_id, &folder_path, data_dir)
        .await
        .map_err(map_err)?;

    // Register the filesystem watcher
    {
        let mut w = state.watcher.lock().await;
        if let Err(e) = w.watch(folder_path, folder_id) {
            eprintln!("Failed to register watcher for folder {folder_id}: {e}");
        }
    }

    // Return updated folder with count
    let folders = db::list_folders_with_counts(pool).await.map_err(map_err)?;
    folders
        .into_iter()
        .find(|f| f.id == folder_id)
        .ok_or_else(|| "Folder not found after insert".to_string())
}

#[tauri::command]
pub async fn remove_folder(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let pool = &state.pool;

    // Find the folder path before deleting
    let folders = db::list_folders_with_counts(pool).await.map_err(map_err)?;
    if let Some(folder) = folders.iter().find(|f| f.id == id) {
        let path = std::path::PathBuf::from(&folder.path);
        let mut w = state.watcher.lock().await;
        if let Err(e) = w.unwatch(&path) {
            eprintln!("Failed to unregister watcher for path {}: {e}", path.display());
        }
    }

    db::delete_folder(pool, id).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_folders(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FolderWithCount>, String> {
    db::list_folders_with_counts(&state.pool)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn list_images(
    folder_id: Option<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Image>, String> {
    db::list_images(&state.pool, folder_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn search_images(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let api_key = {
        let lock = state.api_key.lock().await;
        lock.clone()
    };
    let api_key = api_key.ok_or_else(|| "API key not configured".to_string())?;

    let client = Client::new();
    let query_embedding = crate::embedder::embed_text(&client, &api_key, &query)
        .await
        .map_err(|e| format!("Search requires a connection — try again when online. ({e})"))?;

    let scored = search::search_images(&state.pool, query_embedding, 50)
        .await
        .map_err(map_err)?;

    search::build_search_results(&state.pool, scored)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn search_similar_images(
    image_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let pool = &state.pool;
    let embedding = db::get_image_embedding(pool, image_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Embedding not found for image — try indexing first".to_string())?;

    let embedding_f32 = crate::embedder::bytes_to_f32_vec(&embedding);

    let scored = search::search_images(pool, embedding_f32, 50)
        .await
        .map_err(map_err)?;

    // Exclude the source image from its own results
    let filtered_scored = scored
        .into_iter()
        .filter(|(id, _)| *id != image_id)
        .collect();

    search::build_search_results(pool, filtered_scored)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_embed_status(
    state: tauri::State<'_, AppState>,
) -> Result<EmbedStatus, String> {
    db::get_embed_counts(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn set_api_key(
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    config::write_api_key(&state.data_dir, &key).map_err(map_err)?;
    let mut lock = state.api_key.lock().await;
    *lock = Some(key);
    Ok(())
}

#[tauri::command]
pub async fn get_api_key(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let lock = state.api_key.lock().await;
    Ok(lock.clone())
}

#[tauri::command]
pub async fn generate_thumbnails_for_folder(
    folder_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let images = db::list_images(&state.pool, Some(folder_id))
        .await
        .map_err(map_err)?;

    let pool = state.pool.clone();
    let data_dir = state.data_dir.clone();
    tokio::spawn(async move {
        for image in images.iter().filter(|i| i.thumbnail_path.is_none()) {
            let thumb_path = thumbnail::thumbnail_path_for(&data_dir, image.id);
            let thumb_str = thumb_path.to_string_lossy().to_string();
            let src = std::path::PathBuf::from(&image.path);
            if let Ok(()) = thumbnail::generate_thumbnail(src, thumb_path).await {
                let _ = db::update_thumbnail_path(&pool, image.id, &thumb_str).await;
            }
        }
    });

    Ok(())
}
