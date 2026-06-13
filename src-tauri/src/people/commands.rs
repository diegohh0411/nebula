use crate::{
    media::thumbnail,
    models::{Face, MergeSuggestion, NameSubjectResult, SearchResult, Subject, SubjectMatch},
    people::repo,
    tags::repo as tags_repo,
    AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn list_subjects(state: tauri::State<'_, AppState>) -> Result<Vec<Subject>, String> {
    repo::list_all_subjects(&state.pool).await.map_err(map_err)
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
            repo::find_subject_by_name(pool, trimmed, id)
                .await
                .map_err(map_err)?
                .map(|s| s.id)
        } else {
            None
        }
    } else {
        None
    };

    repo::update_subject_name(pool, id, name.as_deref())
        .await
        .map_err(map_err)?;

    Ok(NameSubjectResult {
        duplicate_subject_id,
    })
}

#[tauri::command]
pub async fn list_faces(
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Face>, String> {
    repo::list_faces_for_subject(&state.pool, subject_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn list_faces_for_image(
    image_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Face>, String> {
    repo::list_faces_for_image(&state.pool, image_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_face_crop(
    face_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let pool = &state.pool;
    let data_dir = &state.data_dir;

    let face = repo::get_face_by_id(pool, face_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Face not found".to_string())?;

    let image = crate::library::repo::get_image_by_id(pool, face.image_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Image not found".to_string())?;

    let crop_path = thumbnail::face_crop_path_for(data_dir, face_id);
    if !crop_path.exists() {
        thumbnail::generate_face_crop(
            std::path::PathBuf::from(&image.path),
            crop_path.clone(),
            (face.bbox_x, face.bbox_y, face.bbox_w, face.bbox_h),
        )
        .await
        .map_err(map_err)?;
    }

    Ok(crop_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn set_subject_thumbnail(
    subject_id: i64,
    face_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::update_subject_thumbnail_face(&state.pool, subject_id, face_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_subject_photos(
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let images = repo::list_images_for_subject(&state.pool, subject_id)
        .await
        .map_err(map_err)?;
    Ok(images
        .into_iter()
        .map(|img| SearchResult {
            image_id: img.id,
            path: img.path,
            thumbnail_path: img.thumbnail_path,
            preview_path: img.preview_path,
            score: 1.0,
            date_taken: img.date_taken,
            mtime: img.mtime,
            semantic_analysis_done: img.semantic_analysis_done,
            subject_analysis_done: img.subject_analysis_done,
        })
        .collect())
}

#[tauri::command]
pub async fn get_subject_photos_with_faces(
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::models::SubjectPhotoFace>, String> {
    repo::list_faces_for_subject_with_images(&state.pool, subject_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn get_subject_detail(
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::SubjectDetail, String> {
    let mut detail = repo::get_subject_detail_with_counts(&state.pool, subject_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Subject not found".to_string())?;

    if detail.subject.thumbnail_face_id.is_none() {
        if let Ok(Some(face_id)) = repo::get_largest_face_for_subject(&state.pool, subject_id).await
        {
            let _ = repo::update_subject_thumbnail_face(&state.pool, subject_id, face_id).await;
            detail.subject.thumbnail_face_id = Some(face_id);
        }
    }

    Ok(detail)
}

#[tauri::command]
pub async fn get_merge_suggestions(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<MergeSuggestion>, String> {
    repo::get_merge_suggestions(&state.pool, limit)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn merge_subjects(
    target_id: i64,
    source_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::merge_subjects(&state.pool, target_id, source_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn dismiss_merge_suggestion(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::dismiss_merge_suggestion(&state.pool, id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn assign_face_to_subject(
    face_id: i64,
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let pool = &state.pool;
    if let Ok(existing) = repo::get_face_ids_for_subject(pool, subject_id).await {
        for existing_face in existing {
            let _ = repo::add_must_link(pool, face_id, existing_face, "manual_assign").await;
        }
    }
    repo::assign_face_to_subject(pool, face_id, subject_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn create_subject_for_face(
    face_id: i64,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::Subject, String> {
    repo::create_subject_for_face(&state.pool, face_id, name.as_deref())
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn unassign_face(face_id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let pool = &state.pool;
    if let Ok(Some(face)) = repo::get_face_by_id(pool, face_id).await {
        if let Some(subject_id) = face.subject_id {
            if let Ok(siblings) = repo::get_face_ids_for_subject(pool, subject_id).await {
                for sibling_id in siblings {
                    if sibling_id != face_id {
                        let _ = repo::add_cannot_link(pool, face_id, sibling_id, "removal").await;
                    }
                }
            }
        }
    }
    repo::unassign_face(pool, face_id).await.map_err(map_err)?;
    let _ = repo::auto_assign_missing_thumbnails(pool).await;
    let _ = repo::delete_subjects_with_no_faces(pool).await;
    Ok(())
}

#[tauri::command]
pub async fn search_subjects(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SubjectMatch>, String> {
    tags_repo::search_subjects_matching(&state.pool, &query)
        .await
        .map_err(map_err)
}
