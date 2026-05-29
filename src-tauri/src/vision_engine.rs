use anyhow::{Result, anyhow};
use face_id::analyzer::FaceAnalyzer;
use ndarray::{Array2, Array4};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use std::path::PathBuf;
use std::sync::Arc;

use crate::models::manager::ModelManager;
use crate::models::registry::{ModelSpec, FaceIdPreset};

pub struct VisionEngine {
    pub data_dir: PathBuf,
    vision_session: std::sync::Mutex<Option<(String, Session)>>,
    text_session: std::sync::Mutex<Option<(String, Session)>>,
    tokenizer: std::sync::Mutex<Option<(String, tokenizers::Tokenizer)>>,
    face_analyzer: std::sync::Mutex<Option<(String, Arc<FaceAnalyzer>)>>,
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            vision_session: std::sync::Mutex::new(None),
            text_session: std::sync::Mutex::new(None),
            tokenizer: std::sync::Mutex::new(None),
            face_analyzer: std::sync::Mutex::new(None),
        }
    }

    pub async fn get_face_analyzer(
        &self,
        manager: &ModelManager,
        preset: &FaceIdPreset
    ) -> Result<Arc<FaceAnalyzer>> {
        {
            let guard = self.face_analyzer.lock().map_err(|e| anyhow!("face analyzer mutex poisoned: {e}"))?;
            if let Some((current_id, analyzer)) = guard.as_ref() {
                if current_id == preset.id {
                    return Ok(Arc::clone(analyzer));
                }
            }
        }

        let det_path = manager.onnx_path(preset.detector);
        let rec_path = manager.onnx_path(preset.embedder);
        let gender_age_path = manager.onnx_path(preset.gender_age);

        let analyzer = FaceAnalyzer::builder(det_path, rec_path, gender_age_path)
            .detector_input_size(preset.detector_input_size)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))?;

        let analyzer = Arc::new(analyzer);
        {
            let mut guard = self.face_analyzer.lock().map_err(|e| anyhow!("face analyzer mutex poisoned: {e}"))?;
            *guard = Some((preset.id.to_string(), Arc::clone(&analyzer)));
        }
        Ok(analyzer)
    }

    pub fn analyze_faces(
        analyzer: &FaceAnalyzer,
        img: &image::DynamicImage,
        _preset: &FaceIdPreset,
    ) -> Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>> {
        Self::analyze_faces_full(analyzer, img)
    }

    fn analyze_faces_full(
        analyzer: &face_id::analyzer::FaceAnalyzer,
        img: &image::DynamicImage,
    ) -> Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>> {
        let faces = analyzer.analyze(img)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(faces.into_iter().map(|f| (f.detection.bbox, f.embedding)).collect())
    }

    fn load_session(path: &std::path::Path) -> anyhow::Result<Session> {
        Session::builder()
            .map_err(|e| anyhow!("failed to create session builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!("failed to set optimization level: {e}"))?
            .with_intra_threads(num_cpus::get_physical())
            .map_err(|e| anyhow!("failed to set intra threads: {e}"))?
            .commit_from_file(path)
            .map_err(|e| anyhow!("failed to load ONNX model '{}': {e}", path.display()))
    }

    pub fn embed_image(&self, manager: &ModelManager, img: &image::DynamicImage, spec: &ModelSpec) -> Result<Vec<f32>> {
        let size = spec.image_size;
        let mut pixel_values = Array4::<f32>::zeros((1, 3, size, size));
        crate::preprocess::fill_pixel_values(&mut pixel_values, 0, img, size);

        let vision_file = spec.vision_file.as_ref()
            .ok_or_else(|| anyhow!("model '{}' has no vision tower configured", spec.id))?;
        let path = manager.model_file_path(spec, vision_file);

        let mut lock = self.vision_session.lock().map_err(|e| anyhow!("mutex poisoned: {e}"))?;
        let needs_load = match &*lock { Some((id, _)) => id != spec.id, None => true };
        if needs_load {
            *lock = Some((spec.id.to_string(), Self::load_session(&path)?));
        }
        let (_, session) = lock.as_mut().unwrap();

        let pv_ref = TensorRef::from_array_view(pixel_values.view())
            .map_err(|e| anyhow!("failed to create pixel_values tensor: {e}"))?;
        let outputs = session
            .run(ort::inputs![spec.vision_input => pv_ref])
            .map_err(|e| anyhow!("image inference failed: {e}"))?;
        let (_shape, data) = outputs[spec.vision_output]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("failed to extract image embedding: {e}"))?;
        Ok(data.to_vec())
    }

    pub fn embed_text(&self, manager: &ModelManager, text: &str, spec: &ModelSpec) -> Result<Vec<f32>> {
        const MAX_SEQ_LEN: usize = 64;

        let encoding = {
            let mut tok_lock = self.tokenizer.lock()
                .map_err(|e| anyhow!("mutex poisoned: {e}"))?;

            let needs_load = match &*tok_lock {
                Some((current_id, _)) => current_id != spec.id,
                None => true,
            };

            if needs_load {
                let tok_path = manager.tokenizer_path(spec)
                    .ok_or_else(|| anyhow!("Model has no tokenizer"))?;
                *tok_lock = Some((
                    spec.id.to_string(),
                    tokenizers::Tokenizer::from_file(tok_path)
                        .map_err(|e| anyhow!("{e}"))?,
                ));
            }
            tok_lock.as_ref().unwrap().1
                .encode(text, true)
                .map_err(|e| anyhow!("{e}"))?
        };

        let input_ids: Vec<i64> = encoding.get_ids().iter()
            .take(MAX_SEQ_LEN)
            .map(|&id| id as i64)
            .collect();
        let seq_len = input_ids.len();
        let input_ids_arr = Array2::from_shape_vec((1, seq_len), input_ids)?;

        let text_file = spec.text_file.as_ref()
            .ok_or_else(|| anyhow!("model '{}' has no text tower configured", spec.id))?;
        let path = manager.model_file_path(spec, text_file);

        let mut lock = self.text_session.lock().map_err(|e| anyhow!("mutex poisoned: {e}"))?;
        let needs_load = match &*lock { Some((id, _)) => id != spec.id, None => true };
        if needs_load {
            *lock = Some((spec.id.to_string(), Self::load_session(&path)?));
        }
        let (_, session) = lock.as_mut().unwrap();

        let ids_ref = TensorRef::from_array_view(input_ids_arr.view())
            .map_err(|e| anyhow!("failed to create input_ids tensor: {e}"))?;
        let outputs = session
            .run(ort::inputs![spec.text_input => ids_ref])
            .map_err(|e| anyhow!("text inference failed: {e}"))?;
        let (_shape, data) = outputs[spec.text_output]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("failed to extract text embedding: {e}"))?;
        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_image_returns_expected_dim_when_model_present() {
        let data_dir = match std::env::var("NEBULA_TEST_DATA_DIR") {
            Ok(d) => std::path::PathBuf::from(d),
            Err(_) => { eprintln!("skipping: NEBULA_TEST_DATA_DIR not set"); return; }
        };
        let manager = crate::models::ModelManager::new(data_dir.clone());
        let spec = &crate::models::registry::SIGLIP_BASE;
        let vf = spec.vision_file.as_ref().unwrap();
        if !manager.model_file_path(spec, vf).exists() {
            eprintln!("skipping: vision model not downloaded");
            return;
        }
        let engine = VisionEngine::new(data_dir);
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(64, 64));
        let emb = engine.embed_image(&manager, &img, spec).unwrap();
        assert_eq!(emb.len(), 768, "SigLIP base image embedding dim");
    }
}
