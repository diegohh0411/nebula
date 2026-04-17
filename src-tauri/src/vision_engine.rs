use ort::session::Session;
use std::sync::Arc;
use std::path::PathBuf;

pub struct VisionEngine {
    data_dir: PathBuf,
    image_session: Arc<std::sync::Mutex<Option<Session>>>,
    text_session: Arc<std::sync::Mutex<Option<Session>>>,
    tokenizer: Arc<std::sync::Mutex<Option<tokenizers::Tokenizer>>>,
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            image_session: Arc::new(std::sync::Mutex::new(None)),
            text_session: Arc::new(std::sync::Mutex::new(None)),
            tokenizer: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}
