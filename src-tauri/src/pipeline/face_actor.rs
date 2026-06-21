use crate::pipeline::DecodedImage;
use face_id::analyzer::FaceAnalyzer;
use face_id::detector::DetectedFace;
use log::{debug, warn};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

/// Per detected face: full detection (bbox + landmarks + score), its embedding,
/// and the sharpness (0..1) measured on the cropped face region.
pub type FaceResult = (DetectedFace, Vec<f32>, f32);

pub struct FaceRequest {
    pub decoded: DecodedImage,
    pub reply: oneshot::Sender<anyhow::Result<Vec<FaceResult>>>,
}

pub fn spawn_face_actor(
    analyzer: Arc<FaceAnalyzer>,
    channel_depth: usize,
) -> mpsc::Sender<FaceRequest> {
    let (tx, mut rx) = mpsc::channel::<FaceRequest>(channel_depth);
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let analyzer_c = analyzer.clone();
            let img = req.decoded.full.clone();
            // Heartbeat around the blocking ONNX call: a hung face-analysis run
            // (DirectML stall) leaves "analyzing" as the last line, pinpointing
            // the stall to face detection/recognition for this image.
            let image_id = req.decoded.image_id;
            debug!("[face] analyzing image {image_id}");
            let started = Instant::now();
            let res = tokio::task::spawn_blocking(move || {
                analyzer_c
                    .analyze(img.as_ref())
                    .map(|faces| {
                        faces
                            .into_iter()
                            .map(|f| {
                                // bbox coords are relative (0..1); crop the region to measure sharpness.
                                let (iw, ih) = (img.width() as f32, img.height() as f32);
                                let x = (f.detection.bbox.x1 * iw).max(0.0) as u32;
                                let y = (f.detection.bbox.y1 * ih).max(0.0) as u32;
                                let w = ((f.detection.bbox.x2 - f.detection.bbox.x1) * iw).max(1.0)
                                    as u32;
                                let h = ((f.detection.bbox.y2 - f.detection.bbox.y1) * ih).max(1.0)
                                    as u32;
                                let region = img.crop_imm(x, y, w, h);
                                let sharp = crate::people::face_quality::sharpness(&region);
                                (f.detection, f.embedding, sharp)
                            })
                            .collect::<Vec<_>>()
                    })
                    .map_err(|e| anyhow::anyhow!("{}", e))
            })
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("face task panicked: {e}")));
            match &res {
                Ok(faces) => debug!(
                    "[face] image {image_id} done in {:.2}s ({} faces)",
                    started.elapsed().as_secs_f32(),
                    faces.len()
                ),
                Err(e) => warn!(
                    "[face] image {image_id} failed in {:.2}s: {e}",
                    started.elapsed().as_secs_f32()
                ),
            }
            let _ = req.reply.send(res);
        }
    });
    tx
}
