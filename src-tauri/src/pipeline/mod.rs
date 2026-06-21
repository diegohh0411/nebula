pub mod decoded_image;
pub mod embed_actor;
pub mod face_actor;
pub mod queue;
pub mod sampler;
pub mod throughput;

pub use decoded_image::DecodedImage;

/// Queue slot for a single pipeline operation: (queue_id, attempt_count).
type WorkSlot = Option<(i64, i32)>;

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

use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::Duration;

async fn save_faces(
    pool: &sqlx::SqlitePool,
    image_id: i64,
    sub_qid: i64,
    sub_attempts: i32,
    faces: Vec<face_actor::FaceResult>,
) {
    let mut all_ok = true;
    for (detection, face_emb, sharp) in faces {
        let bbox = detection.bbox;
        let rel_x = bbox.x1 as f64;
        let rel_y = bbox.y1 as f64;
        let rel_w = (bbox.x2 - bbox.x1) as f64;
        let rel_h = (bbox.y2 - bbox.y1) as f64;

        let frontality = crate::people::face_quality::frontality(detection.landmarks.as_deref());
        let quality = crate::people::face_quality::composite(detection.score, frontality, sharp);

        match crate::people::repo::insert_face(
            pool,
            image_id,
            None,
            (rel_x, rel_y, rel_w, rel_h),
            Some(detection.score as f64),
            Some(quality as f64),
        )
        .await
        {
            Ok(face_id) => {
                if let Err(e) =
                    crate::people::face_store::upsert_vector(pool, face_id, &face_emb).await
                {
                    error!("[pipeline] upsert_vector failed for face {face_id}: {e}");
                    all_ok = false;
                }
            }
            Err(e) => {
                error!("[pipeline] insert_face failed for image {image_id}: {e}");
                all_ok = false;
            }
        }
    }
    if all_ok {
        let _ = crate::pipeline::queue::mark_subject_analysis_done(pool, sub_qid, image_id).await;
    } else {
        let _ = crate::pipeline::queue::mark_failed(
            pool,
            sub_qid,
            sub_attempts,
            "one or more face inserts failed",
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_pipeline(
    pool: sqlx::SqlitePool,
    app: tauri::AppHandle,
    engine: Arc<crate::vision::engine::VisionEngine>,
    manager: Arc<crate::models::ModelManager>,
    index: crate::search::vector_index::IndexStore,
    data_dir: std::path::PathBuf,
    config: PipelineConfig,
    requested_spec: &'static crate::models::registry::ModelSpec,
) {
    use tauri::Emitter;

    // Require split towers; fall back to SIGLIP_BASE if the chosen model lacks them.
    let spec: &'static crate::models::registry::ModelSpec = if requested_spec.vision_file.is_some()
    {
        requested_spec
    } else {
        warn!(
            "[pipeline] model '{}' has no split towers; falling back to SIGLIP_BASE",
            requested_spec.id
        );
        &crate::models::registry::SIGLIP_BASE
    };
    let preset = &crate::models::registry::BUFFALO_S_PRESET;

    info!("[pipeline] Ensuring embed model is ready...");
    if let Err(e) = manager.ensure_ready(&app, spec).await {
        error!("[pipeline] embed model not ready: {e}");
        return;
    }
    info!("[pipeline] Embed model ready.");

    for face_spec in [preset.detector, preset.embedder, preset.gender_age] {
        info!("[pipeline] Ensuring face model is ready ({})", face_spec.id);
        if let Err(e) = manager.ensure_ready(&app, face_spec).await {
            error!("[pipeline] face model not ready ({}): {e}", face_spec.id);
            return;
        }
    }
    info!("[pipeline] Face models ready.");

    let analyzer = match engine.get_face_analyzer(&manager, preset).await {
        Ok(a) => a,
        Err(e) => {
            error!("[pipeline] face analyzer init failed: {e}");
            return;
        }
    };
    info!("[pipeline] Face analyzer initialized.");

    let embed_tx = embed_actor::spawn_embed_actor(
        engine.clone(),
        manager.clone(),
        spec,
        config.batch_size,
        config.infer_channel_depth,
    );
    let face_tx = face_actor::spawn_face_actor(analyzer, config.infer_channel_depth);

    info!("[pipeline] Pipeline background loop started, awaiting tasks...");

    loop {
        // Pull both queues
        let sem_batch = match crate::pipeline::queue::get_queue_batch(
            &pool,
            "semantic",
            config.batch_size as i64,
        )
        .await
        {
            Ok(b) => b,
            Err(e) => {
                error!("[pipeline] semantic queue fetch error: {e}");
                vec![]
            }
        };
        let sub_batch = match crate::pipeline::queue::get_queue_batch(
            &pool,
            "subject",
            config.batch_size as i64,
        )
        .await
        {
            Ok(b) => b,
            Err(e) => {
                error!("[pipeline] subject queue fetch error: {e}");
                vec![]
            }
        };

        if sem_batch.is_empty() && sub_batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        debug!(
            "[pipeline] Loop waking up. Found {} semantic pending, {} subject pending",
            sem_batch.len(),
            sub_batch.len()
        );

        // Merge by image_id, tracking separate queue_ids for each operation
        let mut image_work: std::collections::HashMap<i64, (WorkSlot, WorkSlot)> =
            std::collections::HashMap::new();
        for (qid, image_id, attempts) in sem_batch {
            image_work.entry(image_id).or_default().0 = Some((qid, attempts));
        }
        for (qid, image_id, attempts) in sub_batch {
            image_work.entry(image_id).or_default().1 = Some((qid, attempts));
        }
        let batch: Vec<(i64, WorkSlot, WorkSlot)> = image_work
            .into_iter()
            .map(|(image_id, (sem, sub))| (image_id, sem, sub))
            .collect();

        info!(
            "[pipeline] Processing batch of {} distinct images",
            batch.len()
        );

        // Stage 1: bounded-parallel decode
        let sem = Arc::new(tokio::sync::Semaphore::new(config.load_channel_depth));
        let mut handles = Vec::new();
        for (image_id, sem_entry, sub_entry) in batch {
            let pool_c = pool.clone();
            let permit = sem
                .clone()
                .acquire_owned()
                .await
                .expect("local semaphore closed unexpectedly");
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let image = match crate::library::repo::get_image_by_id(&pool_c, image_id).await {
                    Ok(Some(i)) => i,
                    Ok(None) => {
                        return Err((sem_entry, sub_entry, format!("image {image_id} not found")))
                    }
                    Err(e) => return Err((sem_entry, sub_entry, e.to_string())),
                };
                let path = image.path.clone();
                match tokio::task::spawn_blocking(move || {
                    decoded_image::load_decoded(image_id, std::path::Path::new(&path))
                })
                .await
                {
                    Ok(Ok(d)) => Ok((image_id, sem_entry, sub_entry, d)),
                    Ok(Err(e)) => Err((sem_entry, sub_entry, e.to_string())),
                    Err(e) => Err((sem_entry, sub_entry, e.to_string())),
                }
            }));
        }
        let mut decoded = Vec::new();
        for h in handles {
            match h.await {
                Ok(Ok(x)) => {
                    decoded.push(x);
                }
                Ok(Err((sem_entry, sub_entry, err_msg))) => {
                    error!("[pipeline] decode failed: {err_msg}");
                    if let Some((sem_qid, sem_attempts)) = sem_entry {
                        let _ = crate::pipeline::queue::mark_failed(
                            &pool,
                            sem_qid,
                            sem_attempts,
                            &err_msg,
                        )
                        .await;
                    }
                    if let Some((sub_qid, sub_attempts)) = sub_entry {
                        let _ = crate::pipeline::queue::mark_failed(
                            &pool,
                            sub_qid,
                            sub_attempts,
                            &err_msg,
                        )
                        .await;
                    }
                }
                Err(e) => error!("[pipeline] decode task panicked: {e}"),
            }
        }

        let images_processed_this_iter = decoded.len();
        debug!(
            "[pipeline] Decoded {}/{} images for inference",
            images_processed_this_iter,
            decoded.len()
        );

        // Stage 2: dispatch embed + face, write results
        let mut processed_subject_work = false;

        // Phase A — pre-dispatch all embed requests before awaiting any reply.
        // This fills the embed actor's channel so its try_recv loop drains a
        // real batch (up to batch_size) instead of processing images one-by-one.
        struct Pending {
            image_id: i64,
            sem_entry: Option<(i64, i32)>,
            sub_entry: Option<(i64, i32)>,
            d: DecodedImage,
            erx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<Vec<f32>>>>,
        }
        let mut pending: Vec<Pending> = Vec::with_capacity(decoded.len());
        for (image_id, sem_entry, sub_entry, d) in decoded {
            let erx = if let Some((sem_qid, sem_attempts)) = sem_entry {
                let (etx, erx) = tokio::sync::oneshot::channel();
                if embed_tx
                    .send(embed_actor::EmbedRequest {
                        decoded: d.clone(),
                        reply: etx,
                    })
                    .await
                    .is_ok()
                {
                    Some(erx)
                } else {
                    let _ = crate::pipeline::queue::mark_failed(
                        &pool,
                        sem_qid,
                        sem_attempts,
                        "embed actor closed",
                    )
                    .await;
                    None
                }
            } else {
                None
            };
            pending.push(Pending {
                image_id,
                sem_entry,
                sub_entry,
                d,
                erx,
            });
        }

        // Phase B — for each image: dispatch face then join!(embed_result, face_result).
        // This restores the embed/face overlap that the old serial CPU path dropped,
        // while the pre-dispatched embed batch is processed by the actor.
        for Pending {
            image_id,
            sem_entry,
            sub_entry,
            d,
            erx,
        } in pending
        {
            let frx = if let Some((sub_qid, sub_attempts)) = sub_entry {
                let (ftx, frx) = tokio::sync::oneshot::channel();
                if face_tx
                    .send(face_actor::FaceRequest {
                        decoded: d.clone(),
                        reply: ftx,
                    })
                    .await
                    .is_ok()
                {
                    Some(frx)
                } else {
                    let _ = crate::pipeline::queue::mark_failed(
                        &pool,
                        sub_qid,
                        sub_attempts,
                        "face actor closed",
                    )
                    .await;
                    None
                }
            } else {
                None
            };

            match (erx, frx) {
                (Some(erx), Some(frx)) => {
                    let (emb_result, face_result) = tokio::join!(erx, frx);
                    match emb_result {
                        Ok(Ok(emb)) => {
                            let blob = crate::search::math::f32_slice_to_bytes(&emb);
                            if let Some((sem_qid, _)) = sem_entry {
                                if crate::pipeline::queue::mark_semantic_analysis_done(
                                    &pool, sem_qid, image_id, &blob,
                                )
                                .await
                                .is_ok()
                                {
                                    index.write().unwrap().add(image_id, &emb);
                                    info!(
                                        "[pipeline] Saved semantic embedding for image {image_id}"
                                    );
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            error!("[pipeline] Embedding error for image {image_id}: {e}");
                            if let Some((sem_qid, sem_attempts)) = sem_entry {
                                let _ = crate::pipeline::queue::mark_failed(
                                    &pool,
                                    sem_qid,
                                    sem_attempts,
                                    &e.to_string(),
                                )
                                .await;
                            }
                        }
                        Err(_) => {
                            error!("[pipeline] Embed reply channel dropped for image {image_id}");
                            if let Some((sem_qid, sem_attempts)) = sem_entry {
                                let _ = crate::pipeline::queue::mark_failed(
                                    &pool,
                                    sem_qid,
                                    sem_attempts,
                                    "embed reply channel dropped",
                                )
                                .await;
                            }
                        }
                    }
                    match face_result {
                        Ok(Ok(faces)) => {
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                info!("[pipeline] Found {} faces in image {image_id}", faces.len());
                                save_faces(&pool, image_id, sub_qid, sub_attempts, faces).await;
                                processed_subject_work = true;
                            }
                        }
                        Ok(Err(e)) => {
                            error!("[pipeline] Face analysis error for image {image_id}: {e}");
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                let _ = crate::pipeline::queue::mark_failed(
                                    &pool,
                                    sub_qid,
                                    sub_attempts,
                                    &e.to_string(),
                                )
                                .await;
                            }
                        }
                        Err(_) => {
                            error!("[pipeline] Face reply channel dropped for image {image_id}");
                            if let Some((sub_qid, sub_attempts)) = sub_entry {
                                let _ = crate::pipeline::queue::mark_failed(
                                    &pool,
                                    sub_qid,
                                    sub_attempts,
                                    "face reply channel dropped",
                                )
                                .await;
                            }
                        }
                    }
                }
                (Some(erx), None) => match erx.await {
                    Ok(Ok(emb)) => {
                        let blob = crate::search::math::f32_slice_to_bytes(&emb);
                        if let Some((sem_qid, _)) = sem_entry {
                            if crate::pipeline::queue::mark_semantic_analysis_done(
                                &pool, sem_qid, image_id, &blob,
                            )
                            .await
                            .is_ok()
                            {
                                index.write().unwrap().add(image_id, &emb);
                                info!("[pipeline] Saved semantic embedding for image {image_id}");
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        error!("[pipeline] Embedding error for image {image_id}: {e}");
                        if let Some((sem_qid, sem_attempts)) = sem_entry {
                            let _ = crate::pipeline::queue::mark_failed(
                                &pool,
                                sem_qid,
                                sem_attempts,
                                &e.to_string(),
                            )
                            .await;
                        }
                    }
                    Err(_) => {
                        error!("[pipeline] Embed reply channel dropped for image {image_id}");
                        if let Some((sem_qid, sem_attempts)) = sem_entry {
                            let _ = crate::pipeline::queue::mark_failed(
                                &pool,
                                sem_qid,
                                sem_attempts,
                                "embed reply channel dropped",
                            )
                            .await;
                        }
                    }
                },
                (None, Some(frx)) => match frx.await {
                    Ok(Ok(faces)) => {
                        if let Some((sub_qid, sub_attempts)) = sub_entry {
                            info!("[pipeline] Found {} faces in image {image_id}", faces.len());
                            save_faces(&pool, image_id, sub_qid, sub_attempts, faces).await;
                            processed_subject_work = true;
                        }
                    }
                    Ok(Err(e)) => {
                        error!("[pipeline] Face analysis error for image {image_id}: {e}");
                        if let Some((sub_qid, sub_attempts)) = sub_entry {
                            let _ = crate::pipeline::queue::mark_failed(
                                &pool,
                                sub_qid,
                                sub_attempts,
                                &e.to_string(),
                            )
                            .await;
                        }
                    }
                    Err(_) => {
                        error!("[pipeline] Face reply channel dropped for image {image_id}");
                        if let Some((sub_qid, sub_attempts)) = sub_entry {
                            let _ = crate::pipeline::queue::mark_failed(
                                &pool,
                                sub_qid,
                                sub_attempts,
                                "face reply channel dropped",
                            )
                            .await;
                        }
                    }
                },
                (None, None) => {}
            }

            // Second emit: signals full analysis complete (embeddings + faces written). NOT ordered
            // vs. the Stage 1 thumbnail emit — frontend must handle either order (TT-12 Option A).
            let _ = app.emit(
                "image_updated",
                crate::models::ImageUpdatedPayload { image_id },
            );
        }

        crate::search::math::emit_progress(&pool, &app).await;

        // Persist index snapshot
        let snap_path = data_dir.join("nebula.idx");
        let index_snap = Arc::clone(&index);
        tokio::task::spawn_blocking(move || {
            let guard = index_snap.read().unwrap();
            if let Err(e) = guard.save(&snap_path) {
                error!("[pipeline] failed to save index snapshot: {e}");
            }
        })
        .await
        .ok();

        // Auto-recluster only when subject work was done this iteration
        if processed_subject_work {
            info!("[pipeline] Auto-clustering unassigned faces...");
            if let Ok(_result) = crate::people::clustering::cluster_unassigned_faces(&pool, None).await {
                info!("[pipeline] Clustering complete. Upgrading subject thumbnails...");
                // Upgrade each subject's profile crop to its best-quality face, then
                // generate the crop file eagerly so the People grid has it before the
                // frontend asks (closes the lazy-generation first-paint delay).
                if let Ok(changed) = crate::people::repo::upgrade_subject_thumbnails(&pool).await {
                    info!(
                        "[pipeline] Upgraded thumbnails for {} subjects",
                        changed.len()
                    );
                    for (_subject_id, face_id) in changed {
                        if let Ok(Some((path, bbox))) =
                            crate::people::repo::get_face_with_image(&pool, face_id).await
                        {
                            let dest =
                                crate::media::thumbnail::face_crop_path_for(&data_dir, face_id);
                            debug!(
                                "[pipeline] Eagerly generating face crop for face_id {} to {:?}",
                                face_id, dest
                            );
                            if let Err(e) = crate::media::thumbnail::generate_face_crop(
                                std::path::PathBuf::from(path),
                                dest,
                                bbox,
                            )
                            .await
                            {
                                error!("[pipeline] eager crop gen failed for face {face_id}: {e}");
                            }
                        }
                    }
                }
                let _ = app.emit("subjects_updated", ());
            } else {
                error!("[pipeline] Clustering failed");
            }
        }
    }
}
