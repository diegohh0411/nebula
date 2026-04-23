use anyhow::{Result, anyhow};
use face_id::{analyzer::FaceAnalyzer, face_align::norm_crop};
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
    session: std::sync::Mutex<Option<(String, Session)>>,
    tokenizer: std::sync::Mutex<Option<(String, tokenizers::Tokenizer)>>,
    face_analyzer: std::sync::Mutex<Option<(String, Arc<FaceAnalyzer>)>>,
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            session: std::sync::Mutex::new(None),
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
            let guard = self.face_analyzer.lock().unwrap();
            if let Some((current_id, analyzer)) = guard.as_ref() {
                if current_id == preset.id {
                    return Ok(Arc::clone(analyzer));
                }
            }
        }

        let det_path = manager.onnx_path(preset.detector);
        let rec_path = manager.onnx_path(preset.embedder);

        let analyzer = FaceAnalyzer::builder(det_path, rec_path, "")
            .detector_input_size(preset.detector_input_size)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))?;

        let analyzer = Arc::new(analyzer);
        {
            let mut guard = self.face_analyzer.lock().unwrap();
            *guard = Some((preset.id.to_string(), Arc::clone(&analyzer)));
        }
        Ok(analyzer)
    }

    pub fn analyze_faces(
        analyzer: &FaceAnalyzer,
        img: &image::DynamicImage,
        _preset: &FaceIdPreset,
    ) -> Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>> {
        Self::analyze_faces_direct(analyzer, img)
    }

    fn analyze_faces_direct(
        analyzer: &face_id::analyzer::FaceAnalyzer,
        img: &image::DynamicImage,
    ) -> Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>> {
        let rgb_img = img.to_rgb8();

        let detections = {
            let mut detector = analyzer.detector.lock()
                .map_err(|e| anyhow::anyhow!("detector mutex poisoned: {e}"))?;
            detector.detect(img)?
        };

        if detections.is_empty() {
            return Ok(vec![]);
        }

        let embed_crops: Vec<_> = detections
            .iter()
            .map(|res| {
                let landmarks = res.landmarks.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("face missing landmarks for embedding"))?;
                let lms_array: [(f32, f32); 5] = landmarks
                    .iter()
                    .map(|&(x, y)| (x * rgb_img.width() as f32, y * rgb_img.height() as f32))
                    .collect::<Vec<_>>()
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("landmarks were not 5-point keypoints"))?;
                Ok(norm_crop(&rgb_img, &lms_array, 112))
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;

        let embeddings = {
            let mut embedder = analyzer.embedder.lock()
                .map_err(|e| anyhow::anyhow!("embedder mutex poisoned: {e}"))?;
            embedder.compute_embeddings_batch(&embed_crops)
                .map_err(|e| anyhow::anyhow!("batch embedding failed: {e}"))?
        };

        Ok(detections.into_iter().zip(embeddings).map(|(d, e)| (d.bbox, e)).collect())
    }

    fn analyze_faces_full(
        analyzer: &face_id::analyzer::FaceAnalyzer,
        img: &image::DynamicImage,
    ) -> Result<Vec<(face_id::detector::BoundingBox, Vec<f32>)>> {
        let faces = analyzer.analyze(img)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(faces.into_iter().map(|f| (f.detection.bbox, f.embedding)).collect())
    }

    fn get_session(&self, manager: &ModelManager, spec: &ModelSpec) -> Result<std::sync::MutexGuard<'_, Option<(String, Session)>>> {
        let mut lock = self
            .session
            .lock()
            .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;

        let needs_load = match &*lock {
            Some((current_id, _)) => current_id != spec.id,
            None => true,
        };

        if needs_load {
            let model_path = manager.onnx_path(spec);

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] loading session from: {}", model_path.display());

            let session = Session::builder()
                .map_err(|e| anyhow::anyhow!("failed to create session builder: {e}"))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow::anyhow!("failed to set optimization level: {e}"))?
                .with_intra_threads(4)
                .map_err(|e| anyhow::anyhow!("failed to set intra threads: {e}"))?
                .commit_from_file(&model_path)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to load ONNX model '{}': {e}",
                        model_path.display()
                    )
                })?;

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] session ready");

            *lock = Some((spec.id.to_string(), session));
        }
        Ok(lock)
    }

    pub fn embed_image(&self, manager: &ModelManager, img: &image::DynamicImage, spec: &ModelSpec) -> Result<Vec<f32>> {
        let size = if spec.id.contains("256") { 256 } else { 224 };
        let resized = img.resize_exact(
            size as u32,
            size as u32,
            image::imageops::FilterType::Lanczos3,
        );
        let rgb = resized.to_rgb8();

        let mut pixel_values = Array4::<f32>::zeros((1, 3, size, size));
        for (x, y, pixel) in rgb.enumerate_pixels() {
            for c in 0..3 {
                let val = pixel[c] as f32 / 255.0;
                pixel_values[[0, c, y as usize, x as usize]] = (val - 0.5) / 0.5;
            }
        }

        // Dummy input_ids (batch=1, seq_len=1) — text encoder output is discarded
        let dummy_ids = Array2::<i64>::zeros((1, 1));

        let mut lock = self.get_session(manager, spec)?;
        let (_, session) = lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("session not initialized"))?;

        let pv_ref = TensorRef::from_array_view(pixel_values.view())
            .map_err(|e| anyhow::anyhow!("failed to create pixel_values tensor: {e}"))?;
        let ids_ref = TensorRef::from_array_view(dummy_ids.view())
            .map_err(|e| anyhow::anyhow!("failed to create dummy input_ids tensor: {e}"))?;

        let outputs = session
            .run(ort::inputs!["pixel_values" => pv_ref, "input_ids" => ids_ref])
            .map_err(|e| anyhow::anyhow!("image inference failed: {e}"))?;

        let (_shape, data) = outputs["image_embeds"]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("failed to extract image embedding: {e}"))?;
        Ok(data.to_vec())
    }

    pub fn embed_text(&self, manager: &ModelManager, text: &str, spec: &ModelSpec) -> Result<Vec<f32>> {
        const MAX_SEQ_LEN: usize = 64;

        let encoding = {
            let mut tok_lock = self
                .tokenizer
                .lock()
                .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;

            let needs_load = match &*tok_lock {
                Some((current_id, _)) => current_id != spec.id,
                None => true,
            };

            if needs_load {
                let tok_path = manager.tokenizer_path(spec).ok_or_else(|| anyhow!("Model has no tokenizer"))?;

                #[cfg(debug_assertions)]
                eprintln!(
                    "[vision-engine] loading tokenizer from: {}",
                    tok_path.display()
                );

                *tok_lock = Some((
                    spec.id.to_string(),
                    tokenizers::Tokenizer::from_file(tok_path)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                ));
            }
            tok_lock
                .as_ref()
                .unwrap()
                .1
                .encode(text, true)
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }; // tokenizer lock released before acquiring session lock

        let input_ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .take(MAX_SEQ_LEN)
            .map(|&id| id as i64)
            .collect();
        let seq_len = input_ids.len();
        let input_ids_arr = Array2::from_shape_vec((1, seq_len), input_ids)?;

        // Dummy pixel_values — image encoder output is discarded
        let size = if spec.id.contains("256") { 256 } else { 224 };
        let dummy_pixels = Array4::<f32>::zeros((1, 3, size, size));

        let mut lock = self.get_session(manager, spec)?;
        let (_, session) = lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("session not initialized"))?;

        let ids_ref = TensorRef::from_array_view(input_ids_arr.view())
            .map_err(|e| anyhow::anyhow!("failed to create input_ids tensor: {e}"))?;
        let pv_ref = TensorRef::from_array_view(dummy_pixels.view())
            .map_err(|e| anyhow::anyhow!("failed to create dummy pixel_values tensor: {e}"))?;

        let outputs = session
            .run(ort::inputs!["input_ids" => ids_ref, "pixel_values" => pv_ref])
            .map_err(|e| anyhow::anyhow!("text inference failed: {e}"))?;

        let (_shape, data) = outputs["text_embeds"]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("failed to extract text embedding: {e}"))?;
        Ok(data.to_vec())
    }
}
