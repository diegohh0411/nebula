use anyhow::Result;
use sqlx::SqlitePool;
use tauri::AppHandle;

use tauri::Emitter;

/// Encode a Vec<f32> to raw little-endian bytes for storage as BLOB.
pub fn f32_slice_to_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Decode raw little-endian bytes back to a Vec<f32>.
pub fn bytes_to_f32_vec(bytes: &[u8]) -> Result<Vec<f32>> {
    anyhow::ensure!(
        bytes.len().is_multiple_of(4),
        "invalid embedding byte length: expected a multiple of 4, got {}",
        bytes.len()
    );

    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| {
            f32::from_le_bytes(
                chunk
                    .try_into()
                    .expect("chunks_exact(4) must yield chunks of exactly 4 bytes"),
            )
        })
        .collect())
}

pub(crate) async fn emit_progress(pool: &SqlitePool, app: &AppHandle) {
    use tauri::Manager;
    let images_per_sec = {
        let state = app.state::<crate::AppState>();
        f32::from_bits(
            state
                .throughput_ema
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    };
    if let Ok(status) = crate::pipeline::queue::get_processing_counts(pool).await {
        let _ = app.emit(
            "pipeline_stats",
            crate::models::PipelineStatsPayload {
                total_pending: status.total_pending as u32,
                images_per_sec,
            },
        );
    }
}
