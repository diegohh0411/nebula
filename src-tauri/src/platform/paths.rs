use std::path::{Path, PathBuf};

pub fn thumbnail_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("thumbnails")
}

pub fn face_crop_cache_dir(data_dir: &Path) -> PathBuf {
    thumbnail_cache_dir(data_dir).join("face-crops")
}
