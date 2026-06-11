use crate::{
    db,
    models::{Tag, TagWithCount, SubjectMatch},
    AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn add_subject_tag(
    subject_id: i64,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Tag, String> {
    db::add_subject_tag(&state.pool, subject_id, &name).await.map_err(map_err)
}

#[tauri::command]
pub async fn create_tag(
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Tag, String> {
    db::create_tag(&state.pool, &name).await.map_err(map_err)
}

#[tauri::command]
pub async fn remove_subject_tag(
    subject_id: i64,
    tag_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::remove_subject_tag(&state.pool, subject_id, tag_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn get_subject_tags(
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Tag>, String> {
    db::get_subject_tags(&state.pool, subject_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_tags(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TagWithCount>, String> {
    db::list_tags_with_counts(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn rename_tag(
    tag_id: i64,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::rename_tag(&state.pool, tag_id, &name).await.map_err(map_err)
}

#[tauri::command]
pub async fn delete_tag(
    tag_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::delete_tag(&state.pool, tag_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn get_tag_subjects(
    tag_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SubjectMatch>, String> {
    let pool = &state.pool;
    let rows = db::get_subjects_for_tag(pool, tag_id).await.map_err(map_err)?;
    let mut out = Vec::with_capacity(rows.len());
    for s in rows {
        let tags = db::get_subject_tags(pool, s.id).await.map_err(map_err)?;
        out.push(SubjectMatch { subject: s, tags });
    }
    Ok(out)
}
