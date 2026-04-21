use anyhow::Result;
use futures::StreamExt;
use ndarray::{Array2, Array4};
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

use crate::models::ModelDownloadPayload;

const IMAGE_SIZE: usize = 224;
const MODEL_FILES: &[&str] = &["model.onnx", "tokenizer.json"];

pub struct VisionEngine {
    pub data_dir: PathBuf,
    session: std::sync::Mutex<Option<(String, Session)>>,
    tokenizer: std::sync::Mutex<Option<(String, tokenizers::Tokenizer)>>,
    face_analyzer: tokio::sync::OnceCell<face_id::analyzer::FaceAnalyzer>,
    model_ready_tx: tokio::sync::watch::Sender<bool>,
    model_ready_rx: tokio::sync::watch::Receiver<bool>,
}

fn get_repo_and_subdir(model_id: &str) -> (&str, &str) {
    match model_id {
        "onnx-community/siglip2-base-patch32-256-ONNX" => (model_id, "onnx"),
        _ => (model_id, "siglip2-base-224"),
    }
}

fn get_remote_path(model_id: &str, filename: &str) -> String {
    match (model_id, filename) {
        ("onnx-community/siglip2-base-patch32-256-ONNX", "model.onnx") => "onnx/model_fp16.onnx".to_string(),
        _ => filename.to_string(),
    }
}

