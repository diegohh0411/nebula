use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::sync::mpsc;

use tauri::Emitter;

use crate::{db, models::ImageAddedPayload, thumbnail};

pub struct FolderWatcher {
    inner: RecommendedWatcher,
    watched: HashMap<PathBuf, i64>,
}

impl FolderWatcher {
    pub fn new(event_tx: mpsc::UnboundedSender<(Event, i64)>) -> Result<Self> {
        let tx = event_tx;
        // Build a watched map clone tracker — notify sends paths, we need folder_id
        // We'll use a shared map via a channel approach: forward raw events and let the consumer look up the folder_id
        let watched: HashMap<PathBuf, i64> = HashMap::new();
        // We need to map paths back to folder_ids in the event handler.
        // Since the closure can't borrow `watched`, we pass a clone at watch time using a secondary channel.
        // Simplest: send (Event, dummy_0) and let consumer do a DB lookup to find folder_id.
        let inner = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                // folder_id 0 = unknown; consumer will resolve from DB
                let _ = tx.send((event, 0));
            }
        })?;
        Ok(Self { inner, watched })
    }

    pub fn watch(&mut self, path: PathBuf, folder_id: i64) -> Result<()> {
        self.inner.watch(&path, RecursiveMode::Recursive)?;
        self.watched.insert(path, folder_id);
        Ok(())
    }

    pub fn unwatch(&mut self, path: &Path) -> Result<()> {
        self.inner.unwatch(path)?;
        self.watched.remove(path);
        Ok(())
    }

    pub fn watched_paths(&self) -> Vec<(PathBuf, i64)> {
        self.watched.iter().map(|(p, id)| (p.clone(), *id)).collect()
    }
}

fn is_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "png")
    )
}

async fn compute_sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let path = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await??;
    Ok(hash)
}

async fn get_mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp())
}

async fn handle_new_file(
    pool: &SqlitePool,
    app: &AppHandle,
    path: PathBuf,
    folder_id: i64,
    data_dir: &Path,
) {
    if !is_image(&path) {
        return;
    }
    let path_str = match path.to_str() {
        Some(s) => s.to_string(),
        None => return,
    };

    let hash = match compute_sha256(&path).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to hash {}: {}", path_str, e);
            return;
        }
    };
    let date_file = get_mtime(&path).await;

    let (image_id, is_new) =
        match db::upsert_image(pool, folder_id, &path_str, &hash, date_file).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to upsert image {}: {}", path_str, e);
                return;
            }
        };

    if is_new {
        // Enqueue for embedding
        let _ = db::enqueue_image(pool, image_id).await;

        // Generate thumbnail asynchronously
        let thumb_path = thumbnail::thumbnail_path_for(data_dir, image_id);
        let thumb_path_str = thumb_path.to_string_lossy().to_string();
        let pool_clone = pool.clone();
        let src = path.clone();
        let dest = thumb_path.clone();
        tokio::spawn(async move {
            if let Err(e) = thumbnail::generate_thumbnail(src, dest).await {
                eprintln!("Thumbnail generation failed: {}", e);
                return;
            }
            let _ = db::update_thumbnail_path(&pool_clone, image_id, &thumb_path_str).await;
        });

        // Emit image_added event
        let _ = app.emit(
            "image_added",
            ImageAddedPayload {
                image_id,
                path: path_str,
            },
        );
    }
}

async fn handle_modified_file(
    pool: &SqlitePool,
    app: &AppHandle,
    path: PathBuf,
    data_dir: &Path,
) {
    if !is_image(&path) {
        return;
    }
    let path_str = match path.to_str() {
        Some(s) => s.to_string(),
        None => return,
    };

    let existing = match db::get_image_by_path(pool, &path_str).await {
        Ok(Some(img)) => img,
        _ => return,
    };

    let new_hash = match compute_sha256(&path).await {
        Ok(h) => h,
        Err(_) => return,
    };

    if existing.file_hash != new_hash {
        let date_file = get_mtime(&path).await;
        let folder_id = existing.folder_id;
        if let Ok((image_id, _)) =
            db::upsert_image(pool, folder_id, &path_str, &new_hash, date_file).await
        {
            let _ = db::enqueue_image(pool, image_id).await;

            // Regenerate thumbnail
            let thumb_path = thumbnail::thumbnail_path_for(data_dir, image_id);
            let thumb_path_str = thumb_path.to_string_lossy().to_string();
            let pool_clone = pool.clone();
            tokio::spawn(async move {
                if let Err(e) = thumbnail::generate_thumbnail(path, thumb_path).await {
                    eprintln!("Thumbnail regeneration failed: {}", e);
                    return;
                }
                let _ = db::update_thumbnail_path(&pool_clone, image_id, &thumb_path_str).await;
            });

            let _ = app.emit(
                "image_updated",
                crate::models::ImageUpdatedPayload { image_id },
            );
        }
    }
}

/// Background task that consumes filesystem events from the watcher channel.
pub async fn run_event_consumer(
    mut rx: mpsc::UnboundedReceiver<(Event, i64)>,
    pool: SqlitePool,
    app: AppHandle,
    data_dir: PathBuf,
) {
    while let Some((event, _)) = rx.recv().await {
        // Look up the folder_id for the first affected path
        let folder_id = if let Some(p) = event.paths.first() {
            db::get_image_by_path(&pool, &p.to_string_lossy())
                .await
                .ok()
                .flatten()
                .map(|img| img.folder_id)
                .unwrap_or(0)
        } else {
            0
        };

        match event.kind {
            EventKind::Create(_) => {
                for path in event.paths {
                    // For new files we need to find which folder owns them
                    let fid = if folder_id != 0 {
                        folder_id
                    } else {
                        // Try to find the folder by matching path prefix
                        find_folder_for_path(&pool, &path).await.unwrap_or(0)
                    };
                    if fid != 0 {
                        handle_new_file(&pool, &app, path, fid, &data_dir).await;
                    }
                }
            }
            EventKind::Modify(notify::event::ModifyKind::Data(_))
            | EventKind::Modify(notify::event::ModifyKind::Any) => {
                for path in event.paths {
                    handle_modified_file(&pool, &app, path, &data_dir).await;
                }
            }
            EventKind::Remove(_) => {
                for path in event.paths {
                    let path_str = path.to_string_lossy().to_string();
                    let _ = db::soft_delete_image(&pool, &path_str).await;
                }
            }
            _ => {}
        }
    }
}

async fn find_folder_for_path(pool: &SqlitePool, path: &Path) -> Option<i64> {
    let folders = db::list_all_folders(pool).await.ok()?;
    let path_str = path.to_str()?;
    // Find the folder whose path is the longest prefix of this path
    folders
        .into_iter()
        .filter(|f| path_str.starts_with(&f.path))
        .max_by_key(|f| f.path.len())
        .map(|f| f.id)
}

/// Scan a folder recursively and insert all images.
pub async fn scan_folder(
    pool: &SqlitePool,
    app: &AppHandle,
    folder_id: i64,
    folder_path: &Path,
    data_dir: &Path,
) -> Result<()> {
    let entries = collect_image_paths(folder_path)?;
    for path in entries {
        handle_new_file(pool, app, path, folder_id, data_dir).await;
    }
    Ok(())
}

fn collect_image_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    collect_recursive(dir, &mut results)?;
    Ok(results)
}

fn collect_recursive(dir: &Path, results: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, results)?;
        } else if is_image(&path) {
            results.push(path);
        }
    }
    Ok(())
}
