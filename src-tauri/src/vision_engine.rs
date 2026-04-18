use anyhow::Result;
use ndarray::Array4;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use std::path::PathBuf;

const SIGLIP_REPO: &str = "google/siglip-so400m-patch14-384";

pub struct VisionEngine {
    pub data_dir: PathBuf,
    image_session: std::sync::Mutex<Option<Session>>,
    text_session: std::sync::Mutex<Option<Session>>,
    tokenizer: std::sync::Mutex<Option<tokenizers::Tokenizer>>,
    face_analyzer: tokio::sync::OnceCell<face_id::analyzer::FaceAnalyzer>,
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            image_session: std::sync::Mutex::new(None),
            text_session: std::sync::Mutex::new(None),
            tokenizer: std::sync::Mutex::new(None),
            face_analyzer: tokio::sync::OnceCell::new(),
        }
    }

    pub async fn get_face_analyzer(&self) -> Result<&face_id::analyzer::FaceAnalyzer> {
        self.face_analyzer
            .get_or_try_init(|| async {
                face_id::analyzer::FaceAnalyzer::from_hf()
                    .build()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))
            })
            .await
    }

    fn load_session<'a>(
        &'a self,
        filename: &str,
        session_mutex: &'a std::sync::Mutex<Option<Session>>,
    ) -> Result<std::sync::MutexGuard<'a, Option<Session>>> {
        let mut lock = session_mutex
            .lock()
            .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;
        if lock.is_none() {
            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] loading session for '{}' from HF repo '{}'", filename, SIGLIP_REPO);

            let api = hf_hub::api::sync::Api::new()?;
            let repo = api.model(SIGLIP_REPO.to_string());

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] fetching '{}' from HuggingFace Hub…", filename);

            let model_path = repo.get(filename)
                .map_err(|e| anyhow::anyhow!("HuggingFace download of '{}' from repo '{}' failed: {}", filename, SIGLIP_REPO, e))?;

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] model cached at: {}", model_path.display());

            let session = Session::builder()
                .map_err(|e| anyhow::anyhow!("failed to create session builder: {e}"))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow::anyhow!("failed to set optimization level: {e}"))?
                .with_intra_threads(4)
                .map_err(|e| anyhow::anyhow!("failed to set intra threads: {e}"))?
                .commit_from_file(&model_path)
                .map_err(|e| anyhow::anyhow!("failed to load ONNX model '{}': {e}", model_path.display()))?;

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] session ready for '{}'", filename);

            *lock = Some(session);
        }
        Ok(lock)
    }

    pub fn get_image_session(&self) -> Result<std::sync::MutexGuard<'_, Option<Session>>> {
        self.load_session("model.onnx", &self.image_session)
    }

    pub fn get_text_session(&self) -> Result<std::sync::MutexGuard<'_, Option<Session>>> {
        self.load_session("text_model.onnx", &self.text_session)
    }

    pub fn embed_image(&self, img: &image::DynamicImage) -> Result<Vec<f32>> {
        let mut lock = self.get_image_session()?;
        let session = lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("image session not initialized"))?;

        // Preprocess: resize to 384x384 for so400m model
        let resized = img.resize_exact(384, 384, image::imageops::FilterType::Lanczos3);
        let rgb = resized.to_rgb8();

        let mut input = Array4::<f32>::zeros((1, 3, 384, 384));
        for (x, y, pixel) in rgb.enumerate_pixels() {
            for c in 0..3 {
                // Normalization: (x - mean) / std with mean=0.5, std=0.5
                let val = pixel[c] as f32 / 255.0;
                input[[0, c, y as usize, x as usize]] = (val - 0.5) / 0.5;
            }
        }

        let tensor = TensorRef::from_array_view(input.view())
            .map_err(|e| anyhow::anyhow!("failed to create tensor: {e}"))?;
        let outputs = session
            .run(ort::inputs!["pixel_values" => tensor])
            .map_err(|e| anyhow::anyhow!("image inference failed: {e}"))?;
        let (_shape, data) = outputs["image_embeds"]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("failed to extract image embedding: {e}"))?;
        Ok(data.to_vec())
    }

    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        const MAX_SEQ_LEN: usize = 64;

        // 1. Load tokenizer lazily and encode in one lock acquisition
        let encoding = {
            let mut tok_lock = self
                .tokenizer
                .lock()
                .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;
            if tok_lock.is_none() {
                let api = hf_hub::api::sync::Api::new()?;
                let repo = api.model(SIGLIP_REPO.to_string());
                let tok_path = repo.get("tokenizer.json")?;
                *tok_lock = Some(
                    tokenizers::Tokenizer::from_file(tok_path)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                );
            }
            tok_lock
                .as_ref()
                .unwrap()
                .encode(text, true)
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }; // tokenizer lock released

        // 2. Build input tensors, truncating to MAX_SEQ_LEN
        let input_ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .take(MAX_SEQ_LEN)
            .map(|&id| id as i64)
            .collect();
        let seq_len = input_ids.len();
        let input_ids_tensor =
            ndarray::Array2::from_shape_vec((1, seq_len), input_ids)?;

        let attention_mask: Vec<i64> = vec![1i64; seq_len];
        let attention_mask_arr =
            ndarray::Array2::from_shape_vec((1, seq_len), attention_mask)?;

        // 3. Run text model
        let mut session_lock = self.get_text_session()?;
        let session = session_lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("text session not initialized"))?;

        let ids_tensor_ref =
            ort::value::TensorRef::from_array_view(input_ids_tensor.view())
                .map_err(|e| anyhow::anyhow!("failed to create input_ids tensor: {e}"))?;
        let mask_ref =
            ort::value::TensorRef::from_array_view(attention_mask_arr.view())
                .map_err(|e| anyhow::anyhow!("failed to create attention_mask tensor: {e}"))?;

        let outputs = session
            .run(ort::inputs!["input_ids" => ids_tensor_ref, "attention_mask" => mask_ref])
            .map_err(|e| anyhow::anyhow!("text inference failed: {e}"))?;

        let (_shape, data) = outputs["text_embeds"]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("failed to extract text embedding: {e}"))?;

        Ok(data.to_vec())
    }
}
