use tauri::{command, State};
use sqlx::Row;
use serde::Serialize;
use crate::AppState;
use crate::models::registry::{ModelSpec, FaceIdPreset, ModelType};

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub downloaded: bool,
    pub size_bytes: u64,
}

fn spec_downloaded(state: &AppState, spec: &ModelSpec) -> bool {
    let dir = state.model_manager.model_dir(spec);
    spec.all_files().iter().all(|f| dir.join(f.filename).exists())
}

fn preset_downloaded(state: &AppState, preset: &FaceIdPreset) -> bool {
    spec_downloaded(state, preset.detector)
        && spec_downloaded(state, preset.embedder)
        && spec_downloaded(state, preset.gender_age)
}

fn preset_size_bytes(preset: &FaceIdPreset) -> u64 {
    preset.detector.size_bytes + preset.embedder.size_bytes + preset.gender_age.size_bytes
}

#[command]
pub fn get_available_models(state: State<'_, AppState>) -> Vec<ModelInfo> {
    crate::models::registry::ALL_MODELS
        .iter()
        .filter(|m| matches!(m.model_type, ModelType::TextImageEmbedding))
        .map(|m| ModelInfo {
            id: m.id.to_string(),
            name: m.display_name.to_string(),
            description: m.display_description.to_string(),
            downloaded: spec_downloaded(&state, m),
            size_bytes: m.size_bytes,
        })
        .collect()
}

#[command]
pub fn get_available_subject_models(state: State<'_, AppState>) -> Vec<ModelInfo> {
    crate::models::registry::ALL_PRESETS
        .iter()
        .map(|p| ModelInfo {
            id: p.id.to_string(),
            name: p.name.to_string(),
            description: p.description.to_string(),
            downloaded: preset_downloaded(&state, p),
            size_bytes: preset_size_bytes(p),
        })
        .collect()
}

#[command]
pub async fn get_setting(state: State<'_, AppState>, key: String) -> Result<String, String> {
    let pool = &state.pool;
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(&key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(r) = row {
        return Ok(r.get("value"));
    }

    match key.as_str() {
        "embedding_model" => crate::models::registry::ALL_MODELS
            .iter()
            .find(|m| matches!(m.model_type, ModelType::TextImageEmbedding))
            .map(|m| m.id.to_string())
            .ok_or_else(|| "No embedding models registered".to_string()),
        "subject_model" => crate::models::registry::ALL_PRESETS
            .first()
            .map(|p| p.id.to_string())
            .ok_or_else(|| "No subject model presets registered".to_string()),
        _ => Err(format!("No default for unknown setting key: {}", key)),
    }
}

#[command]
pub async fn update_setting(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let pool = &state.pool;

    if key == "embedding_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            let spec = crate::models::registry::ModelSpec::find_by_id(&value)
                .ok_or_else(|| format!("Unknown model: {}", value))?;
            state.model_manager.ensure_ready(&app, spec).await.map_err(|e| e.to_string())?;
            crate::db::reset_all_embeddings(pool).await.map_err(|e| e.to_string())?;
            if let Ok(mut idx) = state.index.write() {
                *idx = Box::new(crate::search::vector_index::FlatIndex::new(768));
            }
            let idx_path = state.data_dir.join("nebula.idx");
            let _ = std::fs::remove_file(idx_path);
        }
    }

    if key == "subject_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            crate::db::reset_all_subject_data(pool).await.map_err(|e| e.to_string())?;
        }
    }

    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_embedding_model_matches_first_text_image_model() {
        let first = crate::models::registry::ALL_MODELS
            .iter()
            .find(|m| matches!(m.model_type, crate::models::registry::ModelType::TextImageEmbedding))
            .map(|m| m.id);
        assert_eq!(first, Some("onnx-community/siglip2-base-patch16-224-ONNX"));
    }

    #[test]
    fn default_subject_model_matches_first_preset() {
        let first = crate::models::registry::ALL_PRESETS.first().map(|p| p.id);
        assert_eq!(first, Some("blitz"));
    }
}
