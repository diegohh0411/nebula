use rusqlite::{Connection, Result as SqlResult};
use crate::db::{get_all_embeddings, bytes_to_embedding, SearchResult};

pub fn cosine_similarity(v1: &[f32], v2: &[f32]) -> f32 {
    let dot_product: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
    let norm1: f32 = v1.iter().map(|a| a * a).sum::<f32>().sqrt();
    let norm2: f32 = v2.iter().map(|a| a * a).sum::<f32>().sqrt();
    
    if norm1 == 0.0 || norm2 == 0.0 {
        return 0.0;
    }
    
    dot_product / (norm1 * norm2)
}

pub fn search_images(
    conn: &Connection,
    query_embedding: &[f32],
    top_k: usize,
) -> SqlResult<Vec<SearchResult>> {
    let all_embeddings = get_all_embeddings(conn)?;
    let mut results: Vec<SearchResult> = all_embeddings
        .into_iter()
        .map(|(id, file_path, file_name, blob)| {
            let embedding = bytes_to_embedding(&blob);
            let similarity = cosine_similarity(query_embedding, &embedding);
            SearchResult {
                id,
                file_path,
                file_name,
                similarity: similarity as f64,
            }
        })
        .collect();

    // Sort by similarity descending
    results.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());
    
    // Take top K
    Ok(results.into_iter().take(top_k).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v1, &v2) - 1.0).abs() < 1e-6);

        let v3 = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&v1, &v3) - 0.0).abs() < 1e-6);

        let v4 = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v1, &v4) + 1.0).abs() < 1e-6);
    }
}
