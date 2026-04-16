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

use crate::{db, models::{EmbedProgressPayload, ImageUpdatedPayload}, face_detector::Detector};

const CONCURRENT_WORKERS: usize = 3;
const CLUSTERING_THRESHOLD: f32 = 0.4;
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

/// Decode raw little-endian bytes back to a Vec<f32>.
pub fn bytes_to_f32_vec(bytes: &[u8]) -> Result<Vec<f32>> {
    anyhow::ensure!(
        bytes.len() % 4 == 0,
        "invalid embedding byte length: expected a multiple of 4, got {}",
        bytes.len()
    );

    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| {
            f32::from_le_bytes(
                chunk
                    .try_into()
                    .expect("chunks_exact(4) must yield chunks of exactly 4 bytes"),
            )
        })
        .collect())
}

pub fn cosine_similarity(v1: &[f32], v2: &[f32]) -> f32 {
    let dot_product: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
    let norm1: f32 = v1.iter().map(|a| a * a).sum::<f32>().sqrt();
    let norm2: f32 = v2.iter().map(|a| a * a).sum::<f32>().sqrt();
    if norm1 == 0.0 || norm2 == 0.0 {
        return 0.0;
    }
    dot_product / (norm1 * norm2)
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

pub async fn embed_image_bytes(client: &Client, api_key: &str, bytes: Vec<u8>, mime: &str) -> Result<Vec<f32>> {
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
    detector: Option<&Detector>,
    clustering_lock: &tokio::sync::Mutex<()>,
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

            // --- Face Detection ---
            if let Some(detector) = detector {
                let img_res = tokio::task::spawn_blocking({
                    let path = image.path.clone();
                    move || image::open(path)
                }).await;

                if let Ok(Ok(dynamic_img)) = img_res {
                    if let Ok(faces) = detector.analyze(&dynamic_img) {
                        for face_analysis in faces {
                            let bbox = face_analysis.detection.bbox;
                            
                            // Use ArcFace embedding from face_id locally
                            let face_emb = face_analysis.embedding;
                            
                            let (subject_id, face_id) = {
                                let _guard = clustering_lock.lock().await;

                                let existing_subjects = db::get_subject_embeddings(pool).await.unwrap_or_default();
                                let mut best_subject_id = None;
                                let mut best_score = 0.0;

                                for (sid, emb_blob) in existing_subjects {
                                    if let Ok(emb) = bytes_to_f32_vec(&emb_blob) {
                                        let score = cosine_similarity(&face_emb, &emb);
                                        if score > best_score {
                                            best_score = score;
                                            if score > CLUSTERING_THRESHOLD {
                                                best_subject_id = Some(sid);
                                            }
                                        }
                                    }
                                }

                                if best_score > 0.0 {
                                    eprintln!(
                                        "[face-cluster] image_id={} best_score={:.4} threshold={} matched={}",
                                        image_id, best_score, CLUSTERING_THRESHOLD, best_subject_id.is_some()
                                    );
                                }

                                let subject_id = if let Some(sid) = best_subject_id {
                                    Some(sid)
                                } else {
                                    db::insert_subject(pool, None, "person").await.ok()
                                };

                                let face_blob = f32_slice_to_bytes(&face_emb);
                                let face_id = db::insert_face(
                                    pool,
                                    image_id,
                                    subject_id,
                                    (
                                        bbox.x1 as f64,
                                        bbox.y1 as f64,
                                        (bbox.x2 - bbox.x1) as f64,
                                        (bbox.y2 - bbox.y1) as f64,
                                    ),
                                    Some(&face_blob),
                                ).await.ok();

                                (subject_id, face_id)
                            };

                            if let (Some(sid), Some(fid)) = (subject_id, face_id) {
                                if let Ok(subjects) = db::list_all_subjects(pool).await {
                                    if let Some(sub) = subjects.iter().find(|s| s.id == sid) {
                                        if sub.thumbnail_face_id.is_none() {
                                            let _ = db::update_subject_thumbnail_face(pool, sid, fid).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
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
    let clustering_lock = Arc::new(tokio::sync::Mutex::new(()));
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default();

    // Initialize face detector
    let detector = match Detector::new().await {
        Ok(d) => Some(Arc::new(d)),
        Err(e) => {
            eprintln!("Failed to initialize face detector: {}", e);
            None
        }
    };

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

        let had_items = !batch.is_empty();
        let mut handles = vec![];
        for (queue_id, image_id, attempts) in batch {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let pool_c = pool.clone();
            let app_c = app.clone();
            let client_c = client.clone();
            let key_c = key.clone();
            let detector_c = detector.clone();
            let lock_c = clustering_lock.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                process_one(&pool_c, &app_c, &client_c, detector_c.as_deref(), &lock_c, queue_id, image_id, attempts, &key_c).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        if had_items {
            eprintln!("[face-cluster] Batch complete, running auto-recluster...");
            match crate::clustering::recluster_all(&pool).await {
                Ok(result) => {
                    eprintln!(
                        "[face-cluster] Recluster done: {} clusters, {} noise, {} merged, {} deleted",
                        result.clusters, result.noise, result.merged, result.deleted
                    );
                    let _ = app.emit("subjects_updated", ());
                }
                Err(e) => {
                    eprintln!("[face-cluster] Auto-recluster failed: {}", e);
                }
            }
        }
    }
}
