use std::path::PathBuf;
use std::sync::Arc;

pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub indexer: Arc<crate::indexer::Indexer>,
    pub vision_engine: Arc<crate::vision_engine::VisionEngine>,
    pub model_manager: Arc<crate::models::ModelManager>,
    pub index: crate::vector_index::IndexStore,
    pub preview: crate::preview::PreviewHandle,
    pub throughput_ema: std::sync::atomic::AtomicU32,
}
