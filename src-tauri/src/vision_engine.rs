use anyhow::Result;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
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
}
