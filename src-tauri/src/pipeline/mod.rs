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
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;

/// Upgrade each subject's profile crop to its best-quality face, eagerly generate
/// the crop file, then emit `subjects_updated` so the People view refreshes.
async fn upgrade_thumbnails_and_emit(
    pool: &sqlx::SqlitePool,
    data_dir: &std::path::Path,
    app: &tauri::AppHandle,
) {
    use tauri::Emitter;
    if let Ok(changed) = crate::people::repo::upgrade_subject_thumbnails(pool).await {
        debug!(
            "[pipeline] Upgraded thumbnails for {} subjects",
            changed.len()
        );
        for (_subject_id, face_id) in changed {
            if let Ok(Some((path, bbox))) =
                crate::people::repo::get_face_with_image(pool, face_id).await
            {
                let dest = crate::media::thumbnail::face_crop_path_for(data_dir, face_id);
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
}

async fn save_faces(
    pool: &sqlx::SqlitePool,
    image_id: i64,
    sub_qid: i64,
    sub_attempts: i32,
    embedder_id: &str,
    faces: Vec<face_actor::FaceResult>,
) -> Vec<i64> {
    if let Ok(None) = crate::library::repo::get_image_by_id(pool, image_id).await {
        debug!(
            "[pipeline] image {image_id} no longer exists (deleted mid-pipeline), skipping face save"
        );
        return Vec::new();
    }

    let detections: Vec<crate::people::service::DetectedFaceInput> = faces
        .into_iter()
        .map(|(detection, embedding, sharp)| {
            let bbox = detection.bbox;
            let rel = (
                bbox.x1 as f64,
                bbox.y1 as f64,
                (bbox.x2 - bbox.x1) as f64,
                (bbox.y2 - bbox.y1) as f64,
            );
            let frontality =
                crate::people::face_quality::frontality(detection.landmarks.as_deref());
            let quality =
                crate::people::face_quality::composite(detection.score, frontality, sharp);
            crate::people::service::DetectedFaceInput {
                bbox: rel,
                det_score: detection.score as f64,
                quality_score: quality as f64,
                embedding,
            }
        })
        .collect();

    let existing = match crate::people::repo::list_faces_for_image(pool, image_id).await {
        Ok(rows) => rows,
        Err(e) => {
            error!("[pipeline] list_faces_for_image failed for image {image_id}: {e}");
            let _ =
                crate::pipeline::queue::mark_failed(pool, sub_qid, sub_attempts, &e.to_string())
                    .await;
            return Vec::new();
        }
    };

    match crate::people::service::reprocess_image_faces(
        pool,
        image_id,
        embedder_id,
        detections,
        existing,
    )
    .await
    {
        Ok(touched) => {
            let _ =
                crate::pipeline::queue::mark_subject_analysis_done(pool, sub_qid, image_id).await;
            touched
        }
        Err(e) => {
            if is_missing_image_fk_error(&e) {
                debug!(
                    "[pipeline] image {image_id} was deleted mid-pipeline (FK violation on face insert), skipping"
                );
            } else {
                error!("[pipeline] reprocess_image_faces failed for image {image_id}: {e}");
                let _ = crate::pipeline::queue::mark_failed(
                    pool,
                    sub_qid,
                    sub_attempts,
                    &e.to_string(),
                )
                .await;
            }
            Vec::new()
        }
    }
}

/// True if `e` wraps a SQLite "FOREIGN KEY constraint failed" error — the
/// shape `insert_face` raises when the target image row was deleted (e.g.
/// via `delete_folder`'s cascade) between `save_faces`'s proactive existence
/// check and the insert itself.
fn is_missing_image_fk_error(e: &anyhow::Error) -> bool {
    e.to_string().contains("FOREIGN KEY constraint failed")
}

/// Resolve the `subject_model` setting to its preset, falling back to Blitz
/// for an unset or unrecognized value. Delegates to the settings slice's
/// resolution so the pipeline and the settings command agree on what "the
/// active preset" means.
async fn resolve_subject_preset(
    pool: &sqlx::SqlitePool,
) -> &'static crate::models::registry::FaceIdPreset {
    let value = crate::settings::repo::get_setting(pool, "subject_model")
        .await
        .ok()
        .flatten();
    crate::settings::commands::resolve_subject_preset(value.as_deref())
}

/// Ensure a preset's three models are downloaded and return its (cached or
/// freshly built) `FaceAnalyzer`. `VisionEngine::get_face_analyzer` already
/// caches by `preset.id` internally, so calling this repeatedly with the same
/// preset is cheap — only a preset change triggers a real rebuild.
async fn ensure_face_preset(
    app: &tauri::AppHandle,
    engine: &crate::vision::engine::VisionEngine,
    manager: &crate::models::ModelManager,
    preset: &'static crate::models::registry::FaceIdPreset,
) -> anyhow::Result<Arc<face_id::analyzer::FaceAnalyzer>> {
    for face_spec in [preset.detector, preset.embedder, preset.gender_age] {
        manager
            .ensure_ready(app, face_spec)
            .await
            .map_err(|e| anyhow::anyhow!("face model not ready ({}): {e}", face_spec.id))?;
    }
    engine.get_face_analyzer(manager, preset).await
}

/// Distinguishes "the image row was deleted mid-flight" (expected race with
/// `library::repo::delete_folder`'s cascade) from any other decode error.
enum DecodeFailure {
    ImageGone,
    Other(String),
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

    info!("[pipeline] Ensuring embed model is ready...");
    if let Err(e) = manager.ensure_ready(&app, spec).await {
        error!("[pipeline] embed model not ready: {e}");
        return;
    }
    info!("[pipeline] Embed model ready.");

    let initial_preset = resolve_subject_preset(&pool).await;
    let initial_analyzer = match ensure_face_preset(&app, &engine, &manager, initial_preset).await {
        Ok(a) => a,
        Err(e) => {
            error!("[pipeline] face analyzer init failed: {e}");
            return;
        }
    };
    info!(
        "[pipeline] Face analyzer initialized ('{}').",
        initial_preset.id
    );
    let mut subject_preset = initial_preset;
    let mut face_tx = face_actor::spawn_face_actor(initial_analyzer, config.infer_channel_depth);

    let embed_tx = embed_actor::spawn_embed_actor(
        engine.clone(),
        manager.clone(),
        spec,
        config.batch_size,
        config.infer_channel_depth,
    );

    info!("[pipeline] Pipeline background loop started, awaiting tasks...");

    // Recover the dirty flag across restarts. Incremental edges are already
    // persisted in `face_edges`, so the idle full sweep is the only state needed to
    // reconcile work left in flight when the app last closed.
    let mut clustering_dirty: bool = crate::settings::repo::get_setting(&pool, "clustering_dirty")
        .await
        .ok()
        .flatten()
        .map(|s| s == "true")
        .unwrap_or(false);
    info!("[pipeline] clustering state recovered: dirty={clustering_dirty}");

    loop {
        // Per-batch preset resolution (§1 wiring fix): a mid-session
        // subject_model change takes effect on the next iteration with no
        // restart or signalling machinery. The analyzer is only rebuilt when
        // the resolved preset id actually differs from the one already loaded.
        let resolved_preset = resolve_subject_preset(&pool).await;
        if resolved_preset.id != subject_preset.id {
            match ensure_face_preset(&app, &engine, &manager, resolved_preset).await {
                Ok(analyzer) => {
                    face_tx = face_actor::spawn_face_actor(analyzer, config.infer_channel_depth);
                    subject_preset = resolved_preset;
                    info!(
                        "[pipeline] subject_model switched to '{}'",
                        subject_preset.id
                    );
                }
                Err(e) => {
                    error!(
                        "[pipeline] failed to switch subject preset to '{}', keeping '{}': {e}",
                        resolved_preset.id, subject_preset.id
                    );
                }
            }
        }

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
            // Idle backstop: one authoritative full sweep reconciles any drift
            // accumulated by the incremental path. Cancellable so new import work
            // preempts it instead of stalling.
            if clustering_dirty {
                info!("[pipeline] Idle: running authoritative full clustering sweep...");
                // Non-blocking cancellation: a background poller flips the flag when
                // new inference work lands, and the sweep reads it between faces.
                let cancel_flag = Arc::new(AtomicBool::new(false));
                let poll_flag = cancel_flag.clone();
                let poll_pool = pool.clone();
                let poller = tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        if crate::pipeline::queue::count_pending_inference(&poll_pool)
                            .await
                            .unwrap_or(0)
                            > 0
                        {
                            poll_flag.store(true, AtomicOrdering::Relaxed);
                            break;
                        }
                    }
                });
                let result = crate::people::clustering::cluster_unassigned_faces(
                    &pool,
                    subject_preset.embedder.id,
                    Some(cancel_flag.as_ref()),
                )
                .await;
                poller.abort();
                match result {
                    Ok(Some(_)) => {
                        upgrade_thumbnails_and_emit(&pool, &data_dir, &app).await;
                        clustering_dirty = false;
                        let _ =
                            crate::settings::repo::set_setting(&pool, "clustering_dirty", "false")
                                .await;
                        info!("[pipeline] Idle full sweep complete.");
                    }
                    Ok(None) => {
                        info!("[pipeline] Idle full sweep cancelled — new work arrived.");
                    }
                    Err(e) => error!("[pipeline] idle full sweep failed: {e}"),
                }
            }
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
                        return Err((image_id, sem_entry, sub_entry, DecodeFailure::ImageGone))
                    }
                    Err(e) => {
                        return Err((
                            image_id,
                            sem_entry,
                            sub_entry,
                            DecodeFailure::Other(e.to_string()),
                        ))
                    }
                };
                let path = image.path.clone();
                match tokio::task::spawn_blocking(move || {
                    decoded_image::load_decoded(image_id, std::path::Path::new(&path))
                })
                .await
                {
                    Ok(Ok(d)) => Ok((image_id, sem_entry, sub_entry, d)),
                    Ok(Err(e)) => Err((
                        image_id,
                        sem_entry,
                        sub_entry,
                        DecodeFailure::Other(e.to_string()),
                    )),
                    Err(e) => Err((
                        image_id,
                        sem_entry,
                        sub_entry,
                        DecodeFailure::Other(e.to_string()),
                    )),
                }
            }));
        }
        let mut decoded = Vec::new();
        for h in handles {
            match h.await {
                Ok(Ok(x)) => {
                    decoded.push(x);
                }
                Ok(Err((image_id, _sem_entry, _sub_entry, DecodeFailure::ImageGone))) => {
                    debug!(
                        "[pipeline] image {image_id} not found (deleted mid-pipeline), skipping decode"
                    );
                }
                Ok(Err((_image_id, sem_entry, sub_entry, DecodeFailure::Other(err_msg)))) => {
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
        // Exact set of faces vectorized this iteration — drives the incremental
        // edge update without relying on face-id ordering.
        let mut batch_new_face_ids: Vec<i64> = Vec::new();

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
                                let new_ids = save_faces(
                                    &pool,
                                    image_id,
                                    sub_qid,
                                    sub_attempts,
                                    subject_preset.embedder.id,
                                    faces,
                                )
                                .await;
                                batch_new_face_ids.extend(new_ids);
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
                            save_faces(
                                &pool,
                                image_id,
                                sub_qid,
                                sub_attempts,
                                subject_preset.embedder.id,
                                faces,
                            )
                            .await;
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

        // Incremental clustering on the critical path: only edges for newly
        // vectorized faces, then an in-memory relabel. Both are cheap, so the loop
        // immediately pulls the next batch. The authoritative full sweep is
        // deferred to the idle branch.
        if processed_subject_work {
            let incremental_result: anyhow::Result<()> = async {
                if !batch_new_face_ids.is_empty() {
                    crate::people::clustering::update_edges_incremental(
                        &pool,
                        &batch_new_face_ids,
                        subject_preset.embedder.id,
                    )
                    .await?;
                }
                // Constraints/assignments may have changed even with no new
                // vectors, so always relabel.
                crate::people::clustering::relabel_from_edges(&pool, subject_preset.embedder.id)
                    .await?;
                Ok(())
            }
            .await;

            match incremental_result {
                Ok(()) => {
                    upgrade_thumbnails_and_emit(&pool, &data_dir, &app).await;
                    clustering_dirty = true;
                    let _ =
                        crate::settings::repo::set_setting(&pool, "clustering_dirty", "true").await;
                }
                Err(e) => error!("[pipeline] incremental clustering failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_missing_image_fk_error;

    #[test]
    fn recognizes_fk_constraint_error_message() {
        let e = anyhow::anyhow!(
            "error returned from database: (code: 787) FOREIGN KEY constraint failed"
        );
        assert!(is_missing_image_fk_error(&e));
        assert!(!is_missing_image_fk_error(&anyhow::anyhow!(
            "some other db error"
        )));
    }
}
