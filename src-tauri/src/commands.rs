use reqwest::Client;
use tauri::{AppHandle, Emitter};

use crate::{
    config, db,
    models::{EmbedStatus, FolderWithCount, Image, SearchResult, Subject, Face},
    search, thumbnail, watcher, AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn add_folder(
    path: String,
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<FolderWithCount, String> {
    let pool = &state.pool;
    let data_dir = &state.data_dir;

    // Insert folder into DB
    let folder_id = db::insert_folder(pool, &path).await.map_err(map_err)?;

    // Scan folder for existing images
    let folder_path = std::path::PathBuf::from(&path);
    watcher::scan_folder(pool, &app, folder_id, &folder_path, data_dir)
        .await
        .map_err(map_err)?;

    // Register the filesystem watcher
    {
        let mut w = state.watcher.lock().await;
        if let Err(e) = w.watch(folder_path, folder_id) {
            eprintln!("Failed to register watcher for folder {folder_id}: {e}");
        }
    }

    // Return updated folder with count
    let folders = db::list_folders_with_counts(pool).await.map_err(map_err)?;
    folders
        .into_iter()
        .find(|f| f.id == folder_id)
        .ok_or_else(|| "Folder not found after insert".to_string())
}

#[tauri::command]
pub async fn remove_folder(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let pool = &state.pool;

    // Find the folder path before deleting
    let folders = db::list_folders_with_counts(pool).await.map_err(map_err)?;
    if let Some(folder) = folders.iter().find(|f| f.id == id) {
        let path = std::path::PathBuf::from(&folder.path);
        let mut w = state.watcher.lock().await;
        if let Err(e) = w.unwatch(&path) {
            eprintln!("Failed to unregister watcher for path {}: {e}", path.display());
        }
    }

    db::delete_folder(pool, id).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_folders(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FolderWithCount>, String> {
    db::list_folders_with_counts(&state.pool)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn list_images(
    folder_id: Option<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Image>, String> {
    db::list_images(&state.pool, folder_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn search_images(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let pool = &state.pool;

    // 1. Fuzzy Subject Search
    let matched_subjects = db::search_subjects_by_name(pool, &query).await.unwrap_or_default();
    let subject_ids: Vec<i64> = matched_subjects.iter().map(|s| s.id).collect();
    let subject_image_ids = db::get_image_ids_for_subjects(pool, &subject_ids).await.unwrap_or_default();

    let mut final_results = vec![];
    
    // Map explicit subject matches to SearchResult with a high score (1.0)
    for image_id in &subject_image_ids {
        if let Ok(Some(img)) = db::get_image_by_id(pool, *image_id).await {
            final_results.push(SearchResult {
                image_id: *image_id,
                path: img.path,
                thumbnail_path: img.thumbnail_path,
                score: 1.0,
                date_taken: img.date_taken,
                date_file: img.date_file,
                embed_status: img.embed_status,
            });
        }
    }

    // 2. RAG Search Fallback
    let api_key = {
        let lock = state.api_key.lock().await;
        lock.clone()
    };

    if let Some(api_key) = api_key {
        let client = Client::new();
        if let Ok(query_embedding) = crate::embedder::embed_text(&client, &api_key, &query).await {
            if let Ok(scored) = search::search_images(pool, query_embedding, 50).await {
                if let Ok(rag_results) = search::build_search_results(pool, scored).await {
                    for res in rag_results {
                        if !subject_image_ids.contains(&res.image_id) {
                            final_results.push(res);
                        }
                    }
                }
            }
        }
    }

    Ok(final_results)
}

#[tauri::command]
pub async fn search_similar_images(
    image_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let pool = &state.pool;
    let embedding = db::get_image_embedding(pool, image_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Embedding not found for image — try indexing first".to_string())?;

    let embedding_f32 = crate::embedder::bytes_to_f32_vec(&embedding)
        .map_err(|e| e.to_string())?;

    let scored = search::search_images(pool, embedding_f32, 50)
        .await
        .map_err(map_err)?;

    // Exclude the source image from its own results
    let filtered_scored = scored
        .into_iter()
        .filter(|(id, _)| *id != image_id)
        .collect();

    search::build_search_results(pool, filtered_scored)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_embed_status(
    state: tauri::State<'_, AppState>,
) -> Result<EmbedStatus, String> {
    db::get_embed_counts(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn set_api_key(
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    config::write_api_key(&state.data_dir, &key).map_err(map_err)?;
    let mut lock = state.api_key.lock().await;
    *lock = Some(key);
    Ok(())
}

#[tauri::command]
pub async fn get_api_key(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let lock = state.api_key.lock().await;
    Ok(lock.clone())
}

#[tauri::command]
pub async fn regenerate_all_thumbnails(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // 1. Clear database paths
    sqlx::query("UPDATE images SET thumbnail_path = NULL")
        .execute(&state.pool)
        .await
        .map_err(map_err)?;

    // 2. Clear disk cache
    let cache_dir = thumbnail::thumbnail_cache_dir(&state.data_dir);
    if cache_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await {
            eprintln!("Failed to remove thumbnail cache directory {:?}: {}", cache_dir, e);
        }
        if let Err(e) = tokio::fs::create_dir_all(&cache_dir).await {
            eprintln!("Failed to create thumbnail cache directory {:?}: {}", cache_dir, e);
        }
    }

    // 3. Trigger background generation for all images
    let pool = state.pool.clone();
    let data_dir = state.data_dir.clone();
    let images = db::list_images(&pool, None).await.map_err(map_err)?;

    tokio::spawn(async move {
        for image in images {
            let thumb_path = thumbnail::thumbnail_path_for(&data_dir, image.id);
            let thumb_str = thumb_path.to_string_lossy().to_string();
            let src = std::path::PathBuf::from(&image.path);
            
            if let Ok(()) = thumbnail::generate_thumbnail(src, thumb_path).await {
                if db::update_thumbnail_path(&pool, image.id, &thumb_str).await.is_ok() {
                    let _ = app.emit(
                        "image_updated",
                        crate::models::ImageUpdatedPayload { image_id: image.id },
                    );
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn list_subjects(state: tauri::State<'_, AppState>) -> Result<Vec<Subject>, String> {
    db::list_all_subjects(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn name_subject(id: i64, name: Option<String>, state: tauri::State<'_, AppState>) -> Result<(), String> {
    db::update_subject_name(&state.pool, id, name.as_deref()).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_faces(subject_id: i64, state: tauri::State<'_, AppState>) -> Result<Vec<Face>, String> {
    db::list_faces_for_subject(&state.pool, subject_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_faces_for_image(image_id: i64, state: tauri::State<'_, AppState>) -> Result<Vec<Face>, String> {
    db::list_faces_for_image(&state.pool, image_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_face_crop(face_id: i64, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let pool = &state.pool;
    let data_dir = &state.data_dir;

    let face = db::get_face_by_id(pool, face_id).await.map_err(map_err)?
        .ok_or_else(|| "Face not found".to_string())?;

    let image = db::get_image_by_id(pool, face.image_id).await.map_err(map_err)?
        .ok_or_else(|| "Image not found".to_string())?;

    let crop_path = thumbnail::face_crop_path_for(data_dir, face_id);
    if !crop_path.exists() {
        thumbnail::generate_face_crop(
            std::path::PathBuf::from(&image.path),
            crop_path.clone(),
            (face.bbox_x, face.bbox_y, face.bbox_w, face.bbox_h)
        ).await.map_err(map_err)?;
    }

    Ok(crop_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn set_subject_thumbnail(subject_id: i64, face_id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    db::update_subject_thumbnail_face(&state.pool, subject_id, face_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn get_subject_photos(subject_id: i64, state: tauri::State<'_, AppState>) -> Result<Vec<SearchResult>, String> {
    let images = db::list_images_for_subject(&state.pool, subject_id).await.map_err(map_err)?;
    Ok(images.into_iter().map(|img| SearchResult {
        image_id: img.id,
        path: img.path,
        thumbnail_path: img.thumbnail_path,
        score: 1.0, // Subjects are perfect matches
        date_taken: img.date_taken,
        date_file: img.date_file,
        embed_status: img.embed_status,
    }).collect())
}

#[tauri::command]
pub async fn get_subject_detail(subject_id: i64, state: tauri::State<'_, AppState>) -> Result<crate::models::SubjectDetail, String> {
    let mut detail = db::get_subject_detail_with_counts(&state.pool, subject_id).await.map_err(map_err)?
        .ok_or_else(|| "Subject not found".to_string())?;

    // Auto-select thumbnail if not set
    if detail.subject.thumbnail_face_id.is_none() {
        if let Ok(Some(face_id)) = db::get_largest_face_for_subject(&state.pool, subject_id).await {
            let _ = db::update_subject_thumbnail_face(&state.pool, subject_id, face_id).await;
            detail.subject.thumbnail_face_id = Some(face_id);
        }
    }

    Ok(detail)
}
