use crate::pipeline::DecodedImage;
use face_id::analyzer::FaceAnalyzer;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub struct FaceRequest {
    pub decoded: DecodedImage,
    pub reply: oneshot::Sender<anyhow::Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>>>,
}

pub fn spawn_face_actor(analyzer: Arc<FaceAnalyzer>, channel_depth: usize) -> mpsc::Sender<FaceRequest> {
    let (tx, mut rx) = mpsc::channel::<FaceRequest>(channel_depth);
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let analyzer_c = analyzer.clone();
            let img = req.decoded.full.clone();
            let res = tokio::task::spawn_blocking(move || {
                analyzer_c
                    .analyze(img.as_ref())
                    .map(|faces| {
                        faces.into_iter()
                            .map(|f| (f.detection.bbox, f.embedding))
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
