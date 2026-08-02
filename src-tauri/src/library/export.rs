//! Copy library originals into a user-chosen export directory.

use std::path::{Path, PathBuf};

/// Build the preferred destination file name: `{parent}_{basename}`.
/// Falls back to basename when parent is empty, `.`, or a filesystem root.
pub fn preferred_dest_name(source: &Path) -> String {
    let basename = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("export.bin")
        .to_string();

    let parent_name = source
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != ".");

    match parent_name {
        Some(parent) => {
            // Strip any accidental path separators from parent for safety.
            let safe_parent: String = parent.chars().filter(|c| *c != '/' && *c != '\\').collect();
            if safe_parent.is_empty() {
                basename
            } else {
                format!("{safe_parent}_{basename}")
            }
        }
        None => basename,
    }
}

/// If `dest_dir/file_name` is free, return it; else insert ` (2)`, ` (3)`, …
/// before the extension until free. Never returns an existing path.
pub fn unique_dest_path(dest_dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dest_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let ext = path.extension().and_then(|s| s.to_str());

    let mut n = 2u32;
    loop {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dest_dir.join(&name);
        if !candidate.exists() {
            return candidate;
        }
        n = n.saturating_add(1);
        if n > 100_000 {
            return dest_dir.join(format!("{stem} ({n}).tmp"));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFilesResult {
    pub copied: u32,
    pub skipped_missing: u32,
    pub skipped_errors: u32,
}

/// Ensure `dest_dir` exists, is a directory, and is writable.
pub fn validate_dest_dir(dest_dir: &Path) -> Result<(), String> {
    if dest_dir.as_os_str().is_empty() {
        return Err("Destination path is empty".into());
    }
    let meta = std::fs::metadata(dest_dir)
        .map_err(|e| format!("Destination does not exist or is inaccessible: {e}"))?;
    if !meta.is_dir() {
        return Err("Destination is not a directory".into());
    }
    // Writable probe: create + remove a temp file.
    let probe = dest_dir.join(".nebula_export_write_probe");
    std::fs::write(&probe, b"").map_err(|e| format!("Destination is not writable: {e}"))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Copy each source path into `dest_dir` using parent-prefix naming.
/// Soft-skips missing sources and per-file I/O errors.
/// Calls `on_progress(current, total)` after each attempt (`current` is 1-based).
pub async fn copy_paths_to_dir<F>(
    sources: &[PathBuf],
    dest_dir: &Path,
    mut on_progress: F,
) -> Result<CopyFilesResult, String>
where
    F: FnMut(u32, u32),
{
    validate_dest_dir(dest_dir)?;

    let total = sources.len() as u32;
    let mut copied = 0u32;
    let mut skipped_missing = 0u32;
    let mut skipped_errors = 0u32;

    for (idx, src) in sources.iter().enumerate() {
        let current = (idx as u32).saturating_add(1);

        match tokio::fs::metadata(src).await {
            Ok(m) if m.is_file() => {}
            Ok(_) | Err(_) => {
                skipped_missing = skipped_missing.saturating_add(1);
                on_progress(current, total);
                continue;
            }
        }

        let name = preferred_dest_name(src);
        let dest = unique_dest_path(dest_dir, &name);

        match tokio::fs::copy(src, &dest).await {
            Ok(_) => copied = copied.saturating_add(1),
            Err(_) => skipped_errors = skipped_errors.saturating_add(1),
        }
        on_progress(current, total);
    }

    Ok(CopyFilesResult {
        copied,
        skipped_missing,
        skipped_errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nebula_export_{label}_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn preferred_name_uses_parent_prefix() {
        let p = PathBuf::from("/media/Photos/2024/Vacation/IMG_0001.jpg");
        assert_eq!(preferred_dest_name(&p), "Vacation_IMG_0001.jpg");
    }

    #[test]
    fn preferred_name_phone_prefix() {
        let p = PathBuf::from("/media/Phone/DSC_99.JPG");
        assert_eq!(preferred_dest_name(&p), "Phone_DSC_99.JPG");
    }

    #[test]
    fn preferred_name_root_falls_back_to_basename() {
        let p = PathBuf::from("/IMG_0001.jpg");
        assert_eq!(preferred_dest_name(&p), "IMG_0001.jpg");
    }

    #[test]
    fn preferred_name_strips_separators_from_parent() {
        let p = PathBuf::from("/tmp/normal/photo.jpg");
        let name = preferred_dest_name(&p);
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
        assert_eq!(name, "normal_photo.jpg");
    }

    #[test]
    fn unique_path_no_collision() {
        let dir = unique_temp_dir("unique_free");
        let out = unique_dest_path(&dir, "Vacation_IMG.jpg");
        assert_eq!(out, dir.join("Vacation_IMG.jpg"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_path_suffixes_on_collision() {
        let dir = unique_temp_dir("unique_col");
        fs::write(dir.join("Vacation_IMG.jpg"), b"a").unwrap();
        fs::write(dir.join("Vacation_IMG (2).jpg"), b"b").unwrap();
        let out = unique_dest_path(&dir, "Vacation_IMG.jpg");
        assert_eq!(out, dir.join("Vacation_IMG (3).jpg"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_path_preserves_multi_dot_names() {
        let dir = unique_temp_dir("unique_dot");
        fs::write(dir.join("Vacation_photo.raw.jpg"), b"a").unwrap();
        let out = unique_dest_path(&dir, "Vacation_photo.raw.jpg");
        assert_eq!(out, dir.join("Vacation_photo.raw (2).jpg"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn copy_paths_copies_with_parent_prefix() {
        let src_root = unique_temp_dir("copy_src");
        let vacation = src_root.join("Vacation");
        fs::create_dir_all(&vacation).unwrap();
        let a = vacation.join("IMG_01.jpg");
        let b = vacation.join("IMG_02.jpg");
        fs::write(&a, b"one").unwrap();
        fs::write(&b, b"two").unwrap();

        let dest = unique_temp_dir("copy_dest");
        let mut progress = Vec::new();
        let result = copy_paths_to_dir(&[a.clone(), b.clone()], &dest, |cur, total| {
            progress.push((cur, total));
        })
        .await
        .unwrap();

        assert_eq!(
            result,
            CopyFilesResult {
                copied: 2,
                skipped_missing: 0,
                skipped_errors: 0,
            }
        );
        assert!(dest.join("Vacation_IMG_01.jpg").exists());
        assert!(dest.join("Vacation_IMG_02.jpg").exists());
        assert_eq!(progress, vec![(1, 2), (2, 2)]);
        let _ = fs::remove_dir_all(&src_root);
        let _ = fs::remove_dir_all(&dest);
    }

    #[tokio::test]
    async fn copy_paths_skips_missing_source() {
        let src_root = unique_temp_dir("skip_src");
        let phone = src_root.join("Phone");
        fs::create_dir_all(&phone).unwrap();
        let good = phone.join("ok.jpg");
        fs::write(&good, b"ok").unwrap();
        let missing = phone.join("gone.jpg");

        let dest = unique_temp_dir("skip_dest");
        let result = copy_paths_to_dir(&[good, missing], &dest, |_, _| {})
            .await
            .unwrap();

        assert_eq!(result.copied, 1);
        assert_eq!(result.skipped_missing, 1);
        assert_eq!(result.skipped_errors, 0);
        assert!(dest.join("Phone_ok.jpg").exists());
        let _ = fs::remove_dir_all(&src_root);
        let _ = fs::remove_dir_all(&dest);
    }

    #[tokio::test]
    async fn copy_paths_never_overwrites_existing_dest() {
        let src_root = unique_temp_dir("ow_src");
        let folder = src_root.join("Trip");
        fs::create_dir_all(&folder).unwrap();
        let src = folder.join("shot.jpg");
        fs::write(&src, b"new").unwrap();

        let dest = unique_temp_dir("ow_dest");
        fs::write(dest.join("Trip_shot.jpg"), b"old").unwrap();

        let result = copy_paths_to_dir(&[src], &dest, |_, _| {}).await.unwrap();
        assert_eq!(result.copied, 1);
        assert_eq!(fs::read(dest.join("Trip_shot.jpg")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("Trip_shot (2).jpg")).unwrap(), b"new");
        let _ = fs::remove_dir_all(&src_root);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn validate_dest_rejects_missing_path() {
        let p = PathBuf::from("/no/such/export/dir/hopefully_nebula_export");
        let err = validate_dest_dir(&p).unwrap_err();
        assert!(
            err.to_lowercase().contains("not") || err.to_lowercase().contains("exist"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn validate_dest_accepts_writable_dir() {
        let dir = unique_temp_dir("valid");
        validate_dest_dir(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_dest_rejects_file_path() {
        let dir = unique_temp_dir("file_dest");
        let file = dir.join("not-a-dir");
        fs::write(&file, b"x").unwrap();
        assert!(validate_dest_dir(&file).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
