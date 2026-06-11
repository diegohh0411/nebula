use base64::Engine;
use sha2::{Sha256, Digest};
use std::collections::HashSet;

use crate::{
    models::{SearchResult, SearchQuery, SubjectMatch},
    search, AppState,
};

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn search(
    query: SearchQuery,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let pool = &state.pool;

    match query {
        SearchQuery::Text { ref query } => {
            // 1. Tag-derived images, already ordered by tagged-subject count desc.
            let tag_image_ids = crate::tags::repo::get_tag_image_ids_ordered(pool, query).await.unwrap_or_default();

            // 2. Name-derived images (accent-insensitive), appended after tag matches.
            let matched = crate::tags::repo::search_subjects_matching(pool, query).await.unwrap_or_default();
            let subject_ids: Vec<i64> = matched.iter().map(|m| m.subject.id).collect();
            let name_image_ids = crate::tags::repo::get_image_ids_for_subjects(pool, &subject_ids).await.unwrap_or_default();

            let mut pinned_ids: Vec<i64> = Vec::new();
            let mut pinned_set: HashSet<i64> = HashSet::new();
            for id in tag_image_ids.into_iter().chain(name_image_ids.into_iter()) {
                if pinned_set.insert(id) {
                    pinned_ids.push(id);
                }
            }

            let mut results = vec![];
            for image_id in &pinned_ids {
                if let Ok(Some(img)) = crate::library::repo::get_image_by_id(pool, *image_id).await {
                    if img.deleted_at.is_some() {
                        continue;
                    }
                    results.push(SearchResult {
                        image_id: *image_id,
                        path: img.path,
                        thumbnail_path: img.thumbnail_path,
                        preview_path: img.preview_path,
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

            let query_embedding = if let Some(cached) = crate::search::repo::get_cached_embedding(pool, &cache_key, "text").await.unwrap_or(None) {
                crate::search::math::bytes_to_f32_vec(&cached).map_err(map_err)?
            } else {
                let model_id = crate::settings::repo::get_setting(pool, "embedding_model")
                    .await
                    .unwrap_or(None)
                    .unwrap_or_else(|| "diegohh/siglip2-base-patch16-224".to_string());
                let spec = crate::models::registry::ModelSpec::find_by_id(&model_id)
                    .unwrap_or(&crate::models::registry::SIGLIP_BASE);

                state.model_manager.ensure_ready(&app, spec).await.map_err(map_err)?;
                let emb = state.vision_engine.embed_text(&state.model_manager, query, spec).map_err(map_err)?;
                let blob = crate::search::math::f32_slice_to_bytes(&emb);
                let _ = crate::search::repo::insert_cached_embedding(pool, &cache_key, "text", &blob).await;
                emb
            };

            if let Ok(scored) = search::search_images(&state.index, query_embedding, 50).await {
                if let Ok(rag_results) = search::build_search_results(pool, scored).await {
                    for res in rag_results {
                        if !pinned_set.contains(&res.image_id) {
                            results.push(res);
                        }
                    }
                }
            }

            let _ = crate::search::repo::delete_stale_cache_entries(pool).await;
            Ok(results)
        }

        SearchQuery::ImageId { image_id } => {
            let embedding_blob = crate::search::repo::get_image_embedding(pool, image_id)
                .await
                .map_err(map_err)?
                .ok_or_else(|| "Embedding not found for image — try indexing first".to_string())?;
            let embedding_f32 = crate::search::math::bytes_to_f32_vec(&embedding_blob)
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

            let query_embedding = if let Some(cached) = crate::search::repo::get_cached_embedding(pool, &cache_key, "image").await.unwrap_or(None) {
                crate::search::math::bytes_to_f32_vec(&cached).map_err(map_err)?
            } else {
                let model_id = crate::settings::repo::get_setting(pool, "embedding_model")
                    .await
                    .unwrap_or(None)
                    .unwrap_or_else(|| "diegohh/siglip2-base-patch16-224".to_string());
                let spec = crate::models::registry::ModelSpec::find_by_id(&model_id)
                    .unwrap_or(&crate::models::registry::SIGLIP_BASE);

                let img = image::load_from_memory(&raw_bytes).map_err(map_err)?;
                state.model_manager.ensure_ready(&app, spec).await.map_err(map_err)?;
                let emb = state.vision_engine.embed_image(&state.model_manager, &img, spec).map_err(map_err)?;
                let blob = crate::search::math::f32_slice_to_bytes(&emb);
                let _ = crate::search::repo::insert_cached_embedding(pool, &cache_key, "image", &blob).await;
                emb
            };

            let scored = search::search_images(&state.index, query_embedding, 50)
                .await
                .map_err(map_err)?;

            let _ = crate::search::repo::delete_stale_cache_entries(pool).await;
            search::build_search_results(pool, scored).await.map_err(map_err)
        }
    }
}

#[tauri::command]
pub async fn get_processing_status(
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::PipelineStatsPayload, String> {
    let ema_bits = state.throughput_ema.load(std::sync::atomic::Ordering::Relaxed);
    let images_per_sec = f32::from_bits(ema_bits);

    crate::pipeline::queue::get_processing_counts(&state.pool).await
        .map(|s| crate::models::PipelineStatsPayload {
            total_pending: s.total_pending as u32,
            images_per_sec,
        })
        .map_err(map_err)
}

