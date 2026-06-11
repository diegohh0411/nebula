use crate::{
    db,
    models::{FolderWithCount, Image},
    AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn add_folder(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<FolderWithCount, String> {
    let folder = state
        .indexer
        .add_folder(path.clone())
        .await
        .map_err(|e| e.to_string())?;
    let indexer = state.indexer.clone();
    let pool = state.pool.clone();
    let scan_path = std::path::PathBuf::from(&path);
    let folder_id = folder.id;

    tauri::async_runtime::spawn(async move {
        let folder_still_exists = db::list_folders_with_counts(&pool)
            .await
            .map(|folders| folders.iter().any(|f| f.id == folder_id))
            .unwrap_or(false);

        if folder_still_exists {
            indexer.spawn_folder_scan(scan_path, folder_id);
        }
    });

    Ok(folder)
}

#[tauri::command]
pub async fn remove_folder(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .indexer
        .remove_folder(id)
        .await
        .map_err(|e| e.to_string())
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
