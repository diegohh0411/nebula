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

async fn write_faces(
    pool: &sqlx::SqlitePool,
    image_id: i64,
    sub_qid: i64,
    img_w: f64,
    img_h: f64,
    faces: Vec<(face_id::detector::BoundingBox, Vec<f32>)>,
) {
    for (bbox, face_emb) in faces {
        let face_blob = crate::embedder::f32_slice_to_bytes(&face_emb);
        // Convert absolute pixel coordinates to relative fractions
        let rel_x = bbox.x1 as f64 / img_w;
        let rel_y = bbox.y1 as f64 / img_h;
        let rel_w = (bbox.x2 - bbox.x1) as f64 / img_w;
        let rel_h = (bbox.y2 - bbox.y1) as f64 / img_h;
        let _ = crate::db::insert_face(
            pool,
            image_id,
            None,
            (rel_x, rel_y, rel_w, rel_h),
            Some(&face_blob),
        )
        .await;
    }
    let _ = crate::db::mark_subject_analysis_done(pool, sub_qid, image_id).await;
}

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
        // Pull both queues
        let sem_batch = crate::db::get_queue_batch(&pool, "semantic", config.batch_size as i64).await.unwrap_or_default();
        let sub_batch = crate::db::get_queue_batch(&pool, "subject", config.batch_size as i64).await.unwrap_or_default();

        if sem_batch.is_empty() && sub_batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        // Merge by image_id, tracking separate queue_ids for each operation
        let mut image_work: std::collections::HashMap<i64, (Option<(i64, i32)>, Option<(i64, i32)>)> = std::collections::HashMap::new();
        for (qid, image_id, attempts) in sem_batch {
            image_work.entry(image_id).or_default().0 = Some((qid, attempts));
        }
        for (qid, image_id, attempts) in sub_batch {
            image_work.entry(image_id).or_default().1 = Some((qid, attempts));
        }
        let batch: Vec<(i64, Option<(i64, i32)>, Option<(i64, i32)>)> = image_work
            .into_iter()
            .map(|(image_id, (sem, sub))| (image_id, sem, sub))
            .collect();

        // Stage 1: bounded-parallel decode
        let sem = Arc::new(tokio::sync::Semaphore::new(config.load_channel_depth));
        let mut handles = Vec::new();
        for (image_id, sem_entry, sub_entry) in batch {
            let pool_c = pool.clone();
            let permit = sem.clone().acquire_owned().await.expect("local semaphore closed unexpectedly");
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let image = crate::db::get_image_by_id(&pool_c, image_id).await.ok().flatten()?;
                let path = image.path.clone();
                let d = tokio::task::spawn_blocking(move || {
                    decoded_image::load_decoded(image_id, std::path::Path::new(&path))
                })
                .await
                .ok()?
                .ok()?;
                Some((image_id, sem_entry, sub_entry, d))
            }));
        }
        let mut decoded = Vec::new();
        for h in handles {
            if let Ok(Some(x)) = h.await {
                decoded.push(x);
            }
        }

        // Stage 2 & 3: dispatch embed + face, write results
        for (image_id, sem_entry, sub_entry, d) in decoded {
            let img_w = d.full.width() as f64;
            let img_h = d.full.height() as f64;

            if config.placement == ComputePlacement::Cpu {
                // Serialize: embed first, then face (avoid thrashing CPU with both)
                if let Some((sem_qid, _)) = sem_entry {
                    let (etx, erx) = oneshot::channel();
                    let _ = embed_tx
                        .send(embed_actor::EmbedRequest {
                            decoded: d.clone(),
                            reply: etx,
                        })
                        .await;
                    if let Ok(Ok(emb)) = erx.await {
                        let blob = crate::embedder::f32_slice_to_bytes(&emb);
                        if crate::db::mark_semantic_analysis_done(&pool, sem_qid, image_id, &blob)
                            .await
                            .is_ok()
                        {
                            index.write().unwrap().add(image_id, &emb);
                        }
                    }
                }

                if let Some((sub_qid, _)) = sub_entry {
                    let (ftx, frx) = oneshot::channel();
                    let _ = face_tx
                        .send(face_actor::FaceRequest {
                            decoded: d.clone(),
                            reply: ftx,
                        })
                        .await;
                    if let Ok(Ok(faces)) = frx.await {
                        write_faces(&pool, image_id, sub_qid, img_w, img_h, faces).await;
                    }
                }
            } else {
                // Concurrent: embed on iGPU, face on CPU — dispatch both before awaiting
                let erx = if let Some((_, _)) = sem_entry {
                    let (etx, erx) = oneshot::channel();
                    let _ = embed_tx
                        .send(embed_actor::EmbedRequest {
                            decoded: d.clone(),
                            reply: etx,
                        })
                        .await;
                    Some(erx)
                } else {
                    None
                };
                let frx = if let Some((_, _)) = sub_entry {
                    let (ftx, frx) = oneshot::channel();
                    let _ = face_tx
                        .send(face_actor::FaceRequest {
                            decoded: d.clone(),
                            reply: ftx,
                        })
                        .await;
                    Some(frx)
                } else {
                    None
                };

                // Use tokio::join! for true concurrency between embed and face
                match (erx, frx) {
                    (Some(erx), Some(frx)) => {
                        let (emb_result, face_result) = tokio::join!(erx, frx);
                        if let Ok(Ok(emb)) = emb_result {
                            let blob = crate::embedder::f32_slice_to_bytes(&emb);
                            if let Some((sem_qid, _)) = sem_entry {
                                if crate::db::mark_semantic_analysis_done(&pool, sem_qid, image_id, &blob)
                                    .await
                                    .is_ok()
                                {
                                    index.write().unwrap().add(image_id, &emb);
                                }
                            }
                        }
                        if let Ok(Ok(faces)) = face_result {
                            if let Some((sub_qid, _)) = sub_entry {
                                write_faces(&pool, image_id, sub_qid, img_w, img_h, faces).await;
                            }
                        }
                    }
                    (Some(erx), None) => {
                        if let Ok(Ok(emb)) = erx.await {
                            let blob = crate::embedder::f32_slice_to_bytes(&emb);
                            if let Some((sem_qid, _)) = sem_entry {
                                if crate::db::mark_semantic_analysis_done(&pool, sem_qid, image_id, &blob)
                                    .await
                                    .is_ok()
                                {
                                    index.write().unwrap().add(image_id, &emb);
                                }
                            }
                        }
                    }
                    (None, Some(frx)) => {
                        if let Ok(Ok(faces)) = frx.await {
                            if let Some((sub_qid, _)) = sub_entry {
                                write_faces(&pool, image_id, sub_qid, img_w, img_h, faces).await;
                            }
                        }
                    }
                    (None, None) => {}
                }
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
