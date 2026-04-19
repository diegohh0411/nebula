use anyhow::Result;
use sqlx::SqlitePool;

use crate::{db, models::SearchResult};

fn blob_to_f32(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Search all embedded images for the top `limit` most similar to the query embedding.
pub async fn search_images(
    pool: &SqlitePool,
    query_embedding: Vec<f32>,
    limit: usize,
) -> Result<Vec<(i64, f32)>> {
    // Load all embeddings — runs in blocking thread since it can be large
    let all_embeddings = db::get_all_embeddings(pool).await?;

    let results = tokio::task::spawn_blocking(move || {
        let mut scored: Vec<(i64, f32)> = all_embeddings
            .iter()
            .map(|(id, blob)| {
                let embedding = blob_to_f32(blob);
                let score = cosine_similarity(&query_embedding, &embedding);
                (*id, score)
            })
            .collect();

        // Sort descending by score
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        // GAP HEURISTIC (Elbow method):
        // Find the largest drop in similarity between consecutive results.
        // If that drop is significant (>= 0.05), we cut off at that point
        // to return a more "natural" set of matches.
        if scored.len() > 1 {
            let mut max_gap = 0.0;
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

/// Build full SearchResult structs from a list of (image_id, score) pairs.
pub async fn build_search_results(
    pool: &SqlitePool,
    scored: Vec<(i64, f32)>,
) -> Result<Vec<SearchResult>> {
    let mut results = Vec::with_capacity(scored.len());
    for (image_id, score) in scored {
        if let Ok(Some(img)) = db::get_image_by_id(pool, image_id).await {
            results.push(SearchResult {
                image_id,
                path: img.path,
                thumbnail_path: img.thumbnail_path,
                score,
                date_taken: img.date_taken,
                date_file: img.date_file,
                semantic_analysis_done: img.semantic_analysis_done,
                subject_analysis_done: img.subject_analysis_done,
            });
        }
    }
    Ok(results)
}
