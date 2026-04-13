use crate::db::{bytes_to_embedding, SearchResult};

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

pub fn search_embeddings(
    query_embedding: &[f32],
    all_embeddings: &[(i64, String, String, Vec<u8>)],
    limit: usize,
) -> Vec<SearchResult> {
    let mut scored: Vec<SearchResult> = all_embeddings
        .iter()
        .map(|(id, file_path, file_name, embedding_bytes)| {
            let embedding = bytes_to_embedding(embedding_bytes);
            let similarity = cosine_similarity(query_embedding, &embedding) as f64;
            SearchResult {
                id: *id,
                file_path: file_path.clone(),
                file_name: file_name.clone(),
                similarity,
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::embedding_to_bytes;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_search_returns_top_results() {
        let query = vec![1.0, 0.0, 0.0];
        let embeddings = vec![
            (
                1i64,
                "/a.jpg".into(),
                "a.jpg".into(),
                embedding_to_bytes(&[0.9, 0.1, 0.0]),
            ),
            (
                2i64,
                "/b.jpg".into(),
                "b.jpg".into(),
                embedding_to_bytes(&[0.0, 1.0, 0.0]),
            ),
            (
                3i64,
                "/c.jpg".into(),
                "c.jpg".into(),
                embedding_to_bytes(&[0.8, 0.2, 0.0]),
            ),
        ];
        let results = search_embeddings(&query, &embeddings, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, 1); // 0.9 similarity
        assert_eq!(results[1].id, 3); // 0.8 similarity
    }
}
