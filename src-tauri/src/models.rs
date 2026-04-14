use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub added_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Image {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub file_hash: String,
    pub date_taken: Option<i64>,
    pub date_file: i64,
    pub thumbnail_path: Option<String>,
    pub embed_status: String, // "pending" | "done" | "failed"
    pub added_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FolderWithCount {
    pub id: i64,
    pub path: String,
    pub added_at: i64,
    pub photo_count: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EmbedStatus {
    pub pending: i64,
    pub done: i64,
}

// Tauri event payloads
#[derive(Debug, Serialize, Clone)]
pub struct EmbedProgressPayload {
    pub pending: i64,
    pub done: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct ImageAddedPayload {
    pub image_id: i64,
    pub path: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ImageUpdatedPayload {
    pub image_id: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub image_id: i64,
    pub path: String,
    pub thumbnail_path: Option<String>,
    pub score: f32,
    pub date_taken: Option<i64>,
    pub date_file: i64,
    pub embed_status: String,
}
