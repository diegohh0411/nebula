use ort::session::Session;
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
}
