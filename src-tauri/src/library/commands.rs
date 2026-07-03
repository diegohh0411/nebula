use crate::{
    library::repo,
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
        let folder_still_exists = repo::list_folders_with_counts(&pool)
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
pub async fn remove_folder(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let deleted_image_ids = state
        .indexer
        .remove_folder(id)
        .await
        .map_err(|e| e.to_string())?;

    {
        let mut idx = state.index.write().unwrap();
        for img_id in &deleted_image_ids {
            idx.remove(*img_id);
        }
    }

    let snap_path = state.data_dir.join("nebula.idx");
    let index_snap = std::sync::Arc::clone(&state.index);
    tokio::task::spawn_blocking(move || {
        let guard = index_snap.read().unwrap();
        if let Err(e) = guard.save(&snap_path) {
            log::error!("failed to save index snapshot during folder removal: {e}");
        }
    })
    .await
    .ok();

    Ok(())
}

#[tauri::command]
pub async fn list_folders(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FolderWithCount>, String> {
    repo::list_folders_with_counts(&state.pool)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn list_images(
    folder_id: Option<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Image>, String> {
    repo::list_images(&state.pool, folder_id)
        .await
        .map_err(map_err)
}
