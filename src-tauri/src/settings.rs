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
pub async fn update_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let pool = &state.pool;
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    
    Ok(())
}