impl VisionEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        let (tx, rx) = tokio::sync::watch::channel(false);
        Self {
            data_dir,
            session: std::sync::Mutex::new(None),
            tokenizer: std::sync::Mutex::new(None),
            face_analyzer: tokio::sync::OnceCell::new(),
            model_ready_tx: tx,
            model_ready_rx: rx,
        }
    }

    fn model_dir(&self, model_id: &str) -> PathBuf {
        let (_, subdir) = get_repo_and_subdir(model_id);
        self.data_dir.join("models").join(subdir)
    }

    /// Downloads any missing model files from HuggingFace, then signals readiness.
    /// Emits `model_download_progress` events while downloading.
    /// Should be spawned once at startup before the embedding worker runs.
    pub async fn ensure_model_ready(&self, app: &AppHandle, model_id: &str) -> Result<()> {
        let (repo, _) = get_repo_and_subdir(model_id);
        let model_dir = self.model_dir(model_id);

        // If everything is already on disk, signal immediately and return.
        if MODEL_FILES.iter().all(|f| model_dir.join(f).exists()) {
            self.signal_ready();
            return Ok(());
        }

        tokio::fs::create_dir_all(&model_dir).await?;

        let client = reqwest::Client::new();

        for filename in MODEL_FILES {
            let dest = model_dir.join(filename);
            if dest.exists() {
                continue;
            }

            let remote = get_remote_path(model_id, filename);
            let url = format!(
                "https://huggingface.co/{}/resolve/main/{}",
                repo, remote
            );

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] downloading {} from {}", filename, url);

            let resp = client
                .get(&url)
                .send()
                .await?
                .error_for_status()
                .map_err(|e| anyhow::anyhow!("failed to fetch '{}': {}", filename, e))?;

            let total_bytes = resp.content_length();
            let mut downloaded: u64 = 0;

            // Write to a .tmp file first to avoid leaving partial files on crash/cancel.
            let tmp_path = model_dir.join(format!("{}.tmp", filename));
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            let mut stream = resp.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk =
                    chunk.map_err(|e| anyhow::anyhow!("download stream error: {}", e))?;
                downloaded += chunk.len() as u64;
                file.write_all(&chunk).await?;

                let _ = app.emit(
                    "model_download_progress",
                    ModelDownloadPayload {
                        file: filename.to_string(),
                        bytes_done: downloaded,
                        bytes_total: total_bytes,
                        done: false,
                        error: None,
                    },
                );
            }

            file.flush().await?;
            drop(file);
            tokio::fs::rename(&tmp_path, &dest).await?;

            let _ = app.emit(
                "model_download_progress",
                ModelDownloadPayload {
                    file: filename.to_string(),
                    bytes_done: downloaded,
                    bytes_total: total_bytes,
                    done: true,
                    error: None,
                },
            );

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] saved {} ({} bytes)", filename, downloaded);
        }

        self.signal_ready();
        Ok(())
    }

    fn signal_ready(&self) {
        let _ = self.model_ready_tx.send(true);
    }

    /// Resolves as soon as the model files are confirmed present on disk.
    /// Returns immediately if they were already ready when called.
    pub async fn wait_until_ready(&self) {
        let mut rx = self.model_ready_rx.clone();
        // Already ready — fast path.
        if *rx.borrow() {
            return;
        }
        loop {
            if rx.changed().await.is_err() {
                break; // sender dropped — shouldn't happen in normal operation
            }
            if *rx.borrow() {
                return;
            }
        }
    }

    pub async fn get_face_analyzer(&self) -> Result<&face_id::analyzer::FaceAnalyzer> {
        self.face_analyzer
            .get_or_try_init(|| async {
                face_id::analyzer::FaceAnalyzer::from_hf()
                    .build()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to build face analyzer: {}", e))
            })
            .await
    }

    fn get_session(&self, model_id: &str) -> Result<std::sync::MutexGuard<'_, Option<(String, Session)>>> {
        let mut lock = self
            .session
            .lock()
            .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;

        let needs_load = match &*lock {
            Some((current_id, _)) => current_id != model_id,
            None => true,
        };

        if needs_load {
            let model_path = self.model_dir(model_id).join("model.onnx");

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] loading session from: {}", model_path.display());

            let session = Session::builder()
                .map_err(|e| anyhow::anyhow!("failed to create session builder: {e}"))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| anyhow::anyhow!("failed to set optimization level: {e}"))?
                .with_intra_threads(4)
                .map_err(|e| anyhow::anyhow!("failed to set intra threads: {e}"))?
                .commit_from_file(&model_path)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to load ONNX model '{}': {e}",
                        model_path.display()
                    )
                })?;

            #[cfg(debug_assertions)]
            eprintln!("[vision-engine] session ready");

            *lock = Some((model_id.to_string(), session));
        }
        Ok(lock)
    }

    pub fn embed_image(&self, img: &image::DynamicImage, model_id: &str) -> Result<Vec<f32>> {
        let resized = img.resize_exact(
            IMAGE_SIZE as u32,
            IMAGE_SIZE as u32,
            image::imageops::FilterType::Lanczos3,
        );
        let rgb = resized.to_rgb8();

        let mut pixel_values = Array4::<f32>::zeros((1, 3, IMAGE_SIZE, IMAGE_SIZE));
        for (x, y, pixel) in rgb.enumerate_pixels() {
            for c in 0..3 {
                let val = pixel[c] as f32 / 255.0;
                pixel_values[[0, c, y as usize, x as usize]] = (val - 0.5) / 0.5;
            }
        }

        // Dummy input_ids (batch=1, seq_len=1) — text encoder output is discarded
        let dummy_ids = Array2::<i64>::zeros((1, 1));

        let mut lock = self.get_session(model_id)?;
        let (_, session) = lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("session not initialized"))?;

        let pv_ref = TensorRef::from_array_view(pixel_values.view())
            .map_err(|e| anyhow::anyhow!("failed to create pixel_values tensor: {e}"))?;
        let ids_ref = TensorRef::from_array_view(dummy_ids.view())
            .map_err(|e| anyhow::anyhow!("failed to create dummy input_ids tensor: {e}"))?;

        let outputs = session
            .run(ort::inputs!["pixel_values" => pv_ref, "input_ids" => ids_ref])
            .map_err(|e| anyhow::anyhow!("image inference failed: {e}"))?;

        let (_shape, data) = outputs["image_embeds"]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("failed to extract image embedding: {e}"))?;
        Ok(data.to_vec())
    }

    pub fn embed_text(&self, text: &str, model_id: &str) -> Result<Vec<f32>> {
        const MAX_SEQ_LEN: usize = 64;

        let encoding = {
            let mut tok_lock = self
                .tokenizer
                .lock()
                .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;

            let needs_load = match &*tok_lock {
                Some((current_id, _)) => current_id != model_id,
                None => true,
            };

            if needs_load {
                let tok_path = self.model_dir(model_id).join("tokenizer.json");

                #[cfg(debug_assertions)]
                eprintln!(
                    "[vision-engine] loading tokenizer from: {}",
                    tok_path.display()
                );

                *tok_lock = Some((
                    model_id.to_string(),
                    tokenizers::Tokenizer::from_file(tok_path)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                ));
            }
            tok_lock
                .as_ref()
                .unwrap()
                .1
                .encode(text, true)
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }; // tokenizer lock released before acquiring session lock

        let input_ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .take(MAX_SEQ_LEN)
            .map(|&id| id as i64)
            .collect();
        let seq_len = input_ids.len();
        let input_ids_arr = Array2::from_shape_vec((1, seq_len), input_ids)?;

        // Dummy pixel_values — image encoder output is discarded
        let dummy_pixels = Array4::<f32>::zeros((1, 3, IMAGE_SIZE, IMAGE_SIZE));

        let mut lock = self.get_session(model_id)?;
        let (_, session) = lock
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("session not initialized"))?;

        let ids_ref = TensorRef::from_array_view(input_ids_arr.view())
            .map_err(|e| anyhow::anyhow!("failed to create input_ids tensor: {e}"))?;
        let pv_ref = TensorRef::from_array_view(dummy_pixels.view())
            .map_err(|e| anyhow::anyhow!("failed to create dummy pixel_values tensor: {e}"))?;

        let outputs = session
            .run(ort::inputs!["input_ids" => ids_ref, "pixel_values" => pv_ref])
            .map_err(|e| anyhow::anyhow!("text inference failed: {e}"))?;

        let (_shape, data) = outputs["text_embeds"]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("failed to extract text embedding: {e}"))?;
        Ok(data.to_vec())
    }
}
