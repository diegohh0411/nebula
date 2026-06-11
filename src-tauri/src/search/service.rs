use anyhow::Result;
use sqlx::SqlitePool;

use crate::{db, models::SearchResult};

pub async fn search_images(
    index: &crate::search::vector_index::IndexStore,
    query_embedding: Vec<f32>,
    limit: usize,
) -> Result<Vec<(i64, f32)>> {
    let index = std::sync::Arc::clone(index);
    let results = tokio::task::spawn_blocking(move || {
        let guard = index.read().unwrap();
        let mut scored = guard.search(&query_embedding, limit);

        // GAP HEURISTIC (Elbow method): cut at the largest similarity drop >= 0.05
        if scored.len() > 1 {
            let mut max_gap = 0.0f32;
            let mut cut_index = scored.len();
            for i in 0..scored.len() - 1 {
                let gap = scored[i].1 - scored[i + 1].1;
                if gap > max_gap && gap >= 0.05 {
                    max_gap = gap;
                    cut_index = i + 1;
                }
            }
            scored.truncate(cut_index);
        }
        scored
    })
    .await?;
    Ok(results)
}

pub async fn build_search_results(
    pool: &SqlitePool,
    scored: Vec<(i64, f32)>,
) -> Result<Vec<SearchResult>> {
    let mut results = Vec::with_capacity(scored.len());
    for (image_id, score) in scored {
        if let Ok(Some(img)) = db::get_image_by_id(pool, image_id).await {
            if img.deleted_at.is_some() {
                continue; // guard against tombstone/soft-delete race
            }
            results.push(SearchResult {
                image_id,
                path: img.path,
                thumbnail_path: img.thumbnail_path,
                preview_path: img.preview_path,
                score,
                date_taken: img.date_taken,
                mtime: img.mtime,
                semantic_analysis_done: img.semantic_analysis_done,
                subject_analysis_done: img.subject_analysis_done,
            });
        }
    }
    Ok(results)
}
