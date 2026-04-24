use anyhow::Result;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::Semaphore;

use tauri::Emitter;

use crate::{db, models::{ProcessingProgressPayload, ImageUpdatedPayload}};

const CONCURRENT_WORKERS: usize = 3;

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

pub(crate) async fn emit_progress(pool: &SqlitePool, app: &AppHandle) {
    if let Ok(status) = db::get_processing_counts(pool).await {
        let _ = app.emit(
            "processing_progress",
            ProcessingProgressPayload {
                semantic_pending: status.semantic_pending,
                subject_pending: status.subject_pending,
                done: status.done,
            },
        );
    }
}

/// Process one image through the semantic (SigLIP embedding) pipeline.
async fn process_semantic_one(
    pool: &SqlitePool,
    app: &AppHandle,
    vision_engine: &crate::vision_engine::VisionEngine,
    model_manager: &crate::models::ModelManager,
    queue_id: i64,
    image_id: i64,
    attempts: i32,
    index: &crate::vector_index::IndexStore,
    spec: &crate::models::registry::ModelSpec,
) {
    let image = match db::get_image_by_id(pool, image_id).await {
        Ok(Some(img)) => img,
        _ => return,
    };

    let img_res = tokio::task::spawn_blocking({
        let path = image.path.clone();
        move || image::open(path)
    }).await;

    let embed_result = match img_res {
        Ok(Ok(dynamic_img)) => vision_engine.embed_image(model_manager, &dynamic_img, spec),
        Ok(Err(e)) => Err(anyhow::anyhow!("failed to open image: {}", e)),
        Err(e) => Err(anyhow::anyhow!("spawn_blocking panicked: {}", e)),
    };

    match embed_result {
        Ok(values) => {
            let blob = f32_slice_to_bytes(&values);
            if db::mark_semantic_analysis_done(pool, queue_id, image_id, &blob).await.is_ok() {
                index.write().unwrap().add(image_id, &values);
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
        }
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Semantic embedding failed for image {}: {}", image_id, err_str);
            #[cfg(debug_assertions)]
            {
                let mut src = e.source();
                while let Some(cause) = src {
                    eprintln!("  caused by: {}", cause);
                    src = cause.source();
                }
            }
            if db::mark_failed(pool, queue_id, attempts, &err_str).await.is_ok() {
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
        }
    }

    emit_progress(pool, app).await;
}

async fn process_subject_one(
    pool: &SqlitePool,
    app: &AppHandle,
    vision_engine: &crate::vision_engine::VisionEngine,
    model_manager: &crate::models::ModelManager,
    queue_id: i64,
    image_id: i64,
    attempts: i32,
    preset: &crate::models::registry::FaceIdPreset,
) {
    let image = match db::get_image_by_id(pool, image_id).await {
        Ok(Some(img)) => img,
        _ => return,
    };

    let analyzer = match vision_engine.get_face_analyzer(model_manager, preset).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Face analyzer unavailable for image {}: {}", image_id, e);
            if db::mark_failed(pool, queue_id, attempts, &e.to_string()).await.is_ok() {
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
            emit_progress(pool, app).await;
            return;
        }
    };

    let img_res = tokio::task::spawn_blocking({
        let path = image.path.clone();
        move || image::open(path)
    }).await;

    let open_result = match img_res {
        Ok(Ok(dynamic_img)) => Ok(dynamic_img),
        Ok(Err(e)) => Err(anyhow::anyhow!("failed to open image: {}", e)),
        Err(e) => Err(anyhow::anyhow!("spawn_blocking panicked: {}", e)),
    };

    match open_result {
        Ok(dynamic_img) => {
            let faces_result = crate::vision_engine::VisionEngine::analyze_faces(&analyzer, &dynamic_img, preset);

            match faces_result {
                Ok(faces) => {
                    for (bbox, face_emb, gender, age) in faces {
                        let face_blob = f32_slice_to_bytes(&face_emb);
                        let gender_str = format!("{:?}", gender);
                        let _ = db::insert_face(
                            pool,
                            image_id,
                            None,
                            (
                                bbox.x1 as f64,
                                bbox.y1 as f64,
                                (bbox.x2 - bbox.x1) as f64,
                                (bbox.y2 - bbox.y1) as f64,
                            ),
                            Some(&face_blob),
                            Some(&gender_str),
                            Some(age),
                        ).await;
                    }
                    if db::mark_subject_analysis_done(pool, queue_id, image_id).await.is_ok() {
                        let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    eprintln!("Face analysis failed for image {}: {}", image_id, err_str);
                    if db::mark_failed(pool, queue_id, attempts, &err_str).await.is_ok() {
                        let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
                    }
                }
            }
        }
        Err(e) => {
            let err_str = e.to_string();
            eprintln!("Subject analysis failed for image {}: {}", image_id, err_str);
            if db::mark_failed(pool, queue_id, attempts, &err_str).await.is_ok() {
                let _ = app.emit("image_updated", ImageUpdatedPayload { image_id });
            }
        }
    }

    emit_progress(pool, app).await;
}

/// Long-running background task for semantic (SigLIP) embedding pipeline.
pub async fn run_semantic_worker(
    pool: SqlitePool,
    app: AppHandle,
    vision_engine: Arc<crate::vision_engine::VisionEngine>,
    model_manager: Arc<crate::models::ModelManager>,
    index: crate::vector_index::IndexStore,
    data_dir: std::path::PathBuf,
) {
    let semaphore = Arc::new(Semaphore::new(CONCURRENT_WORKERS));

    loop {
        let batch = match db::get_queue_batch(&pool, "semantic", (CONCURRENT_WORKERS * 2) as i64).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[semantic-worker] Failed to fetch batch: {}", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        if batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        let model_id = db::get_setting(&pool, "embedding_model")
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| "diegohh/siglip2-base-patch16-224".to_string());
        
        let spec = crate::models::registry::ModelSpec::find_by_id(&model_id)
            .unwrap_or(&crate::models::registry::SIGLIP_BASE);

        // Ensure model is ready before processing batch
        if let Err(e) = model_manager.ensure_ready(&app, spec).await {
            eprintln!("[semantic-worker] Model not ready: {}", e);
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        let mut handles = vec![];
        for (queue_id, image_id, attempts) in batch {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let pool_c = pool.clone();
            let app_c = app.clone();
            let ve_c = Arc::clone(&vision_engine);
            let mm_c = Arc::clone(&model_manager);
            let index_c = Arc::clone(&index);
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                process_semantic_one(
                    &pool_c, &app_c, ve_c.as_ref(), mm_c.as_ref(),
                    queue_id, image_id, attempts,
                    &index_c,
                    spec,
                ).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        // Persist index snapshot after each batch (off async thread to avoid blocking runtime)
        let snap_path = data_dir.join("nebula.idx");
        let index_snap = Arc::clone(&index);
        tokio::task::spawn_blocking(move || {
            let guard = index_snap.read().unwrap();
            if let Err(e) = guard.save(&snap_path) {
                eprintln!("[semantic-worker] Failed to save index snapshot: {e}");
            }
        }).await.ok();
    }
}

pub async fn run_subject_worker(
    pool: SqlitePool,
    app: AppHandle,
    vision_engine: Arc<crate::vision_engine::VisionEngine>,
    model_manager: Arc<crate::models::ModelManager>,
) {
    let preset = &crate::models::registry::BUFFALO_S_PRESET;

    // Ensure model is ready before starting
    if let Err(e) = model_manager.ensure_ready(&app, preset.detector).await {
        eprintln!("[subject-worker] Detector model not ready: {}", e);
    }
    if let Err(e) = model_manager.ensure_ready(&app, preset.embedder).await {
        eprintln!("[subject-worker] Embedder model not ready: {}", e);
    }
    if let Err(e) = model_manager.ensure_ready(&app, preset.gender_age).await {
        eprintln!("[subject-worker] Gender/age model not ready: {}", e);
    }

    if let Err(e) = vision_engine.get_face_analyzer(&model_manager, preset).await {
        eprintln!("[subject-worker] Failed to initialize face analyzer: {}", e);
    }

    let semaphore = Arc::new(Semaphore::new(CONCURRENT_WORKERS));

    loop {
        let batch = match db::get_queue_batch(&pool, "subject", (CONCURRENT_WORKERS * 2) as i64).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[subject-worker] Failed to fetch batch: {}", e);
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
            let ve_c = Arc::clone(&vision_engine);
            let mm_c = Arc::clone(&model_manager);
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                process_subject_one(&pool_c, &app_c, ve_c.as_ref(), mm_c.as_ref(), queue_id, image_id, attempts, preset).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        if had_items {
            eprintln!("[subject-worker] Batch complete, running auto-recluster...");
            match crate::clustering::cluster_unassigned_faces(&pool).await {
                Ok(result) => {
                    eprintln!(
                        "[subject-worker] Recluster done: {} clusters, {} noise, {} merged, {} deleted",
                        result.clusters, result.noise, result.merged, result.deleted
                    );
                    let _ = app.emit("subjects_updated", ());
                }
                Err(e) => {
                    eprintln!("[subject-worker] Auto-recluster failed: {}", e);
                }
            }
        }
    }
}
