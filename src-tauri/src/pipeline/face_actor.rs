use crate::pipeline::DecodedImage;
use face_id::analyzer::FaceAnalyzer;
use face_id::detector::DetectedFace;
use std::sync::Arc;
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
            let _ = req.reply.send(res);
        }
    });
    tx
}
