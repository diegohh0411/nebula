use crate::AppState;

#[tauri::command]
pub async fn prioritize_previews(
    image_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.preview.prioritize(image_ids);
    Ok(())
}
