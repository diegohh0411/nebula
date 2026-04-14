use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::Client;
use serde::Deserialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::{Mutex, Semaphore};

use tauri::Emitter;

use crate::{db, models::{EmbedProgressPayload, ImageUpdatedPayload}};

const CONCURRENT_WORKERS: usize = 3;
const GEMINI_EMBED_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-2-preview:embedContent";

#[derive(Deserialize, Debug)]
struct EmbedResponse {
    embedding: EmbedValues,
}

#[derive(Deserialize, Debug)]
struct EmbedValues {
    values: Vec<f32>,
}

/// Encode a Vec<f32> to raw little-endian bytes for storage as BLOB.
pub fn f32_slice_to_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

/// Embed a text query using the Gemini API.
pub async fn embed_text(client: &Client, api_key: &str, text: &str) -> Result<Vec<f32>> {
    let body = serde_json::json!({
        "content": {
            "parts": [{ "text": text }]
        }
    });

    let resp = client
        .post(GEMINI_EMBED_URL)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<EmbedResponse>()
        .await?;

    Ok(resp.embedding.values)
}

/// Embed an image using the Gemini API.
async fn embed_image(client: &Client, api_key: &str, image_path: &str) -> Result<Vec<f32>> {
    let path = std::path::PathBuf::from(image_path);

    // Determine MIME type
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => "image/jpeg",
    };

    let bytes =
        tokio::task::spawn_blocking(move || std::fs::read(&path)).await??;
    let b64 = BASE64.encode(&bytes);

    let body = serde_json::json!({
        "content": {
            "parts": [{ "inlineData": { "mimeType": mime, "data": b64 } }]
        }
    });

    let resp = client
        .post(GEMINI_EMBED_URL)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<EmbedResponse>()
        .await?;

    Ok(resp.embedding.values)
}

async fn process_one(
    pool: &SqlitePool,
    app: &AppHandle,
    client: &Client,
    queue_id: i64,
    image_id: i64,
    attempts: i32,
    api_key: &str,
) {
    // Load image path from DB
    let image = match db::get_image_by_id(pool, image_id).await {
        Ok(Some(img)) => img,
        _ => return,
    };

    match embed_image(client, api_key, &image.path).await {
        Ok(values) => {
            let blob = f32_slice_to_bytes(&values);
            if db::mark_embedded(pool, image_id, &blob).await.is_ok() {
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
        }
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Embedding failed for image {}: {}", image_id, err_str);
            if db::mark_failed(pool, queue_id, attempts, &err_str).await.is_ok() {
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
        }
    }

    // Emit progress event
    if let Ok(status) = db::get_embed_counts(pool).await {
        let _ = app.emit(
            "embed_progress",
            EmbedProgressPayload {
                pending: status.pending,
                done: status.done,
            },
        );
    }
}

/// Long-running background task that processes the embedding queue.
pub async fn run_embedding_worker(
    pool: SqlitePool,
    app: AppHandle,
    api_key: Arc<Mutex<Option<String>>>,
) {
    let semaphore = Arc::new(Semaphore::new(CONCURRENT_WORKERS));
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();

    loop {
        // Check for API key
        let key = {
            let lock = api_key.lock().await;
            lock.clone()
        };
        let Some(key) = key else {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        };

        // Fetch a batch from the queue (up to CONCURRENT_WORKERS * 2)
        let batch = match db::get_queue_batch(&pool, (CONCURRENT_WORKERS * 2) as i64).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Failed to fetch queue batch: {}", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        if batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let mut handles = vec![];
        for (queue_id, image_id, attempts) in batch {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let pool_c = pool.clone();
            let app_c = app.clone();
            let client_c = client.clone();
            let key_c = key.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                process_one(&pool_c, &app_c, &client_c, queue_id, image_id, attempts, &key_c).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }
    }
}
