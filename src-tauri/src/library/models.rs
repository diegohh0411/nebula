pub use crate::models::{Folder, FolderWithCount, Image};

#[derive(Clone)]
pub struct DbImage {
    pub id: i64,
    pub path: String,
    pub mtime: i64,
    pub file_size: i64,
    pub file_hash: String,
    pub hash_status: String,
    pub deleted_at: Option<i64>,
}
