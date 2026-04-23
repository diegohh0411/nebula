use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Result;
use tauri::{AppHandle, Emitter};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use serde::{Serialize};

use crate::models::registry::ModelSpec;

#[derive(Serialize, Clone)]
pub struct ModelDownloadPayload {
  pub file: String,
  pub bytes_done: u64,
  pub bytes_total: Option<u64>,
  /// true on the final chunk for each file
  pub done: bool,
  /// non-empty if the download failed
  pub error: Option<String>,
}

pub struct ModelManager {
  data_dir: PathBuf,
  readiness: std::sync::Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
}

impl ModelManager {
  pub fn new(data_dir: PathBuf) -> Self {
    Self {
      data_dir,
      readiness: std::sync::Mutex::new(HashMap::new()),
    }
  }

  pub fn model_dir(&self, spec: &ModelSpec) -> PathBuf {
    self.data_dir.join("models").join(spec.cache_dir)
  }

  pub fn onnx_path(&self, spec: &ModelSpec) -> PathBuf {
    self.model_dir(spec).join(spec.model_file.filename)
  }

  pub fn tokenizer_path(&self, spec: &ModelSpec) -> Option<PathBuf> {
    spec.tokenizer_file.as_ref().map(|f| self.model_dir(spec).join(f.filename))
  }

  fn signal_ready(&self, model_id: &str) {
    let guard = self.readiness.lock().unwrap();
    if let Some(tx)= guard.get(model_id) {
      let _ = tx.send(true);
    }
  }

  pub async fn ensure_ready(&self, app: &AppHandle, spec: &ModelSpec) -> Result<()> {
    let dir = self.model_dir(spec);

    // 1. Ensure a readiness channel exists for this model
    {
      let mut guard = self.readiness.lock().unwrap();
      guard
        .entry(spec.id.to_string())
        .or_insert_with(|| tokio::sync::watch::channel(false).0);
    }

    // 2. Fast path — all files already on disk
    if spec.all_files().iter().all(|f| dir.join(f.filename).exists()) {
      self.signal_ready(spec.id);
      return Ok(());
    }

    // 3. Create the cache directory
    tokio::fs::create_dir_all(&dir).await?;

    // 4. Download each missing file
    let client = reqwest::Client::new();

    for file in spec.all_files() {
      let dest = dir.join(file.filename);
      if dest.exists() {
        continue;
      }

      let remote = file.remote_path.unwrap_or(file.filename);
      let url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        spec.hf_repo, remote
      );

      let resp = client
        .get(&url)
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow::anyhow!("failed to fetch '{}': {}", file.filename, e))?;

      let total_bytes = resp.content_length();
      let mut downloaded_bytes: u64 = 0;

      let tmp_path = dir.join(format!("{}.tmp", file.filename));
      let mut fh = tokio::fs::File::create(&tmp_path).await?;
      let mut stream = resp.bytes_stream();

      while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("download stream error: {}", e))?;
        downloaded_bytes += chunk.len() as u64;
        fh.write_all(&chunk).await?;

        let _ = app.emit(
          "model_download_progress",
          ModelDownloadPayload {
            file: file.filename.to_string(),
            bytes_done: downloaded_bytes,
            bytes_total: total_bytes,
            done: false,
            error: None,
          },
        );
      }

      fh.flush().await?;
      drop(fh);
      tokio::fs::rename(&tmp_path, &dest).await?;

      let _ = app.emit(
        "model_download_progress",
        ModelDownloadPayload {
          file: file.filename.to_string(),
          bytes_done: downloaded_bytes,
          bytes_total: total_bytes,
          done: true,
          error: None,
        },
      );
    }

    self.signal_ready(spec.id);
    Ok(())
  }

  pub async fn wait_until_ready(&self, model_id: &str) {
    let mut rx = {
      let guard = self.readiness.lock().unwrap();
      match guard.get(model_id) {
        Some(tx) => tx.subscribe(),
        None => return,
      }
    };

    if *rx.borrow() { return; }
    loop {
      if rx.changed().await.is_err() { break; }
      if *rx.borrow() { return; }
    }
  }
}
