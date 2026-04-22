use tauri::{command, State};
use sqlx::Row;
use serde::Serialize;
use crate::AppState;

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[command]
pub fn get_available_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "diegohh/siglip2-base-patch16-224".into(),
            name: "Standard".into(),
            description: "Balanced quality and speed (86M params)".into(),
        },
        ModelInfo {
            id: "onnx-community/siglip2-base-patch32-256-ONNX".into(),
            name: "Fast".into(),
            description: "Optimized for consumer CPUs with larger patches".into(),
        },
    ]
}

#[command]
pub fn get_available_subject_models() -> Vec<ModelInfo> {
    crate::vision_engine::FaceIdConfig::ALL
        .iter()
        .map(|p| ModelInfo {
            id: p.id.to_string(),
            name: p.name.to_string(),
            description: p.description.to_string(),
        })
        .collect()
}

#[command]
pub async fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    let pool = &state.pool;
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(row.map(|r| r.get("value")))
}

#[command]
pub async fn update_setting(app: tauri::AppHandle, state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let pool = &state.pool;

    if key == "embedding_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            // First, ensure the model is downloaded and ready
            state.vision_engine.ensure_model_ready(&app, &value).await.map_err(|e| e.to_string())?;

            // Trigger full reset in DB
            crate::db::reset_all_embeddings(pool).await.map_err(|e| e.to_string())?;

            // Clear in-memory vector index
            if let Ok(mut idx) = state.index.write() {
                // Re-initialize with a standard dim, it will be corrected on first add if different
                *idx = Box::new(crate::vector_index::FlatIndex::new(768));
            }

            // Delete persisted index file to force rebuild from new embeddings
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
