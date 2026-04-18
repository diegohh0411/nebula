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
pub struct ModelDownloadPayload {
    pub file: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    /// true on the final chunk for each file
    pub done: bool,
    /// non-empty if the download failed
    pub error: Option<String>,
}

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

#[derive(Debug, Serialize, Clone)]
pub struct ImageRemovedPayload {
    pub path: String,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Subject {
    pub id: i64,
    pub name: Option<String>,
    pub thumbnail_face_id: Option<i64>,
    #[serde(rename = "type")]
    pub subject_type: String,
    pub added_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Face {
    pub id: i64,
    pub image_id: i64,
    pub subject_id: Option<i64>,
    pub bbox_x: f64,
    pub bbox_y: f64,
    pub bbox_w: f64,
    pub bbox_h: f64,
    pub embedding: Option<Vec<u8>>,
    pub added_at: i64,
    pub is_manual: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubjectDetail {
    pub subject: Subject,
    pub photo_count: i64,
    pub face_count: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MergeSuggestion {
    pub id: i64,
    pub subject_a: Subject,
    pub subject_b: Subject,
    pub cross_match_count: i64,
    pub total_pairs: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NameSubjectResult {
    pub duplicate_subject_id: Option<i64>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SearchQuery {
    Text { query: String },
    ImageId { image_id: i64 },
    ImageBytes { data: String, mime_type: String },
}
