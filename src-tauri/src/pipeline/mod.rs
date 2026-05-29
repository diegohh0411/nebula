pub mod decoded_image;
pub mod embed_actor;
pub mod face_actor;

pub use decoded_image::{load_decoded, DecodedImage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputePlacement {
    /// SigLIP runs on CPU (default, always available).
    Cpu,
    /// SigLIP offloaded to the iGPU via DirectML; CPU stays free for face work.
    Gpu,
}

#[derive(Clone, Debug)]
pub struct PipelineConfig {
    pub batch_size: usize,
    pub load_channel_depth: usize,
    pub infer_channel_depth: usize,
    pub placement: ComputePlacement,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            batch_size: 12,
            load_channel_depth: 24,
            infer_channel_depth: 24,
            placement: ComputePlacement::Cpu,
        }
    }
}

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

#[allow(clippy::too_many_arguments)]
pub async fn run_pipeline(
    pool: sqlx::SqlitePool,
    app: tauri::AppHandle,
    engine: Arc<crate::vision_engine::VisionEngine>,
    manager: Arc<crate::models::ModelManager>,
    index: crate::vector_index::IndexStore,
    data_dir: std::path::PathBuf,
    config: PipelineConfig,
) {
    use tauri::Emitter;

    let spec = &crate::models::registry::SIGLIP_BASE;
    let preset = &crate::models::registry::BUFFALO_S_PRESET;

    if let Err(e) = manager.ensure_ready(&app, spec).await {
        eprintln!("[pipeline] embed model not ready: {e}");
    }
    let analyzer = match engine.get_face_analyzer(&manager, preset).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[pipeline] face analyzer init failed: {e}");
            return;
        }
    };

    let embed_tx = embed_actor::spawn_embed_actor(
        engine.clone(), manager.clone(), spec, config.batch_size,
    );
    let face_tx = face_actor::spawn_face_actor(analyzer);

    loop {
        let batch = {
            let sem_batch = crate::db::get_queue_batch(&pool, "semantic", config.batch_size as i64).await;
            let sub_batch = crate::db::get_queue_batch(&pool, "subject", config.batch_size as i64).await;

            let mut combined = sem_batch.unwrap_or_default();
            combined.extend(sub_batch.unwrap_or_default());
            // deduplicate by image_id
            let mut seen = std::collections::HashSet::new();
            combined.retain(|(_, image_id, _)| seen.insert(*image_id));
            combined
        };

        if batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        // Stage 1: decode once
        let mut decoded = Vec::with_capacity(batch.len());
        for (queue_id, image_id, attempts) in batch {
            let image = match crate::db::get_image_by_id(&pool, image_id).await {
                Ok(Some(i)) => i,
                _ => continue,
            };
            let path = image.path.clone();
            let res = tokio::task::spawn_blocking(move || {
                decoded_image::load_decoded(image_id, std::path::Path::new(&path))
            })
            .await;
            match res {
                Ok(Ok(d)) => decoded.push((queue_id, image_id, attempts, d)),
                Ok(Err(e)) => {
                    let _ = crate::db::mark_failed(&pool, queue_id, attempts, &e.to_string()).await;
                }
                Err(e) => {
                    let _ = crate::db::mark_failed(&pool, queue_id, attempts, &e.to_string()).await;
                }
            }
        }

        // Stage 2 & 3: dispatch embed + face, write results
        for (queue_id, image_id, _attempts, d) in decoded {
            // Send to embed actor
            let (etx, erx) = oneshot::channel();
            let _ = embed_tx
                .send(embed_actor::EmbedRequest {
                    decoded: d.clone(),
                    reply: etx,
                })
                .await;
            // Send to face actor
            let (ftx, frx) = oneshot::channel();
            let _ = face_tx
                .send(face_actor::FaceRequest {
                    decoded: d.clone(),
                    reply: ftx,
                })
                .await;

            // Write embed result
            if let Ok(Ok(emb)) = erx.await {
                let blob = crate::embedder::f32_slice_to_bytes(&emb);
                if crate::db::mark_semantic_analysis_done(&pool, queue_id, image_id, &blob)
                    .await
                    .is_ok()
                {
                    index.write().unwrap().add(image_id, &emb);
                }
            }

            // Write face results
            if let Ok(Ok(faces)) = frx.await {
                let img_w = d.full.width() as f64;
                let img_h = d.full.height() as f64;
                for (bbox, face_emb) in faces {
                    let face_blob = crate::embedder::f32_slice_to_bytes(&face_emb);
                    // Convert absolute pixel coordinates to relative fractions
                    let rel_x = bbox.x1 as f64 / img_w;
                    let rel_y = bbox.y1 as f64 / img_h;
                    let rel_w = (bbox.x2 - bbox.x1) as f64 / img_w;
                    let rel_h = (bbox.y2 - bbox.y1) as f64 / img_h;
                    let _ = crate::db::insert_face(
                        &pool,
                        image_id,
                        None,
                        (rel_x, rel_y, rel_w, rel_h),
                        Some(&face_blob),
                    )
                    .await;
                }
                let _ =
                    crate::db::mark_subject_analysis_done(&pool, queue_id, image_id).await;
            }

            // Thumbnail from same buffer
            let thumb_path = crate::thumbnail::thumbnail_path_for(&data_dir, image_id);
            let thumb_path_str = thumb_path.to_string_lossy().to_string();
            let d_thumb = d.clone();
            let _ = tokio::task::spawn_blocking(move || {
                crate::thumbnail::write_thumbnail_from_image(d_thumb.full.as_ref(), &thumb_path)
            })
            .await;
            let _ = crate::db::update_thumbnail_path(&pool, image_id, &thumb_path_str).await;
            let _ = app.emit(
                "image_updated",
                crate::models::ImageUpdatedPayload { image_id },
            );
        }

        crate::embedder::emit_progress(&pool, &app).await;

        // Persist index snapshot
        let snap_path = data_dir.join("nebula.idx");
        let index_snap = Arc::clone(&index);
        tokio::task::spawn_blocking(move || {
            let guard = index_snap.read().unwrap();
            if let Err(e) = guard.save(&snap_path) {
                eprintln!("[pipeline] failed to save index snapshot: {e}");
            }
        })
        .await
        .ok();

        // Auto-recluster
        if let Ok(_result) = crate::clustering::cluster_unassigned_faces(&pool).await {
            let _ = app.emit("subjects_updated", ());
        }
    }
}
