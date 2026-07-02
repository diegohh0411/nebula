use crate::{
    reports::{
        models::{CoverageReport, SavedReport},
        repo,
    },
    AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn get_folder_coverage(
    folder_id: i64,
    tag_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<CoverageReport, String> {
    repo::get_folder_coverage(&state.pool, folder_id, &tag_ids)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn create_saved_report(
    name: String,
    folder_id: i64,
    tag_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<SavedReport, String> {
    repo::create_saved_report(&state.pool, &name, folder_id, &tag_ids)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn list_saved_reports(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SavedReport>, String> {
    repo::list_saved_reports(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn get_saved_report(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Option<SavedReport>, String> {
    repo::get_saved_report(&state.pool, id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn delete_saved_report(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    repo::delete_saved_report(&state.pool, id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn update_saved_report_name(
    state: tauri::State<'_, AppState>,
    id: i64,
    name: String,
) -> Result<(), String> {
    repo::update_saved_report_name(&state.pool, id, &name)
        .await
        .map_err(map_err)
}
