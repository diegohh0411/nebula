use crate::models::registry::{FaceIdPreset, ModelSpec, ModelType};
use crate::AppState;
use serde::Serialize;
use sqlx::Row;
use tauri::{command, State};

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
    spec.all_files()
        .iter()
        .all(|f| dir.join(f.filename).exists())
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

/// Resolve a possibly-unset `subject_model` setting value to its preset,
/// defaulting to Blitz — the preset actually used for every face embedded
/// before the §1 wiring fix, regardless of what the setting said. Also the
/// fallback for a value that no longer matches a known preset id.
pub(crate) fn resolve_subject_preset(value: Option<&str>) -> &'static crate::models::registry::FaceIdPreset {
    value
        .and_then(crate::models::registry::FaceIdPreset::find_by_id)
        .unwrap_or(&crate::models::registry::BUFFALO_S_PRESET)
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
        let current = crate::settings::repo::get_setting(pool, &key)
            .await
            .unwrap_or(None);
        if current.as_ref() != Some(&value) {
            let spec = crate::models::registry::ModelSpec::find_by_id(&value)
                .ok_or_else(|| format!("Unknown model: {}", value))?;
            state
                .model_manager
                .ensure_ready(&app, spec)
                .await
                .map_err(|e| e.to_string())?;
            crate::search::repo::reset_all_embeddings(pool)
                .await
                .map_err(|e| e.to_string())?;
            if let Ok(mut idx) = state.index.write() {
                *idx = Box::new(crate::search::vector_index::FlatIndex::new(768));
            }
            let idx_path = state.data_dir.join("nebula.idx");
            let _ = std::fs::remove_file(idx_path);
        }
    }

    if key == "subject_model" {
        let current = crate::settings::repo::get_setting(pool, &key)
            .await
            .unwrap_or(None);
        if current.as_ref() != Some(&value) {
            let preset = crate::models::registry::FaceIdPreset::find_by_id(&value)
                .ok_or_else(|| format!("Unknown preset: {}", value))?;
            state
                .model_manager
                .ensure_ready(&app, preset.detector)
                .await
                .map_err(|e| e.to_string())?;
            state
                .model_manager
                .ensure_ready(&app, preset.embedder)
                .await
                .map_err(|e| e.to_string())?;
            state
                .model_manager
                .ensure_ready(&app, preset.gender_age)
                .await
                .map_err(|e| e.to_string())?;

            let old_preset = resolve_subject_preset(current.as_deref());
            if old_preset.embedder.id != preset.embedder.id {
                crate::people::repo::mark_subject_data_stale(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
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
            .find(|m| {
                matches!(
                    m.model_type,
                    crate::models::registry::ModelType::TextImageEmbedding
                )
            })
            .map(|m| m.id);
        assert_eq!(first, Some("onnx-community/siglip2-base-patch32-256-ONNX"));
    }


    #[test]
    fn resolve_subject_preset_defaults_to_blitz_when_unset() {
        use super::resolve_subject_preset;
        let resolved = resolve_subject_preset(None);
        assert_eq!(resolved.id, "blitz");
    }

    #[test]
    fn resolve_subject_preset_falls_back_to_blitz_for_unknown_id() {
        use super::resolve_subject_preset;
        let resolved = resolve_subject_preset(Some("not-a-real-preset"));
        assert_eq!(resolved.id, "blitz");
    }

    #[test]
    fn resolve_subject_preset_returns_the_matching_preset() {
        use super::resolve_subject_preset;
        assert_eq!(resolve_subject_preset(Some("precision")).id, "precision");
    }

    #[test]
    fn unset_setting_and_explicit_blitz_resolve_to_the_same_embedder() {
        // Confirms selecting Blitz when nothing was previously set is a no-op
        // for staleness purposes: both resolve to the same embedder id, since
        // the wiring bug meant every pre-fix embedding was already buffalo_s.
        use super::resolve_subject_preset;
        assert_eq!(
            resolve_subject_preset(None).embedder.id,
            resolve_subject_preset(Some("blitz")).embedder.id
        );
    }

    #[test]
    fn switching_between_presets_changes_the_resolved_embedder_id() {
        use super::resolve_subject_preset;
        assert_ne!(
            resolve_subject_preset(Some("blitz")).embedder.id,
            resolve_subject_preset(Some("precision")).embedder.id
        );
    }
    #[test]
    fn default_subject_model_matches_first_preset() {
        let first = crate::models::registry::ALL_PRESETS.first().map(|p| p.id);
        assert_eq!(first, Some("blitz"));
    }
}
