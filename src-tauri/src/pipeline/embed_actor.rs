use crate::models::{registry::ModelSpec, ModelManager};
use crate::pipeline::DecodedImage;
use crate::vision::engine::VisionEngine;
use log::{debug, warn};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

pub struct EmbedRequest {
    pub decoded: DecodedImage,
    pub reply: oneshot::Sender<anyhow::Result<Vec<f32>>>,
}

pub fn spawn_embed_actor(
    engine: Arc<VisionEngine>,
    manager: Arc<ModelManager>,
    spec: &'static ModelSpec,
    batch_size: usize,
    channel_depth: usize,
) -> mpsc::Sender<EmbedRequest> {
    let (tx, mut rx) = mpsc::channel::<EmbedRequest>(channel_depth);
    tokio::spawn(async move {
        loop {
            let first = match rx.recv().await {
                Some(r) => r,
                None => break,
            };
            let mut batch = vec![first];
            while batch.len() < batch_size {
                match rx.try_recv() {
                    Ok(r) => batch.push(r),
                    Err(_) => break,
                }
            }

            let imgs: Vec<Arc<image::DynamicImage>> =
                batch.iter().map(|r| r.decoded.full.clone()).collect();
            let engine_c = engine.clone();
            let manager_c = manager.clone();

            // Heartbeat around the blocking ONNX call: if inference hangs (a real
            // DirectML failure mode on Windows), the "running" line is the last
            // thing logged and pinpoints the stall to embedding.
            let n = imgs.len();
            let ids: Vec<i64> = batch.iter().map(|r| r.decoded.image_id).collect();
            debug!("[embed] running inference on {n} image(s): {ids:?}");
            let started = Instant::now();
            let results = tokio::task::spawn_blocking(move || {
                let refs: Vec<&image::DynamicImage> = imgs.iter().map(|a| a.as_ref()).collect();
                engine_c.embed_images_batch(manager_c.as_ref(), &refs, spec)
            })
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("embed task panicked: {e}")));

            match results {
                Ok(vecs) => {
                    debug!(
                        "[embed] inference done for {n} image(s) in {:.2}s",
                        started.elapsed().as_secs_f32()
                    );
                    for (req, v) in batch.into_iter().zip(vecs) {
                        let _ = req.reply.send(Ok(v));
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    warn!(
                        "[embed] inference failed for {n} image(s) in {:.2}s: {msg}",
                        started.elapsed().as_secs_f32()
                    );
                    for req in batch {
                        let _ = req.reply.send(Err(anyhow::anyhow!(msg.clone())));
                    }
                }
            }
        }
    });
    tx
}
