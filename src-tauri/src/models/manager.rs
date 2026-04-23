use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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
  downloads: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl ModelManager {
  pub fn new(data_dir: PathBuf) -> Self {
    Self {
      data_dir,
      readiness: std::sync::Mutex::new(HashMap::new()),
      downloads: std::sync::Mutex::new(HashMap::new()),
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
    // 1. Ensure a readiness channel exists for this model
    {
      let mut guard = self.readiness.lock().unwrap();
      guard
        .entry(spec.id.to_string())
        .or_insert_with(|| tokio::sync::watch::channel(false).0);
    }

    // 2. Acquire per-model download lock (fast if uncontended)
    let model_lock = {
      let mut guard = self.downloads.lock().unwrap();
      guard
        .entry(spec.id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
    };
    let _lock_guard = model_lock.lock().await;

    // 3. Fast path — all files already on disk (re-check after acquiring lock)
    let dir = self.model_dir(spec);
    if spec.all_files().iter().all(|f| dir.join(f.filename).exists()) {
      self.signal_ready(spec.id);
      return Ok(());
    }

    // 4. Create the cache directory
    tokio::fs::create_dir_all(&dir).await?;

    // 5. Download each missing file
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicUsize, Ordering};

  #[tokio::test]
  async fn per_model_lock_serializes_concurrent_access() {
    let map: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> =
      std::sync::Mutex::new(HashMap::new());

    let lock_a = {
      let mut guard = map.lock().unwrap();
      guard
        .entry("model-a".to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
    };
    let lock_b = {
      let mut guard = map.lock().unwrap();
      guard
        .entry("model-b".to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
    };

    let counter = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));

    let counter_a = counter.clone();
    let max_a = max_concurrent.clone();
    let lock_a_clone = lock_a.clone();

    let counter_b = counter.clone();
    let max_b = max_concurrent.clone();
    let lock_b_clone = lock_b.clone();

    // Different models should be able to run concurrently
    let h1 = tokio::spawn(async move {
      let _g = lock_a_clone.lock().await;
      counter_a.fetch_add(1, Ordering::SeqCst);
      let now = counter_a.load(Ordering::SeqCst);
      max_a.fetch_max(now, Ordering::SeqCst);
      tokio::task::yield_now().await;
      counter_a.fetch_sub(1, Ordering::SeqCst);
    });

    let h2 = tokio::spawn(async move {
      let _g = lock_b_clone.lock().await;
      counter_b.fetch_add(1, Ordering::SeqCst);
      let now = counter_b.load(Ordering::SeqCst);
      max_b.fetch_max(now, Ordering::SeqCst);
      tokio::task::yield_now().await;
      counter_b.fetch_sub(1, Ordering::SeqCst);
    });

    h1.await.unwrap();
    h2.await.unwrap();

    assert_eq!(
      max_concurrent.load(Ordering::SeqCst),
      2,
      "different model locks should allow concurrent access"
    );
  }

  #[tokio::test]
  async fn same_model_lock_serializes_callers() {
    let map: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> =
      std::sync::Mutex::new(HashMap::new());

    let lock = {
      let mut guard = map.lock().unwrap();
      guard
        .entry("model-x".to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
    };

    let counter = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for _ in 0..4 {
      let c = counter.clone();
      let m = max_concurrent.clone();
      let l = lock.clone();
      handles.push(tokio::spawn(async move {
        let _g = l.lock().await;
        c.fetch_add(1, Ordering::SeqCst);
        let now = c.load(Ordering::SeqCst);
        m.fetch_max(now, Ordering::SeqCst);
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        c.fetch_sub(1, Ordering::SeqCst);
      }));
    }

    for h in handles {
      h.await.unwrap();
    }

    assert_eq!(
      max_concurrent.load(Ordering::SeqCst),
      1,
      "same model lock should serialize all callers"
    );
  }
}
