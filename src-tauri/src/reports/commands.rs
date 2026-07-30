use crate::{
    reports::{
        models::{CoverageReport, ProcessingProgress, SavedReport},
        repo,
    },
    AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn get_folder_coverage(
    folder_ids: Vec<i64>,
    tag_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<CoverageReport, String> {
    repo::get_folder_coverage(&state.pool, &folder_ids, &tag_ids)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn create_saved_report(
    name: String,
    folder_ids: Vec<i64>,
    tag_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<SavedReport, String> {
    repo::create_saved_report(&state.pool, &name, &folder_ids, &tag_ids)
        .await
        .map_err(map_err)
}

/// Bump every queued image in the report's source folders to the front of the
/// inference queue. Returns the number of queue entries moved.
#[tauri::command]
pub async fn prioritize_report_processing(
    report_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<u64, String> {
    let report = repo::get_saved_report(&state.pool, report_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Report not found".to_string())?;
    crate::pipeline::queue::prioritize_folders(&state.pool, &report.folder_ids)
        .await
        .map_err(map_err)
}

/// How many of the report's source-folder images are fully processed by the
/// pipeline. Drives the progress bar on the report detail page.
#[tauri::command]
pub async fn get_report_processing_progress(
    report_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<ProcessingProgress, String> {
    let report = repo::get_saved_report(&state.pool, report_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Report not found".to_string())?;
    repo::get_folders_processing_progress(&state.pool, &report.folder_ids)
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
