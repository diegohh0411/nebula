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
use crate::pipeline::ComputePlacement;

pub struct VisionEngine {
    pub data_dir: PathBuf,
    pub placement: ComputePlacement,
    vision_session: std::sync::Mutex<Option<(String, Session)>>,
    text_session: std::sync::Mutex<Option<(String, Session)>>,
    tokenizer: std::sync::Mutex<Option<(String, tokenizers::Tokenizer)>>,
    face_analyzer: std::sync::Mutex<Option<(String, Arc<FaceAnalyzer>)>>,
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            placement: ComputePlacement::Cpu,
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

        #[cfg(feature = "directml")]
        let dml_eps: Vec<ort::ep::ExecutionProviderDispatch> = if self.placement == ComputePlacement::Gpu {
            vec![ort::ep::DirectML::default().build()]
        } else {
            vec![]
        };
        #[cfg(not(feature = "directml"))]
        let dml_eps: Vec<ort::ep::ExecutionProviderDispatch> = vec![];

        let analyzer = FaceAnalyzer::builder(det_path, rec_path, gender_age_path)
            .detector_input_size(preset.detector_input_size)
            .with_execution_providers(&dml_eps)
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

    fn load_session(path: &std::path::Path, placement: ComputePlacement) -> anyhow::Result<Session> {
        let mut builder = Session::builder()
            .map_err(|e| anyhow!("failed to create session builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!("failed to set optimization level: {e}"))?
            .with_intra_threads(num_cpus::get_physical())
            .map_err(|e| anyhow!("failed to set intra threads: {e}"))?;

        #[cfg(feature = "directml")]
        if placement == ComputePlacement::Gpu {
            builder = builder
                .with_execution_providers([ort::ep::DirectML::default().build()])
                .map_err(|e| anyhow!("failed to register DirectML EP: {e}"))?;
        }

        #[cfg(not(feature = "directml"))]
        let _ = placement;

        builder
            .commit_from_file(path)
            .map_err(|e| anyhow!("failed to load ONNX model '{}': {e}", path.display()))
    }

    pub fn embed_images_batch(
        &self,
        manager: &ModelManager,
        imgs: &[&image::DynamicImage],
        spec: &ModelSpec,
    ) -> Result<Vec<Vec<f32>>> {
        if imgs.is_empty() {
            return Ok(vec![]);
        }
        let size = spec.image_size;
        let n = imgs.len();
        let mut pixel_values = ndarray::Array4::<f32>::zeros((n, 3, size, size));
        for (b, img) in imgs.iter().enumerate() {
            crate::preprocess::fill_pixel_values(&mut pixel_values, b, img, size);
        }

        let vision_file = spec.vision_file.as_ref()
            .ok_or_else(|| anyhow!("model '{}' has no vision tower configured", spec.id))?;
        let path = manager.model_file_path(spec, vision_file);

        let mut lock = self.vision_session.lock().map_err(|e| anyhow!("mutex poisoned: {e}"))?;
        let needs_load = match &*lock { Some((id, _)) => id != spec.id, None => true };
        if needs_load {
            *lock = Some((spec.id.to_string(), Self::load_session(&path, self.placement)?));
        }
        let (_, session) = lock.as_mut().unwrap();

        let pv_ref = TensorRef::from_array_view(pixel_values.view())
            .map_err(|e| anyhow!("failed to create pixel_values tensor: {e}"))?;
        let outputs = session
            .run(ort::inputs![spec.vision_input => pv_ref])
            .map_err(|e| anyhow!("batched image inference failed: {e}"))?;
        let (shape, data) = outputs[spec.vision_output]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("failed to extract batched image embeddings: {e}"))?;

        let dim = data.len() / n;
        anyhow::ensure!(
            shape.first().copied() == Some(n as i64) || data.len() % n == 0,
            "unexpected batch output shape {:?} for n={}", shape, n
        );
        Ok((0..n).map(|i| data[i * dim..(i + 1) * dim].to_vec()).collect())
    }

    pub fn embed_image(&self, manager: &ModelManager, img: &image::DynamicImage, spec: &ModelSpec) -> Result<Vec<f32>> {
        let mut out = self.embed_images_batch(manager, &[img], spec)?;
        out.pop().ok_or_else(|| anyhow!("empty batch result"))
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
            *lock = Some((spec.id.to_string(), Self::load_session(&path, self.placement)?));
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

    #[test]
    fn batched_embeddings_match_single_when_model_present() {
        let data_dir = match std::env::var("NEBULA_TEST_DATA_DIR") {
            Ok(d) => std::path::PathBuf::from(d),
            Err(_) => { eprintln!("skipping"); return; }
        };
        let manager = crate::models::ModelManager::new(data_dir.clone());
        let spec = &crate::models::registry::SIGLIP_BASE;
        let vf = spec.vision_file.as_ref().unwrap();
        if !manager.model_file_path(spec, vf).exists() { eprintln!("skipping"); return; }
        let engine = VisionEngine::new(data_dir);

        let a = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb([200,40,40])));
        let b = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb([40,40,200])));
        let single_a = engine.embed_image(&manager, &a, spec).unwrap();
        let batch = engine.embed_images_batch(&manager, &[&a, &b], spec).unwrap();
        assert_eq!(batch.len(), 2);
        for (x, y) in single_a.iter().zip(batch[0].iter()) {
            assert!((x - y).abs() < 1e-3, "batched vs single mismatch: {x} vs {y}");
        }
    }
}
