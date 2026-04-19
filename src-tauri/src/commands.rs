use base64::Engine;
use sha2::{Sha256, Digest};
use std::collections::HashSet;
use tauri::Emitter;

use crate::{
    db,
    models::{ProcessingStatus, FolderWithCount, Image, SearchResult, SearchQuery, Subject, Face, MergeSuggestion, NameSubjectResult},
    search, thumbnail, AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn add_folder(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<FolderWithCount, String> {
    let folder = state
        .indexer
        .add_folder(path.clone())
        .await
        .map_err(|e| e.to_string())?;
    state
        .indexer
        .clone()
        .spawn_folder_scan(std::path::PathBuf::from(&path), folder.id);
    Ok(folder)
}

#[tauri::command]
pub async fn remove_folder(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .indexer
        .remove_folder(id)
        .await
        .map_err(|e| e.to_string())
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
pub async fn search(
    query: SearchQuery,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let pool = &state.pool;

    match query {
        SearchQuery::Text { ref query } => {
            let matched_subjects = db::search_subjects_by_name(pool, query).await.unwrap_or_default();
            let subject_ids: Vec<i64> = matched_subjects.iter().map(|s| s.id).collect();
            let subject_image_ids: HashSet<i64> = db::get_image_ids_for_subjects(pool, &subject_ids).await.unwrap_or_default().into_iter().collect();

            let mut results = vec![];
            for image_id in &subject_image_ids {
                if let Ok(Some(img)) = db::get_image_by_id(pool, *image_id).await {
                    results.push(SearchResult {
                        image_id: *image_id,
                        path: img.path,
                        thumbnail_path: img.thumbnail_path,
                        score: 1.0,
                        date_taken: img.date_taken,
                        mtime: img.mtime,
                        semantic_analysis_done: img.semantic_analysis_done,
                        subject_analysis_done: img.subject_analysis_done,
                    });
                }
            }

            let cache_key = {
                let mut hasher = Sha256::new();
                hasher.update(query.as_bytes());
                format!("{:x}", hasher.finalize())
            };

            let query_embedding = if let Some(cached) = db::get_cached_embedding(pool, &cache_key, "text").await.unwrap_or(None) {
                crate::embedder::bytes_to_f32_vec(&cached).map_err(map_err)?
            } else {
                let emb = state.vision_engine.embed_text(query).map_err(map_err)?;
                let blob = crate::embedder::f32_slice_to_bytes(&emb);
                let _ = db::insert_cached_embedding(pool, &cache_key, "text", &blob).await;
                emb
            };

            if let Ok(scored) = search::search_images(&state.index, query_embedding, 50).await {
                if let Ok(rag_results) = search::build_search_results(pool, scored).await {
                    for res in rag_results {
                        if !subject_image_ids.contains(&res.image_id) {
                            results.push(res);
                        }
                    }
                }
            }

            let _ = db::delete_stale_cache_entries(pool).await;
            Ok(results)
        }

        SearchQuery::ImageId { image_id } => {
            let embedding_blob = db::get_image_embedding(pool, image_id)
                .await
                .map_err(map_err)?
                .ok_or_else(|| "Embedding not found for image — try indexing first".to_string())?;
            let embedding_f32 = crate::embedder::bytes_to_f32_vec(&embedding_blob)
                .map_err(map_err)?;

            let mut scored = search::search_images(&state.index, embedding_f32, 50)
                .await
                .map_err(map_err)?;
            scored.retain(|(id, _)| *id != image_id);

            search::build_search_results(pool, scored).await.map_err(map_err)
        }

        SearchQuery::ImageBytes { ref data, mime_type: _ } => {
            let raw_bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(map_err)?;

            let cache_key = {
                let mut hasher = Sha256::new();
                hasher.update(&raw_bytes);
                format!("{:x}", hasher.finalize())
            };

            let query_embedding = if let Some(cached) = db::get_cached_embedding(pool, &cache_key, "image").await.unwrap_or(None) {
                crate::embedder::bytes_to_f32_vec(&cached).map_err(map_err)?
            } else {
                let img = image::load_from_memory(&raw_bytes).map_err(map_err)?;
                let emb = state.vision_engine.embed_image(&img).map_err(map_err)?;
                let blob = crate::embedder::f32_slice_to_bytes(&emb);
                let _ = db::insert_cached_embedding(pool, &cache_key, "image", &blob).await;
                emb
            };

            let scored = search::search_images(&state.index, query_embedding, 50)
                .await
                .map_err(map_err)?;

            let _ = db::delete_stale_cache_entries(pool).await;
            search::build_search_results(pool, scored).await.map_err(map_err)
        }
    }
}

#[tauri::command]
pub async fn get_processing_status(
    state: tauri::State<'_, AppState>,
) -> Result<ProcessingStatus, String> {
    db::get_processing_counts(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_subjects(state: tauri::State<'_, AppState>) -> Result<Vec<Subject>, String> {
    db::list_all_subjects(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn name_subject(
    id: i64,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<NameSubjectResult, String> {
    let pool = &state.pool;

    let duplicate_subject_id = if let Some(ref n) = name {
        let trimmed = n.trim();
        if !trimmed.is_empty() {
            db::find_subject_by_name(pool, trimmed, id)
                .await
                .map_err(map_err)?
                .map(|s| s.id)
        } else {
            None
        }
    } else {
        None
    };

    db::update_subject_name(pool, id, name.as_deref())
        .await
        .map_err(map_err)?;

    Ok(NameSubjectResult {
        duplicate_subject_id,
    })
}

#[tauri::command]
pub async fn list_faces(subject_id: i64, state: tauri::State<'_, AppState>) -> Result<Vec<Face>, String> {
    db::list_faces_for_subject(&state.pool, subject_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_faces_for_image(image_id: i64, state: tauri::State<'_, AppState>) -> Result<Vec<Face>, String> {
    db::list_faces_for_image(&state.pool, image_id).await.map_err(map_err)
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
        score: 1.0,
        date_taken: img.date_taken,
        mtime: img.mtime,
        semantic_analysis_done: img.semantic_analysis_done,
        subject_analysis_done: img.subject_analysis_done,
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

#[tauri::command]
pub async fn recluster_faces(
    state: tauri::State<'_, AppState>,
) -> Result<crate::clustering::ReclusterResult, String> {
    crate::clustering::recluster_all(&state.pool)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_merge_suggestions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MergeSuggestion>, String> {
    db::get_merge_suggestions(&state.pool)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn merge_subjects(
    target_id: i64,
    source_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::merge_subjects(&state.pool, target_id, source_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn dismiss_merge_suggestion(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::dismiss_merge_suggestion(&state.pool, id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn assign_face_to_subject(
    face_id: i64,
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::assign_face_to_subject(&state.pool, face_id, subject_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn create_subject_for_face(
    face_id: i64,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::Subject, String> {
    db::create_subject_for_face(&state.pool, face_id, name.as_deref())
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn unassign_face(
    face_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let old_subject_id = db::get_face_subject_id(&state.pool, face_id)
        .await
        .map_err(map_err)?;
    db::unassign_face(&state.pool, face_id)
        .await
        .map_err(map_err)?;
    db::record_face_correction(&state.pool, face_id, old_subject_id, None)
        .await
        .map_err(map_err)?;
    let _ = db::auto_assign_missing_thumbnails(&state.pool).await;
    let _ = db::delete_subjects_with_no_faces(&state.pool).await;
    Ok(())
}
