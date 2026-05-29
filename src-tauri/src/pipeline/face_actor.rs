use crate::pipeline::DecodedImage;
use face_id::analyzer::FaceAnalyzer;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub struct FaceRequest {
    pub decoded: DecodedImage,
    pub reply: oneshot::Sender<anyhow::Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>>>,
}

pub fn spawn_face_actor(analyzer: Arc<FaceAnalyzer>) -> mpsc::Sender<FaceRequest> {
    let (tx, mut rx) = mpsc::channel::<FaceRequest>(8);
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let analyzer_c = analyzer.clone();
            let img = req.decoded.full.clone();
            let res = tokio::task::block_in_place(|| {
                analyzer_c
                    .analyze(img.as_ref())
                    .map(|faces| {
                        faces.into_iter()
                            .map(|f| (f.detection.bbox, f.embedding))
                            .collect::<Vec<_>>()
                    })
                    .map_err(|e| anyhow::anyhow!("{}", e))
            });
            let _ = req.reply.send(res);
        }
    });
    tx
}
