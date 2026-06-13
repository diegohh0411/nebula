pub use crate::models::{Folder, Image, FolderWithCount};

#[derive(Clone)]
pub struct DbImage {
    pub id: i64,
    pub path: String,
    pub mtime: i64,
    pub file_size: i64,
    pub file_hash: String,
    #[allow(dead_code)]
    pub hash_status: String,
    pub deleted_at: Option<i64>,
}
