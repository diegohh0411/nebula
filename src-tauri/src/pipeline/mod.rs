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
            record_queue_failure(
                pool,
                "subject",
                image_id,
                sub_qid,
                sub_attempts,
                &e.to_string(),
            )
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
            if let Err(e) =
                crate::pipeline::queue::mark_subject_analysis_done(pool, sub_qid, image_id).await
            {
                error!(
                    "[pipeline] failed to mark subject analysis done for image {image_id}: {e:#}"
                );
            }
            touched
        }
        Err(e) => {
            if is_missing_image_fk_error(&e) {
                debug!(
                    "[pipeline] image {image_id} was deleted mid-pipeline (FK violation on face insert), skipping"
                );
            } else {
                error!("[pipeline] reprocess_image_faces failed for image {image_id}: {e}");
                record_queue_failure(
                    pool,
                    "subject",
                    image_id,
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

/// Persist face detections and collect newly touched face IDs for incremental
/// clustering. Shared by both dual-work and face-only Phase B arms so neither
/// can silently drop `save_faces`'s return value (TT-95).
///
/// Returns `true` only when faces were delivered and a subject queue entry was
/// present — i.e. `save_faces` was invoked (even if it returned no IDs). Analysis
/// errors and dropped oneshots return `false` via [`handle_face_result`]; those
/// paths record queue failures but do not flip `processed_subject_work`.
async fn save_faces_and_collect(
    pool: &sqlx::SqlitePool,
    image_id: i64,
    sub_entry: WorkSlot,
    embedder_id: &str,
    faces: Vec<face_actor::FaceResult>,
    batch_new_face_ids: &mut Vec<i64>,
) -> bool {
    let Some((sub_qid, sub_attempts)) = sub_entry else {
        return false;
    };
    info!("[pipeline] Found {} faces in image {image_id}", faces.len());
    let new_ids = save_faces(pool, image_id, sub_qid, sub_attempts, embedder_id, faces).await;
    batch_new_face_ids.extend(new_ids);
    true
}

/// Handle a face-actor oneshot reply for one image. Shared by dual-work and
/// face-only Phase B arms so success/error paths cannot drift (TT-95).
///
/// Returns `true` only on the success path when a subject queue entry was present
/// (see [`save_faces_and_collect`]). Analysis errors and dropped replies return
/// `false` after recording the queue failure.
async fn handle_face_result(
    pool: &sqlx::SqlitePool,
    image_id: i64,
    sub_entry: WorkSlot,
    embedder_id: &str,
    face_result: Result<
        anyhow::Result<Vec<face_actor::FaceResult>>,
        tokio::sync::oneshot::error::RecvError,
    >,
    batch_new_face_ids: &mut Vec<i64>,
) -> bool {
    match face_result {
        Ok(Ok(faces)) => {
            save_faces_and_collect(
                pool,
                image_id,
                sub_entry,
                embedder_id,
                faces,
                batch_new_face_ids,
            )
            .await
        }
        Ok(Err(e)) => {
            error!("[pipeline] Face analysis error for image {image_id}: {e}");
            if let Some((sub_qid, sub_attempts)) = sub_entry {
                record_queue_failure(
                    pool,
                    "subject",
                    image_id,
                    sub_qid,
                    sub_attempts,
                    &e.to_string(),
                )
                .await;
            }
            false
        }
        Err(_) => {
            error!("[pipeline] Face reply channel dropped for image {image_id}");
            if let Some((sub_qid, sub_attempts)) = sub_entry {
                record_queue_failure(
                    pool,
                    "subject",
                    image_id,
                    sub_qid,
                    sub_attempts,
                    "face reply channel dropped",
                )
                .await;
            }
            false
        }
    }
}

/// Phase B body for one image: await pre-dispatched embed/face replies and
/// persist results. Extracted so tests can drive the face-only arm
/// (`erx = None, frx = Some`) and assert `batch_new_face_ids` is extended —
/// the exact TT-95 wiring that previously discarded `save_faces` IDs.
///
/// Returns whether subject work was attempted (OR into `processed_subject_work`).
async fn await_phase_b_replies(
    pool: &sqlx::SqlitePool,
    index: &crate::search::vector_index::IndexStore,
    pending: Pending,
    embedder_id: &str,
    batch_new_face_ids: &mut Vec<i64>,
) -> bool {
    let Pending {
        image_id,
        sem_entry,
        sub_entry,
        erx,
        frx,
    } = pending;
    let mut subject_work = false;
    match (erx, frx) {
        (Some(erx), Some(frx)) => {
            let (emb_result, face_result) = tokio::join!(erx, frx);
            match emb_result {
                Ok(Ok(emb)) => {
                    save_semantic(pool, index, sem_entry, image_id, &emb).await;
                }
                Ok(Err(e)) => {
                    error!("[pipeline] Embedding error for image {image_id}: {e}");
                    if let Some((sem_qid, sem_attempts)) = sem_entry {
                        record_queue_failure(
                            pool,
                            "semantic",
                            image_id,
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
                        record_queue_failure(
                            pool,
                            "semantic",
                            image_id,
                            sem_qid,
                            sem_attempts,
                            "embed reply channel dropped",
                        )
                        .await;
                    }
                }
            }
            subject_work |= handle_face_result(
                pool,
                image_id,
                sub_entry,
                embedder_id,
                face_result,
                batch_new_face_ids,
            )
            .await;
        }
        (Some(erx), None) => match erx.await {
            Ok(Ok(emb)) => {
                save_semantic(pool, index, sem_entry, image_id, &emb).await;
            }
            Ok(Err(e)) => {
                error!("[pipeline] Embedding error for image {image_id}: {e}");
                if let Some((sem_qid, sem_attempts)) = sem_entry {
                    record_queue_failure(
                        pool,
                        "semantic",
                        image_id,
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
                    record_queue_failure(
                        pool,
                        "semantic",
                        image_id,
                        sem_qid,
                        sem_attempts,
                        "embed reply channel dropped",
                    )
                    .await;
                }
            }
        },
        (None, Some(frx)) => {
            subject_work |= handle_face_result(
                pool,
                image_id,
                sub_entry,
                embedder_id,
                frx.await,
                batch_new_face_ids,
            )
            .await;
        }
        (None, None) => {}
    }
    subject_work
}

/// Per-iteration accumulator for subject-analysis outcomes.
///
/// Owns both the `processed` flag and the face IDs that must reach
/// `update_edges_incremental`, so `run_pipeline` cannot feed incremental
/// clustering a different buffer than Phase B wrote into (TT-95 call-site
/// wiring). Phase B always extends `face_ids` via [`SubjectWorkBatch::absorb`].
struct SubjectWorkBatch {
    face_ids: Vec<i64>,
    processed: bool,
}

impl SubjectWorkBatch {
    fn new() -> Self {
        Self {
            face_ids: Vec::new(),
            processed: false,
        }
    }

    async fn absorb(
        &mut self,
        pool: &sqlx::SqlitePool,
        index: &crate::search::vector_index::IndexStore,
        pending: Pending,
        embedder_id: &str,
    ) {
        self.processed |=
            await_phase_b_replies(pool, index, pending, embedder_id, &mut self.face_ids).await;
    }
}

/// True if `e` wraps a SQLite "FOREIGN KEY constraint failed" error — the
/// shape `insert_face` raises when the target image row was deleted (e.g.
/// via `delete_folder`'s cascade) between `save_faces`'s proactive existence
/// check and the insert itself.
fn is_missing_image_fk_error(e: &anyhow::Error) -> bool {
    e.to_string().contains("FOREIGN KEY constraint failed")
}

/// Record a failed queue attempt without ever swallowing the outcome: a
/// dead-letter is warned, an exhausted-but-rescheduled retry is silent, and a
/// DB write failure is logged so an entry can never get stuck invisibly.
/// `stage` names the pipeline ("semantic"/"subject") for the log line.
async fn record_queue_failure(
    pool: &sqlx::SqlitePool,
    stage: &str,
    image_id: i64,
    queue_id: i64,
    attempts: i32,
    error: &str,
) {
    use crate::pipeline::queue::FailureOutcome;
    match crate::pipeline::queue::mark_failed(pool, queue_id, attempts, error).await {
        Ok(FailureOutcome::DeadLettered) => {
            warn!("[pipeline] image {image_id} {stage} entry dead-lettered after repeated failures")
        }
        Ok(FailureOutcome::Retrying) => {}
        Err(e) => error!("[pipeline] failed to record {stage} failure for image {image_id}: {e:#}"),
    }
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
    /// The `images` row was deleted mid-pipeline (FK/lookup miss).
    ImageGone,
    /// The source file no longer exists on disk (moved/deleted after indexing).
    /// Carries the path for logging. Permanent — dead-lettered immediately.
    Missing(String),
    /// Any other decode error, carrying the full cause chain for logging.
    Other(String),
}

/// A batch image whose applicable embed/face requests were dispatched in
/// Phase A. Phase B awaits `erx`/`frx` and persists the results. Receivers are
/// `None` when that image had no corresponding queue slot, or the actor's
/// channel was already closed when we tried to send.
struct Pending {
    image_id: i64,
    sem_entry: Option<(i64, i32)>,
    sub_entry: Option<(i64, i32)>,
    erx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<Vec<f32>>>>,
    frx: Option<tokio::sync::oneshot::Receiver<anyhow::Result<Vec<face_actor::FaceResult>>>>,
}

/// Phase A: pre-dispatch every image's embed *and* face request before any
/// reply is awaited. Filling both actor channels up front lets the embed actor
/// drain a real batch, and lets face inference for image N+1 begin while image
/// N's results are still being persisted in Phase B — instead of gating each
/// face dispatch behind the previous image's `join!`.
///
/// Non-blocking as long as each channel's depth (`infer_channel_depth`) is at
/// least the batch size — `run_pipeline` enforces this with an `assert!`
/// before spawning the actors. Under that invariant, every `send().await`
/// here returns without waiting for the actor to free buffer space (or for a
/// oneshot reply). On a closed channel the affected queue slot is marked
/// failed and its receiver is left `None`.
async fn dispatch_batch(
    pool: &sqlx::SqlitePool,
    decoded: Vec<(i64, WorkSlot, WorkSlot, DecodedImage)>,
    embed_tx: &tokio::sync::mpsc::Sender<embed_actor::EmbedRequest>,
    face_tx: &tokio::sync::mpsc::Sender<face_actor::FaceRequest>,
) -> Vec<Pending> {
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
                record_queue_failure(
                    pool,
                    "semantic",
                    image_id,
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
                record_queue_failure(
                    pool,
                    "subject",
                    image_id,
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
        pending.push(Pending {
            image_id,
            sem_entry,
            sub_entry,
            erx,
            frx,
        });
    }
    pending
}

/// Persist a successful semantic embedding, logging any DB write failure
/// instead of silently discarding it (which would leave the queue entry to be
/// retried forever with no signal).
async fn save_semantic(
    pool: &sqlx::SqlitePool,
    index: &crate::search::vector_index::IndexStore,
    sem_entry: WorkSlot,
    image_id: i64,
    emb: &[f32],
) {
    if let Some((sem_qid, _)) = sem_entry {
        let blob = crate::search::math::f32_slice_to_bytes(emb);
        match crate::pipeline::queue::mark_semantic_analysis_done(pool, sem_qid, image_id, &blob)
            .await
        {
            Ok(()) => {
                index.write().unwrap().add(image_id, emb);
                info!("[pipeline] Saved semantic embedding for image {image_id}");
            }
            Err(e) => {
                error!("[pipeline] failed to save semantic embedding for image {image_id}: {e:#}")
            }
        }
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

    // dispatch_batch's non-blocking Phase A depends on this holding: every
    // batch (bounded by batch_size) must fit in each actor channel's depth
    // without a send waiting for the actor to free buffer space. Catch a
    // misconfiguration loudly here (including release builds) rather than
    // surface as backpressure or a hang deep inside the pipeline loop.
    assert!(
        config.batch_size <= config.infer_channel_depth,
        "PipelineConfig.batch_size ({}) must be <= infer_channel_depth ({}) \
         or Phase A pre-dispatch can block on a full channel",
        config.batch_size,
        config.infer_channel_depth
    );

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
            &[],
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
            &[],
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
                        if crate::pipeline::queue::count_due_inference(&poll_pool)
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

        let batch_size = batch.len();
        info!("[pipeline] Processing batch of {batch_size} distinct images");

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
                    // Classify inside the blocking context: a file moved or
                    // deleted after indexing can never be decoded, so the caller
                    // drops it immediately instead of retrying forever. The
                    // existence check is filesystem IO, so it belongs here rather
                    // than on the async runtime thread. Use the alternate
                    // formatter (`{:#}`) to keep the full cause chain;
                    // `to_string()` would only show the outer context.
                    match decoded_image::load_decoded(image_id, std::path::Path::new(&path)) {
                        Ok(d) => Ok(d),
                        Err(_) if !std::path::Path::new(&path).exists() => {
                            Err(DecodeFailure::Missing(path))
                        }
                        Err(e) => Err(DecodeFailure::Other(format!("{e:#}"))),
                    }
                })
                .await
                {
                    Ok(Ok(d)) => Ok((image_id, sem_entry, sub_entry, d)),
                    Ok(Err(failure)) => Err((image_id, sem_entry, sub_entry, failure)),
                    Err(e) => Err((
                        image_id,
                        sem_entry,
                        sub_entry,
                        DecodeFailure::Other(format!("decode task failed: {e}")),
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
                Ok(Err((image_id, sem_entry, sub_entry, DecodeFailure::Missing(path)))) => {
                    // File is gone from disk — permanent. Drop both queue
                    // entries now so they stop dominating every batch.
                    warn!(
                        "[pipeline] image {image_id} file missing on disk, dropping from queue: {path}"
                    );
                    if let Some((sem_qid, _)) = sem_entry {
                        if let Err(e) = crate::pipeline::queue::dead_letter(&pool, sem_qid).await {
                            error!("[pipeline] failed to drop semantic entry for missing image {image_id}: {e:#}");
                        }
                    }
                    if let Some((sub_qid, _)) = sub_entry {
                        if let Err(e) = crate::pipeline::queue::dead_letter(&pool, sub_qid).await {
                            error!("[pipeline] failed to drop subject entry for missing image {image_id}: {e:#}");
                        }
                    }
                }
                Ok(Err((image_id, sem_entry, sub_entry, DecodeFailure::Other(err_msg)))) => {
                    error!("[pipeline] decode failed for image {image_id}: {err_msg}");
                    if let Some((sem_qid, sem_attempts)) = sem_entry {
                        record_queue_failure(
                            &pool,
                            "semantic",
                            image_id,
                            sem_qid,
                            sem_attempts,
                            &err_msg,
                        )
                        .await;
                    }
                    if let Some((sub_qid, sub_attempts)) = sub_entry {
                        record_queue_failure(
                            &pool,
                            "subject",
                            image_id,
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
            images_processed_this_iter, batch_size
        );

        // Stage 2: dispatch embed + face, write results.
        // SubjectWorkBatch owns both the processed flag and face IDs so
        // incremental clustering cannot be wired to a different buffer (TT-95).
        let mut subject_batch = SubjectWorkBatch::new();

        // Phase A — pre-dispatch every image's embed AND face request before
        // awaiting any reply. This fills the embed actor's channel so its
        // try_recv loop drains a real batch, and queues all face work so the
        // single face worker never idles waiting for Phase B to hand it the next
        // image. See dispatch_batch for the channel-depth invariant.
        let pending = dispatch_batch(&pool, decoded, &embed_tx, &face_tx).await;

        // Phase B — await both replies per image and persist results. Dispatch
        // already happened in Phase A (dispatch_batch), so this loop no longer
        // gates the next image's face work behind the current image's writes.
        for item in pending {
            let image_id = item.image_id;
            subject_batch
                .absorb(&pool, &index, item, subject_preset.embedder.id)
                .await;

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
        if subject_batch.processed {
            let incremental_result: anyhow::Result<()> = async {
                if !subject_batch.face_ids.is_empty() {
                    crate::people::clustering::update_edges_incremental(
                        &pool,
                        &subject_batch.face_ids,
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

    use super::{
        dispatch_batch, embed_actor, face_actor, handle_face_result, save_faces_and_collect,
        Pending, SubjectWorkBatch,
    };
    use crate::pipeline::DecodedImage;
    use crate::search::vector_index::{FlatIndex, IndexStore};
    use face_id::detector::{BoundingBox, DetectedFace};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// Owned test DB: closes the pool and removes the temp dir on drop.
    struct TestDb {
        pool: Option<sqlx::SqlitePool>,
        dir: PathBuf,
    }

    impl TestDb {
        async fn init() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            crate::db::ensure_sqlite_vec_registered();
            let dir = std::env::temp_dir().join(format!(
                "nebula_pipeline_test_{}_{}",
                std::process::id(),
                n
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let pool = crate::db::init_db(&dir).await.unwrap();
            Self {
                pool: Some(pool),
                dir,
            }
        }

        fn pool(&self) -> &sqlx::SqlitePool {
            self.pool.as_ref().expect("test pool still open")
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            drop(self.pool.take());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn empty_index() -> IndexStore {
        Arc::new(std::sync::RwLock::new(Box::new(FlatIndex::new(768))))
    }

    async fn seed_image(pool: &sqlx::SqlitePool) -> i64 {
        let folder_id: i64 = sqlx::query_scalar(
            "INSERT INTO folders (path, added_at) VALUES ('/tmp', 0) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query_scalar(
            "INSERT INTO images (folder_id, path, file_hash, mtime, added_at, updated_at)
             VALUES (?, '/tmp/x.jpg', 'hash', 0, 0, 0) RETURNING id",
        )
        .bind(folder_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn seed_subject_queue(pool: &sqlx::SqlitePool, image_id: i64) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO embedding_queue (image_id, pipeline, attempts, scheduled_at)
             VALUES (?, 'subject', 0, 0) RETURNING id",
        )
        .bind(image_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn queue_row_count(pool: &sqlx::SqlitePool, qid: i64) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM embedding_queue WHERE id = ?")
            .bind(qid)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn dummy_face(seed: f32) -> face_actor::FaceResult {
        // Distinct axis-aligned unit vectors so two faces aren't near-identical.
        let mut embedding = vec![0.0f32; 512];
        let idx = (seed as usize).saturating_sub(1).min(511);
        embedding[idx] = 1.0;
        (
            DetectedFace {
                bbox: BoundingBox {
                    x1: 0.1 + seed * 0.05,
                    y1: 0.1,
                    x2: 0.3 + seed * 0.05,
                    y2: 0.3,
                },
                landmarks: None,
                score: 0.95,
            },
            embedding,
            0.8,
        )
    }

    /// Without a subject queue entry, the helper must not claim work was
    /// attempted and must not touch the batch vec.
    #[tokio::test]
    async fn save_faces_and_collect_none_entry_returns_false_untouched() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let mut batch = vec![42i64];
        let attempted = save_faces_and_collect(
            &pool,
            1,
            None,
            "buffalo_s_recognition",
            vec![dummy_face(1.0)],
            &mut batch,
        )
        .await;
        assert!(!attempted);
        assert_eq!(batch, vec![42]);
    }

    /// Missing image → empty extend, but still `true` (save_faces was invoked).
    #[tokio::test]
    async fn save_faces_and_collect_missing_image_attempts_without_ids() {
        let db = TestDb::init().await;
        let mut batch = Vec::new();
        let attempted = save_faces_and_collect(
            db.pool(),
            999_999,
            Some((1, 0)),
            "buffalo_s_recognition",
            vec![dummy_face(1.0)],
            &mut batch,
        )
        .await;
        assert!(
            attempted,
            "Some(sub_entry) + delivered faces means save_faces was invoked"
        );
        assert!(
            batch.is_empty(),
            "missing image must not invent face IDs for incremental clustering"
        );
    }

    /// Helper-level pin: successful face persist must extend the batch vec
    /// and consume the subject queue entry.
    #[tokio::test]
    async fn save_faces_and_collect_extends_batch_with_new_face_ids() {
        let db = TestDb::init().await;
        let pool = db.pool();
        let image_id = seed_image(pool).await;
        let qid = seed_subject_queue(pool, image_id).await;
        let mut batch = Vec::new();
        let attempted = save_faces_and_collect(
            pool,
            image_id,
            Some((qid, 0)),
            "buffalo_s_recognition",
            vec![dummy_face(1.0), dummy_face(2.0)],
            &mut batch,
        )
        .await;
        assert!(attempted);
        assert_eq!(
            batch.len(),
            2,
            "new face IDs must land in batch_new_face_ids for update_edges_incremental"
        );
        let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM faces WHERE image_id = ?")
            .bind(image_id)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(stored, 2);
        assert_eq!(
            queue_row_count(pool, qid).await,
            0,
            "successful save must mark subject analysis done (delete queue row)"
        );
    }

    /// TT-95 regression guard at the production call-site shape: `run_pipeline`
    /// uses [`SubjectWorkBatch::absorb`], which always writes into the same
    /// `face_ids` buffer later passed to `update_edges_incremental`. A throwaway
    /// vec at either Phase B arm *or* the outer absorb call site fails this.
    #[tokio::test]
    async fn subject_work_batch_absorb_face_only_collects_ids_for_incremental() {
        let db = TestDb::init().await;
        let pool = db.pool();
        let image_id = seed_image(pool).await;
        let qid = seed_subject_queue(pool, image_id).await;
        let index = empty_index();

        let (ftx, frx) = tokio::sync::oneshot::channel();
        ftx.send(Ok(vec![dummy_face(1.0), dummy_face(2.0)]))
            .expect("oneshot send");

        let mut subject_batch = SubjectWorkBatch::new();
        subject_batch
            .absorb(
                pool,
                &index,
                Pending {
                    image_id,
                    sem_entry: None, // no semantic work — face-only path
                    sub_entry: Some((qid, 0)),
                    erx: None,
                    frx: Some(frx),
                },
                "buffalo_s_recognition",
            )
            .await;

        assert!(
            subject_batch.processed,
            "face-only arm must report subject work"
        );
        assert_eq!(
            subject_batch.face_ids.len(),
            2,
            "SubjectWorkBatch.face_ids is what update_edges_incremental receives (TT-95)"
        );
        assert_eq!(queue_row_count(pool, qid).await, 0);
    }

    #[tokio::test]
    async fn handle_face_result_ok_collects_ids() {
        let db = TestDb::init().await;
        let pool = db.pool();
        let image_id = seed_image(pool).await;
        let qid = seed_subject_queue(pool, image_id).await;
        let mut batch = Vec::new();
        let attempted = handle_face_result(
            pool,
            image_id,
            Some((qid, 0)),
            "buffalo_s_recognition",
            Ok(Ok(vec![dummy_face(1.0)])),
            &mut batch,
        )
        .await;
        assert!(attempted);
        assert_eq!(batch.len(), 1);
        assert_eq!(
            queue_row_count(pool, qid).await,
            0,
            "success path must consume the subject queue entry"
        );
    }

    #[tokio::test]
    async fn handle_face_result_analysis_err_records_queue_failure() {
        let db = TestDb::init().await;
        let pool = db.pool();
        let image_id = seed_image(pool).await;
        let qid = seed_subject_queue(pool, image_id).await;
        let mut batch = Vec::new();
        let attempted = handle_face_result(
            pool,
            image_id,
            Some((qid, 0)),
            "buffalo_s_recognition",
            Ok(Err(anyhow::anyhow!("detector failed"))),
            &mut batch,
        )
        .await;
        assert!(!attempted);
        assert!(batch.is_empty());
        let (attempts, err): (i32, String) =
            sqlx::query_as("SELECT attempts, last_error FROM embedding_queue WHERE id = ?")
                .bind(qid)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(attempts, 1);
        assert!(err.contains("detector failed"));
    }

    #[tokio::test]
    async fn handle_face_result_recv_dropped_records_queue_failure() {
        let db = TestDb::init().await;
        let pool = db.pool();
        let image_id = seed_image(pool).await;
        let qid = seed_subject_queue(pool, image_id).await;
        let (ftx, frx) =
            tokio::sync::oneshot::channel::<anyhow::Result<Vec<face_actor::FaceResult>>>();
        drop(ftx);
        let mut batch = Vec::new();
        let attempted = handle_face_result(
            pool,
            image_id,
            Some((qid, 0)),
            "buffalo_s_recognition",
            frx.await,
            &mut batch,
        )
        .await;
        assert!(!attempted);
        assert!(batch.is_empty());
        let (attempts, err): (i32, String) =
            sqlx::query_as("SELECT attempts, last_error FROM embedding_queue WHERE id = ?")
                .bind(qid)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(attempts, 1);
        assert!(err.contains("face reply channel dropped"));
    }

    /// Regression guard for TT-94: `dispatch_batch` must enqueue *every* image's
    /// embed and face request before returning, without awaiting any reply.
    /// Under the old Phase-B dispatch, only image 1's face request would be sent
    /// until its reply came back — here no reply is ever produced, so a
    /// per-image-gated dispatch would enqueue at most one face request.
    #[tokio::test]
    async fn dispatch_batch_enqueues_all_requests_without_awaiting_replies() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();

        // Depth (4) >= batch (3), mirroring prod's infer_channel_depth >= batch_size.
        // We keep the receivers so nothing consumes/replies during dispatch.
        let (embed_tx, mut embed_rx) = tokio::sync::mpsc::channel::<embed_actor::EmbedRequest>(4);
        let (face_tx, mut face_rx) = tokio::sync::mpsc::channel::<face_actor::FaceRequest>(4);

        let mk = |id: i64| DecodedImage {
            image_id: id,
            full: Arc::new(image::DynamicImage::new_rgb8(1, 1)),
        };
        // Three images, each with both a semantic and a subject queue slot.
        let decoded = vec![
            (1i64, Some((10, 0)), Some((20, 0)), mk(1)),
            (2i64, Some((11, 0)), Some((21, 0)), mk(2)),
            (3i64, Some((12, 0)), Some((22, 0)), mk(3)),
        ];

        // Bounded so a regression that awaits a reply inside dispatch_batch
        // fails fast with a clear panic instead of hanging the test / CI job.
        let pending = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            dispatch_batch(&pool, decoded, &embed_tx, &face_tx),
        )
        .await
        .expect("dispatch_batch must return without awaiting any reply");

        assert_eq!(pending.len(), 3);
        assert!(
            pending.iter().all(|p| p.erx.is_some() && p.frx.is_some()),
            "every image should hold both reply receivers"
        );

        // Both channels received all three requests up front.
        let mut embed_count = 0;
        while embed_rx.try_recv().is_ok() {
            embed_count += 1;
        }
        let mut face_count = 0;
        while face_rx.try_recv().is_ok() {
            face_count += 1;
        }
        assert_eq!(embed_count, 3, "all embed requests dispatched in Phase A");
        assert_eq!(face_count, 3, "all face requests dispatched in Phase A");
    }
}
