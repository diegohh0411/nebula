use std::path::PathBuf;
use std::sync::Arc;

pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub data_dir: PathBuf,
    pub indexer: Arc<crate::library::indexer::Indexer>,
    pub vision_engine: Arc<crate::vision::engine::VisionEngine>,
    pub model_manager: Arc<crate::models::ModelManager>,
    pub index: crate::search::vector_index::IndexStore,
    pub preview: crate::media::preview::PreviewHandle,
    pub throughput_ema: std::sync::atomic::AtomicU32,
    /// Signals background tasks (e.g. throughput sampler) to shut down cleanly.
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
}
