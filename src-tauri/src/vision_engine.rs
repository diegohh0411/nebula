use anyhow::Result;
use ndarray::Array4;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use std::path::PathBuf;

pub struct VisionEngine {
    pub data_dir: PathBuf,
    image_session: std::sync::Mutex<Option<Session>>,
    text_session: std::sync::Mutex<Option<Session>>,
    tokenizer: std::sync::Mutex<Option<tokenizers::Tokenizer>>,
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            image_session: std::sync::Mutex::new(None),
            text_session: std::sync::Mutex::new(None),
            tokenizer: std::sync::Mutex::new(None),
        }
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
            let api = hf_hub::api::sync::Api::new()?;
            let repo = api.model("google/siglip-so400m-patch14-384".to_string());
            let model_path = repo.get(filename)?;

            let session = Session::builder()
                .map_err(|e| anyhow::anyhow!("failed to create session builder: {e}"))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow::anyhow!("failed to set optimization level: {e}"))?
                .with_intra_threads(4)
                .map_err(|e| anyhow::anyhow!("failed to set intra threads: {e}"))?
                .commit_from_file(model_path)
                .map_err(|e| anyhow::anyhow!("failed to load model from file: {e}"))?;

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
        let session = lock.as_mut().unwrap();

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
            .run(ort::inputs![tensor])
            .map_err(|e| anyhow::anyhow!("inference failed: {e}"))?;
        let (_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("failed to extract output tensor: {e}"))?;
        Ok(data.to_vec())
    }
}
