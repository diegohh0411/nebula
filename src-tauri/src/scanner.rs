use std::path::Path;
use walkdir::WalkDir;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

pub struct ScannedImage {
    pub file_path: String,
    pub file_name: String,
    pub file_size: Option<i64>,
}

pub fn scan_directory(dir: &Path) -> Result<Vec<ScannedImage>, String> {
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }

    let mut images = Vec::new();

    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase());

        let Some(ext) = extension else { continue };

        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let file_size = entry.metadata().ok().map(|m| m.len() as i64);

        images.push(ScannedImage {
            file_path: path.to_string_lossy().to_string(),
            file_name,
            file_size,
        });
    }

    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_nonexistent_directory() {
        let result = scan_directory(Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_filters_by_extension() {
        let dir = std::env::temp_dir().join("nebula_test_scan");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("photo.jpg"), "fake jpg").unwrap();
        fs::write(dir.join("photo.PNG"), "fake png").unwrap();
        fs::write(dir.join("photo.JPEG"), "fake jpeg").unwrap();
        fs::write(dir.join("document.pdf"), "fake pdf").unwrap();
        fs::write(dir.join("notes.txt"), "fake txt").unwrap();

        let images = scan_directory(&dir).unwrap();
        let names: Vec<&str> = images.iter().map(|i| i.file_name.as_str()).collect();
        assert!(names.contains(&"photo.jpg"));
        assert!(names.contains(&"photo.PNG"));
        assert!(names.contains(&"photo.JPEG"));
        assert!(!names.contains(&"document.pdf"));
        assert!(!names.contains(&"notes.txt"));
        assert_eq!(images.len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }
}
