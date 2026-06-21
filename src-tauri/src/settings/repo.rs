use anyhow::Result;
use sqlx::{Row, SqlitePool};

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("value")))
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_then_get_round_trips_and_overwrites() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();

        set_setting(&pool, "clustering_dirty", "true").await.unwrap();
        assert_eq!(
            get_setting(&pool, "clustering_dirty").await.unwrap().as_deref(),
            Some("true")
        );

        set_setting(&pool, "clustering_dirty", "false").await.unwrap();
        assert_eq!(
            get_setting(&pool, "clustering_dirty").await.unwrap().as_deref(),
            Some("false"),
            "second write must overwrite"
        );
    }
}
