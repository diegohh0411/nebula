use anyhow::Result;
use log::{info, warn, error, debug};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::{
    library::{repo, models::DbImage},
    models::{DebouncedEvent, DebouncedEventKind, SyncProgressPayload, SyncCompletePayload},
    library::watcher::FolderWatcher,
};

pub struct Indexer {
    pool: SqlitePool,
    #[allow(dead_code)]
    data_dir: PathBuf,
    folder_map: RwLock<Vec<(PathBuf, i64)>>,
    app: AppHandle,
    watcher: Arc<Mutex<FolderWatcher>>,
    hash_semaphore: Arc<Semaphore>,
    scan_mutex: Arc<tokio::sync::Mutex<()>>,
    preview: crate::media::preview::PreviewHandle,
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

fn stat_file(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)?;
    let file_size = meta.len() as i64;
    Some((mtime, file_size))
}

async fn compute_sha256(path: &Path) -> Result<String> {
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

fn find_folder_id(folder_map: &[(PathBuf, i64)], path: &Path) -> Option<i64> {
    folder_map
        .iter()
        .filter(|(p, _)| path.starts_with(p))
        .max_by_key(|(p, _)| p.as_os_str().len())
        .map(|(_, id)| *id)
}

fn walk_dir(dir: &Path, results: &mut Vec<(PathBuf, i64, i64, i64)>, folder_id: i64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, results, folder_id);
        } else if is_image(&path) {
            if let Some((mtime, file_size)) = stat_file(&path) {
                results.push((path, folder_id, mtime, file_size));
            }
        }
    }
}

fn walk_dir_for_scan(dir: &Path, results: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir_for_scan(&path, results);
        } else if is_image(&path) {
            results.push(path);
        }
    }
}

