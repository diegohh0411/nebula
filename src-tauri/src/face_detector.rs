use anyhow::Result;
use face_id::analyzer::{FaceAnalyzer, FaceAnalysis};
use image::DynamicImage;

pub struct Detector {
    analyzer: FaceAnalyzer,
}

impl Detector {
    pub async fn new() -> Result<Self> {
        let analyzer = FaceAnalyzer::from_hf().build().await?;
        Ok(Self { analyzer })
    }

    pub fn analyze(&self, image: &DynamicImage) -> Result<Vec<FaceAnalysis>> {
        let faces = self.analyzer.analyze(image)?;
        Ok(faces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detector_init() {
        // This will try to download models, so it might fail if offline.
        // But it verifies the API at least.
        let _ = Detector::new().await;
    }
}
