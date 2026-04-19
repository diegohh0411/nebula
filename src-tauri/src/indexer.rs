use anyhow::Result;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::{
    db,
    models::{DebouncedEvent, DebouncedEventKind, SyncProgressPayload, SyncCompletePayload},
    thumbnail,
    watcher::FolderWatcher,
};

pub struct Indexer {
    pool: SqlitePool,
    data_dir: PathBuf,
    folder_map: RwLock<Vec<(PathBuf, i64)>>,
    app: AppHandle,
    watcher: Arc<Mutex<FolderWatcher>>,
    hash_semaphore: Arc<Semaphore>,
    scan_mutex: Arc<tokio::sync::Mutex<()>>,
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
    let path_str = path.to_str()?;
    folder_map
        .iter()
        .filter(|(p, _)| path_str.starts_with(p.to_str().unwrap_or("")))
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
    pub async fn init(pool: SqlitePool, data_dir: PathBuf, app: AppHandle) -> Result<Arc<Self>> {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher = Arc::new(Mutex::new(FolderWatcher::new(event_tx)?));

        let mut folder_map = Vec::new();
        {
            let folders = db::list_all_folders(&pool).await?;
            let mut w = watcher.lock().await;
            for folder in &folders {
                let path = PathBuf::from(&folder.path);
                if path.exists() {
                    if let Err(e) = w.watch(path.clone(), folder.id) {
                    eprintln!("Failed to watch folder {}: {}", folder.path, e);
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
        });

        let debounce_indexer = indexer.clone();
        tokio::spawn(async move {
            crate::watcher::run_debounce_loop(event_rx, debounce_indexer).await;
        });

        Ok(indexer)
    }

    async fn sync_folder_map(&self) {
        if let Ok(folders) = db::list_all_folders(&self.pool).await {
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
        known: Option<db::DbImage>,
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
            None => db::get_image_metadata_by_path(&self.pool, &path_str)
                .await
                .ok()
                .flatten(),
        };

        match existing {
            None => {
                let _permit = self.hash_semaphore.acquire().await;
                let hash = match compute_sha256(path).await {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("Failed to hash {}: {}", path_str, e);
                        return;
                    }
                };

                let image_id = match db::insert_image(
                    &self.pool,
                    folder_id,
                    &path_str,
                    &hash,
                    file_size,
                    mtime,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Failed to insert image {}: {}", path_str, e);
                        return;
                    }
                };

                if let Err(e) = db::enqueue_image(&self.pool, image_id).await {
                    eprintln!("Failed to enqueue image {}: {}", image_id, e);
                }
                self.spawn_thumbnail(image_id, path.to_path_buf());

                let _ = self.app.emit(
                    "image_added",
                    crate::models::ImageAddedPayload {
                        image_id,
                        path: path_str,
                    },
                );
            }
            Some(existing) => {
                if mtime == existing.mtime && file_size == existing.file_size {
                    if existing.deleted_at.is_some() {
                        let _ = db::clear_image_deleted(&self.pool, existing.id).await;
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

                let _permit = self.hash_semaphore.acquire().await;
                let hash = match compute_sha256(path).await {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("Failed to hash {}: {}", path_str, e);
                        return;
                    }
                };

                if hash == existing.file_hash {
                    let _ =
                        db::update_image_metadata(&self.pool, existing.id, file_size, mtime).await;
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
                    let _ = db::update_image_hash_changed(
                        &self.pool,
                        existing.id,
                        &hash,
                        file_size,
                        mtime,
                    )
                    .await;
                    if let Err(e) = db::enqueue_image(&self.pool, existing.id).await {
                        eprintln!("Failed to enqueue image {}: {}", existing.id, e);
                    }
                    self.spawn_thumbnail(existing.id, path.to_path_buf());
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

    fn spawn_thumbnail(&self, image_id: i64, src: PathBuf) {
        let thumb_path = thumbnail::thumbnail_path_for(&self.data_dir, image_id);
        let thumb_path_str = thumb_path.to_string_lossy().to_string();
        let pool = self.pool.clone();
        let app = self.app.clone();
        tokio::spawn(async move {
            if let Err(e) = thumbnail::generate_thumbnail(src, thumb_path).await {
                eprintln!("Thumbnail generation failed: {}", e);
                return;
            }
            if db::update_thumbnail_path(&pool, image_id, &thumb_path_str)
                .await
                .is_ok()
            {
                let _ = app.emit(
                    "image_updated",
                    crate::models::ImageUpdatedPayload { image_id },
                );
            }
        });
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

        let db_images = match db::get_all_images_for_rescan(&self.pool).await {
            Ok(imgs) => imgs,
            Err(e) => {
                eprintln!("Rescan: failed to load DB images: {}", e);
                return;
            }
        };

        let mut db_map: HashMap<String, db::DbImage> = HashMap::with_capacity(db_images.len());
        for img in db_images {
            db_map.insert(img.path.clone(), img);
        }

        let disk_set: std::collections::HashSet<String> = disk_files
            .iter()
            .map(|(p, _, _, _)| p.to_string_lossy().to_string())
            .collect();

        for (path, img) in &db_map {
            if img.deleted_at.is_none() && !disk_set.contains(path.as_str()) {
                let _ = db::soft_delete_image_by_id(&self.pool, img.id).await;
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
        let folder_id = db::insert_folder(&self.pool, &path).await?;

        {
            let mut w = self.watcher.lock().await;
            if let Err(e) = w.watch(PathBuf::from(&path), folder_id) {
                eprintln!("Failed to watch folder {}: {}", path, e);
            }
        }

        self.sync_folder_map().await;

        let folder_path = PathBuf::from(&path);
        self.start_folder_scan(&folder_path, folder_id).await;

        let folders = db::list_folders_with_counts(&self.pool).await?;
        folders
            .into_iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| anyhow::anyhow!("Folder not found after insert"))
    }

    pub async fn remove_folder(&self, id: i64) -> Result<()> {
        let folders = db::list_folders_with_counts(&self.pool).await?;
        if let Some(folder) = folders.iter().find(|f| f.id == id) {
            let path = PathBuf::from(&folder.path);
            let mut w = self.watcher.lock().await;
            let _ = w.unwatch(&path);
        }

        db::delete_folder(&self.pool, id).await?;
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
                    if db::soft_delete_image(&self.pool, &path_str).await.is_ok() {
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

        let db_map = db::get_all_images_for_rescan(&self.pool)
            .await
            .ok()
            .map(|imgs| {
                imgs.into_iter()
                    .map(|i| (i.path.clone(), i))
                    .collect::<HashMap<String, db::DbImage>>()
            })
            .unwrap_or_default();

        for path in entries {
            let path_str = path.to_string_lossy().to_string();
            let known = db_map.get(&path_str).cloned();
            self.process_file(&path, folder_id, known).await;
        }
    }
}
