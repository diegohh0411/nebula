//! Bounded, low-priority content-hash worker (TT-75).
//!
//! Change-detection authority is `(file_size, mtime)`; the content hash is only
//! a tie-breaker. We use BLAKE3 (non-cryptographic strength is sufficient and it
//! is ~5–10× cheaper than SHA256) and compute it off the import critical path.

use anyhow::Result;
use std::path::Path;

/// Compute the BLAKE3 hex digest of a file's contents on a blocking thread.
pub async fn compute_blake3(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(hasher.finalize().to_hex().to_string())
    })
    .await??;
    Ok(hash)
}
