use anyhow::Result;
use sqlx::{Row, SqlitePool};

pub async fn upsert_vector(pool: &SqlitePool, face_id: i64, embedding: &[f32]) -> Result<()> {
    let bytes: Vec<u8> = embedding.iter().flat_map(|v| v.to_le_bytes()).collect();
    sqlx::query("INSERT OR REPLACE INTO face_vectors(rowid, embedding) VALUES (?, ?)")
        .bind(face_id)
        .bind(&bytes)
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn delete_vector(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM face_vectors WHERE rowid = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// k nearest neighbors of `face_id` by cosine similarity, ascending distance.
/// Excludes `face_id` itself. Returns at most k results.
pub async fn knn(pool: &SqlitePool, face_id: i64, k: usize) -> Result<Vec<(i64, f32)>> {
    let query_bytes: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT embedding FROM face_vectors WHERE rowid = ?")
            .bind(face_id)
            .fetch_optional(pool)
            .await?;

    let Some(qb) = query_bytes else {
        return Ok(vec![]);
    };

    // Request k+1 to compensate for filtering out the query face itself.
    let rows = sqlx::query(
        "SELECT rowid, distance FROM face_vectors \
         WHERE embedding MATCH ? AND k = ? \
         ORDER BY distance",
    )
    .bind(&qb)
    .bind(k.saturating_add(1).min(i64::MAX as usize) as i64)
    .fetch_all(pool)
    .await?;

    let results: Result<Vec<_>> = rows
        .into_iter()
        .map(|r| {
            let id: i64 = r.try_get("rowid")?;
            let dist: f32 = r.try_get("distance")?;
            Ok((id, dist))
        })
        .collect();
    let results = results?;

    Ok(results
        .into_iter()
        .filter(|(id, _)| *id != face_id)
        .take(k)
        .collect())
}

/// Convert sqlite-vec L2 distance to cosine similarity (valid for L2-normalized unit vectors).
/// cos_sim = 1 - d² / 2
pub fn l2_dist_to_cosine_sim(l2_dist: f32) -> f32 {
    1.0 - (l2_dist * l2_dist) / 2.0
}

/// k nearest neighbors of `face_id` by cosine similarity, descending.
/// Excludes `face_id` itself. Returns at most k results.
pub async fn knn_cosine_sim(pool: &SqlitePool, face_id: i64, k: usize) -> Result<Vec<(i64, f32)>> {
    let mut sims: Vec<(i64, f32)> = knn(pool, face_id, k)
        .await?
        .into_iter()
        .map(|(id, dist)| (id, l2_dist_to_cosine_sim(dist)))
        .collect();
    sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(sims)
}

#[allow(dead_code)]
pub async fn get_all_face_vectors(pool: &SqlitePool) -> Result<Vec<(i64, Vec<f32>)>> {
    let rows = sqlx::query("SELECT rowid, embedding FROM face_vectors")
        .fetch_all(pool)
        .await?;

    rows.into_iter()
        .map(|r| {
            let id: i64 = r.get("rowid");
            let bytes: Vec<u8> = r.get("embedding");
            let embedding = crate::search::math::bytes_to_f32_vec(&bytes)?;
            Ok((id, embedding))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_pool(dim: usize) -> SqlitePool {
        crate::db::ensure_sqlite_vec_registered();
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(&format!(
            "CREATE VIRTUAL TABLE face_vectors USING vec0(embedding float[{}])",
            dim
        ))
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn knn_returns_empty_for_unknown_face() {
        let pool = make_pool(3).await;
        let result = knn(&pool, 999, 5).await.unwrap();
        assert!(result.is_empty(), "unknown face should return no neighbors");
    }

    #[tokio::test]
    async fn knn_returns_correct_ordering() {
        // A=[1,0,0], B=[0.9,0.1,0], C=[0,0,1]
        // Querying from A: B is much closer than C (cosine similarity ~ 0.993 vs 0.0)
        let pool = make_pool(3).await;
        upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap(); // face A
        upsert_vector(&pool, 2, &[0.9, 0.1, 0.0]).await.unwrap(); // face B — very close to A
        upsert_vector(&pool, 3, &[0.0, 0.0, 1.0]).await.unwrap(); // face C — orthogonal to A

        let neighbors = knn(&pool, 1, 2).await.unwrap();
        assert_eq!(neighbors.len(), 2, "should return exactly k=2 neighbors");
        assert_eq!(neighbors[0].0, 2, "B should be closest to A");
        assert_eq!(neighbors[1].0, 3, "C should be second-closest");
        assert!(
            neighbors[0].1 < neighbors[1].1,
            "distances should be ascending"
        );
    }

    #[tokio::test]
    async fn knn_excludes_self() {
        let pool = make_pool(3).await;
        upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap();
        upsert_vector(&pool, 2, &[0.9, 0.1, 0.0]).await.unwrap();

        let neighbors = knn(&pool, 1, 5).await.unwrap();
        let self_included = neighbors.iter().any(|(id, _)| *id == 1);
        assert!(!self_included, "knn must not include the query face itself");
    }

    #[tokio::test]
    async fn get_all_face_vectors_returns_seeded_data() {
        let pool = make_pool(3).await;
        upsert_vector(&pool, 10, &[1.0, 0.0, 0.0]).await.unwrap();
        upsert_vector(&pool, 20, &[0.0, 1.0, 0.0]).await.unwrap();

        let all = get_all_face_vectors(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        let ids: Vec<i64> = all.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
    }

    #[tokio::test]
    async fn delete_vector_removes_entry() {
        let pool = make_pool(3).await;
        upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap();
        delete_vector(&pool, 1).await.unwrap();
        let all = get_all_face_vectors(&pool).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn knn_cosine_sim_returns_similarity_descending() {
        let pool = make_pool(3).await;
        // A=[1,0,0], B=[0.9,0.44,0] (close to A), C=[0,0,1] (orthogonal)
        upsert_vector(&pool, 1, &[1.0, 0.0, 0.0]).await.unwrap();
        upsert_vector(&pool, 2, &[0.9, 0.44, 0.0]).await.unwrap();
        upsert_vector(&pool, 3, &[0.0, 0.0, 1.0]).await.unwrap();

        let sims = knn_cosine_sim(&pool, 1, 2).await.unwrap();
        assert_eq!(sims.len(), 2, "should return k=2 results");
        assert_eq!(sims[0].0, 2, "B should be most similar to A");
        assert!(sims[0].1 > sims[1].1, "similarities must be descending");
        assert!(sims[0].1 > 0.5, "B-A cosine similarity should be > 0.5");
        assert!(sims[1].1 < 0.2, "C-A cosine similarity should be near 0");
    }

    #[tokio::test]
    async fn l2_dist_to_cosine_sim_unit_vector_identity() {
        // For identical unit vectors: L2 dist = 0 → cosine sim = 1.0
        assert!((l2_dist_to_cosine_sim(0.0) - 1.0).abs() < 1e-6);
        // For orthogonal unit vectors: L2 dist = sqrt(2) → cosine sim = 0.0
        assert!((l2_dist_to_cosine_sim(2.0f32.sqrt()) - 0.0).abs() < 0.01);
        // For opposite unit vectors: L2 dist = 2.0 → cosine sim = -1.0
        assert!((l2_dist_to_cosine_sim(2.0) - (-1.0)).abs() < 1e-6);
    }
}