impl Indexer {
    pub async fn init(pool: SqlitePool, data_dir: PathBuf, app: AppHandle, preview: crate::media::preview::PreviewHandle) -> Result<Arc<Self>> {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher = Arc::new(Mutex::new(FolderWatcher::new(event_tx)?));

        let mut folder_map = Vec::new();
        {
            let folders = repo::list_all_folders(&pool).await?;
            let mut w = watcher.lock().await;
            for folder in &folders {
                let path = PathBuf::from(&folder.path);
                if path.exists() {
                    if let Err(e) = w.watch(path.clone(), folder.id) {
                    error!("Failed to watch folder {}: {}", folder.path, e);
                }
                }
                folder_map.push((path, folder.id));
            }
        }
        folder_map.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));

        let indexer = Arc::new(Self {
            pool,
            data_dir,
            folder_map: RwLock::new(folder_map),
            app,
            watcher,
            hash_semaphore: Arc::new(Semaphore::new(4)),
            scan_mutex: Arc::new(tokio::sync::Mutex::new(())),
            preview,
        });

        let debounce_indexer = indexer.clone();
        tokio::spawn(async move {
            crate::library::watcher::run_debounce_loop(event_rx, debounce_indexer).await;
        });

        Ok(indexer)
    }

    async fn sync_folder_map(&self) {
        if let Ok(folders) = repo::list_all_folders(&self.pool).await {
            let mut map: Vec<(PathBuf, i64)> = folders
                .into_iter()
                .map(|f| (PathBuf::from(f.path), f.id))
                .collect();
            map.sort_by(|a, b| b.0.as_os_str().len().cmp(&a.0.as_os_str().len()));
            *self.folder_map.write().await = map;
        }
    }

    async fn process_file(
        &self,
        path: &Path,
        folder_id: i64,
        known: Option<DbImage>,
    ) {
        if !is_image(path) {
            return;
        }
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => return,
        };

        let (mtime, file_size) = match stat_file(path) {
            Some(ms) => ms,
            None => return,
        };

        let existing = match known {
            Some(img) => Some(img),
            None => repo::get_image_metadata_by_path(&self.pool, &path_str)
                .await
                .ok()
                .flatten(),
        };

        match existing {
            None => {
                debug!("process_file: found new file: {}", path_str);

                // TT-BUGFIX: Defer SHA256 computation to make discovery "blitz-fast"
                // Insert with a placeholder hash to get it queued instantly.
                let image_id = match repo::insert_image(
                    &self.pool,
                    folder_id,
                    &path_str,
                    "",
                    file_size,
                    mtime,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        error!("Failed to insert image {}: {}", path_str, e);
                        return;
                    }
                };

                if let Err(e) = crate::pipeline::queue::enqueue_image(&self.pool, image_id).await {
                    error!("Failed to enqueue image {}: {}", image_id, e);
                }
                self.preview.enqueue_low(image_id);

                let _ = self.app.emit(
                    "image_added",
                    crate::models::ImageAddedPayload {
                        image_id,
                        path: path_str.clone(),
                    },
                );

                // Spawn a background task to compute and update the real hash
                let pool = self.pool.clone();
                let semaphore = self.hash_semaphore.clone();
                let path_buf = path.to_path_buf();
                tokio::spawn(async move {
                    if let Ok(_permit) = semaphore.acquire().await {
                        if let Ok(hash) = compute_sha256(&path_buf).await {
                            let _ = sqlx::query("UPDATE images SET file_hash = ? WHERE id = ?")
                                .bind(&hash)
                                .bind(image_id)
                                .execute(&pool)
                                .await;
                        }
                    }
                });
            }
            Some(existing) => {
                if mtime == existing.mtime && file_size == existing.file_size {
                    if existing.deleted_at.is_some() {
                        let _ = repo::clear_image_deleted(&self.pool, existing.id).await;
                        let _ = self.app.emit(
                            "image_added",
                            crate::models::ImageAddedPayload {
                                image_id: existing.id,
                                path: path_str,
                            },
                        );
                    }
                    return;
                }

                let _permit = match self.hash_semaphore.acquire().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        error!("Failed to acquire hash semaphore for {}: {}", path_str, e);
                        return;
                    }
                };
                let hash = match compute_sha256(path).await {
                    Ok(h) => h,
                    Err(e) => {
                        error!("Failed to hash {}: {}", path_str, e);
                        return;
                    }
                };

                if hash == existing.file_hash {
                    let _ =
                        repo::update_image_metadata(&self.pool, existing.id, file_size, mtime).await;
                    if existing.deleted_at.is_some() {
                        let _ = self.app.emit(
                            "image_added",
                            crate::models::ImageAddedPayload {
                                image_id: existing.id,
                                path: path_str,
                            },
                        );
                    }
                } else {
                    let _ = repo::update_image_hash_changed(
                        &self.pool,
                        existing.id,
                        &hash,
                        file_size,
                        mtime,
                    )
                    .await;
                    if let Err(e) = crate::pipeline::queue::enqueue_image(&self.pool, existing.id).await {
                        error!("Failed to enqueue image {}: {}", existing.id, e);
                    }
                    self.preview.enqueue_low(existing.id);
                    let _ = self.app.emit(
                        "image_updated",
                        crate::models::ImageUpdatedPayload {
                            image_id: existing.id,
                        },
                    );
                }
            }
        }
    }

    pub async fn start_rescan(&self) {
        let _guard = self.scan_mutex.lock().await;

        let folder_map = self.folder_map.read().await;
        let folders: Vec<(PathBuf, i64)> = folder_map
            .iter()
            .filter(|(p, _)| p.exists())
            .cloned()
            .collect();
        drop(folder_map);

        let disk_files: Vec<(PathBuf, i64, i64, i64)> = folders
            .par_iter()
            .flat_map(|(folder_path, folder_id)| {
                let mut results = Vec::new();
                walk_dir(folder_path, &mut results, *folder_id);
                results
            })
            .collect();

        let db_images = match repo::get_all_images_for_rescan(&self.pool).await {
            Ok(imgs) => imgs,
            Err(e) => {
                error!("Rescan: failed to load DB images: {}", e);
                return;
            }
        };

        let mut db_map: HashMap<String, DbImage> = HashMap::with_capacity(db_images.len());
        for img in db_images {
            db_map.insert(img.path.clone(), img);
        }

        let disk_set: std::collections::HashSet<String> = disk_files
            .iter()
            .map(|(p, _, _, _)| p.to_string_lossy().to_string())
            .collect();

        for (path, img) in &db_map {
            if img.deleted_at.is_none() && !disk_set.contains(path.as_str()) {
                let _ = repo::soft_delete_image_by_id(&self.pool, img.id).await;
            }
        }

        let total = disk_files.len() as u32;
        let mut done: u32 = 0;

        for (path, folder_id, _mtime, _file_size) in &disk_files {
            let path_str = path.to_string_lossy().to_string();
            let known = db_map.get(&path_str).cloned();

            self.process_file(path, *folder_id, known).await;

            done += 1;
            if done % 100 == 0 || done == total {
                let _ = self.app.emit(
                    "sync_progress",
                    SyncProgressPayload { done, total },
                );
            }
        }

        let _ = self.app.emit("sync_complete", SyncCompletePayload {});
    }

    pub async fn add_folder(&self, path: String) -> Result<crate::models::FolderWithCount> {
        let folder_id = repo::insert_folder(&self.pool, &path).await?;

        {
            let mut w = self.watcher.lock().await;
            if let Err(e) = w.watch(PathBuf::from(&path), folder_id) {
                error!("Failed to watch folder {}: {}", path, e);
            }
        }

        self.sync_folder_map().await;

        let folders = repo::list_folders_with_counts(&self.pool).await?;
        folders
            .into_iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| anyhow::anyhow!("Folder not found after insert"))
    }

    pub fn spawn_folder_scan(self: Arc<Self>, folder_path: PathBuf, folder_id: i64) {
        let handle = tokio::spawn(async move {
            self.start_folder_scan(&folder_path, folder_id).await;
        });

        tokio::spawn(async move {
            if let Err(e) = handle.await {
                error!("Folder scan task failed for folder_id {}: {}", folder_id, e);
            }
        });
    }

    pub async fn remove_folder(&self, id: i64) -> Result<()> {
        let folders = repo::list_folders_with_counts(&self.pool).await?;
        if let Some(folder) = folders.iter().find(|f| f.id == id) {
            let path = PathBuf::from(&folder.path);
            let mut w = self.watcher.lock().await;
            let _ = w.unwatch(&path);
        }

        repo::delete_folder(&self.pool, id).await?;
        self.sync_folder_map().await;
        Ok(())
    }

    pub async fn handle_event_batch(&self, events: Vec<DebouncedEvent>) {
        let folder_map = self.folder_map.read().await;
        for event in events {
            match event.kind {
                DebouncedEventKind::Create | DebouncedEventKind::Modify => {
                    if let Some(folder_id) = find_folder_id(&folder_map, &event.path) {
                        self.process_file(&event.path, folder_id, None).await;
                    }
                }
                DebouncedEventKind::Remove => {
                    let path_str = event.path.to_string_lossy().to_string();
                    if repo::soft_delete_image(&self.pool, &path_str).await.is_ok() {
                        let _ = self.app.emit(
                            "image_removed",
                            crate::models::ImageRemovedPayload { path: path_str },
                        );
                    }
                }
            }
        }
    }

    async fn start_folder_scan(&self, folder_path: &Path, folder_id: i64) {
        let _guard = self.scan_mutex.lock().await;

        let folder_path_owned = folder_path.to_path_buf();
        let entries: Vec<PathBuf> = match tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            walk_dir_for_scan(&folder_path_owned, &mut results);
            results
        })
        .await
        {
            Ok(e) => e,
            _ => return,
        };

        let db_map = repo::get_all_images_for_rescan(&self.pool)
            .await
            .ok()
            .map(|imgs| {
                imgs.into_iter()
                    .map(|i| (i.path.clone(), i))
                    .collect::<HashMap<String, DbImage>>()
            })
            .unwrap_or_default();

        for (i, path) in entries.iter().enumerate() {
            let path_str = path.to_string_lossy().to_string();
            let known = db_map.get(&path_str).cloned();
            self.process_file(path, folder_id, known).await;
            if (i + 1) % 10 == 0 {
                crate::search::math::emit_progress(&self.pool, &self.app).await;
            }
        }
        crate::search::math::emit_progress(&self.pool, &self.app).await;
    }
}
