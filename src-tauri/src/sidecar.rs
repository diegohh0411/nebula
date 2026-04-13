use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::siglip;
use hf_hub::api::sync::Api;
use serde_json::{json, Value};
use tokenizers::Tokenizer;

pub struct SidecarManager {
    model: Option<siglip::Model>,
    tokenizer: Option<Tokenizer>,
    device: Device,
    ready: bool,
}

impl SidecarManager {
    pub fn new() -> Self {
        SidecarManager {
            model: None,
            tokenizer: None,
            device: Device::Cpu,
            ready: false,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.ready {
            return Ok(());
        }

        let api = Api::new().map_err(|e| e.to_string())?;
        let repo = api.model("google/siglip2-base-patch16-384".to_string());

        let model_file = repo.get("model.safetensors").map_err(|e| e.to_string())?;
        let config_file = repo.get("config.json").map_err(|e| e.to_string())?;
        let tokenizer_file = repo.get("tokenizer.json").map_err(|e| e.to_string())?;

        let config: siglip::Config = serde_json::from_reader(
            std::fs::File::open(config_file).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

        let device = if candle_core::utils::cuda_is_available() {
            Device::new_cuda(0).unwrap_or(Device::Cpu)
        } else if candle_core::utils::metal_is_available() {
            Device::new_metal(0).unwrap_or(Device::Cpu)
        } else {
            Device::Cpu
        };
        self.device = device;

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_file], candle_core::DType::F32, &self.device)
                .map_err(|e| e.to_string())?
        };

        let model = siglip::Model::new(&config, vb).map_err(|e| e.to_string())?;
        let tokenizer = Tokenizer::from_file(tokenizer_file).map_err(|e| e.to_string())?;

        self.model = Some(model);
        self.tokenizer = Some(tokenizer);
        self.ready = true;

        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        self.model = None;
        self.tokenizer = None;
        self.ready = false;
        Ok(())
    }

    pub fn send_request(&mut self, request: &Value) -> Result<Value, String> {
        if !self.ready {
            return Err("Model not loaded".to_string());
        }

        let action = request["action"].as_str().ok_or("Missing action")?;

        match action {
            "embed_image" => {
                let path = request["image_path"].as_str().ok_or("Missing image_path")?;
                let embedding = self.embed_image(path)?;
                Ok(json!({
                    "status": "ok",
                    "action": "embed_image",
                    "image_path": path,
                    "embedding": embedding
                }))
            }
            "embed_text" => {
                let text = request["text"].as_str().ok_or("Missing text")?;
                let embedding = self.embed_text(text)?;
                Ok(json!({
                    "status": "ok",
                    "action": "embed_text",
                    "text": text,
                    "embedding": embedding
                }))
            }
            "health_check" => {
                Ok(json!({
                    "status": "ok",
                    "action": "health_check",
                    "ready": self.ready
                }))
            }
            "shutdown" => {
                self.shutdown()?;
                Ok(json!({"status": "ok"}))
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }

    pub fn embed_image(&self, path: &str) -> Result<Vec<f32>, String> {
        let model = self.model.as_ref().ok_or("Model not loaded")?;
        
        let img = image::ImageReader::open(path)
            .map_err(|e| format!("Failed to open image {}: {}", path, e))?
            .decode()
            .map_err(|e| format!("Failed to decode image {}: {}", path, e))?;
        
        // SigLIP 2 base-patch16-384 expects 384x384
        let img = img.resize_to_fill(384, 384, image::imageops::FilterType::Triangle);
        let img = img.to_rgb8();
        
        let data = img.into_raw();
        let data = Tensor::from_vec(data, (384, 384, 3), &self.device).map_err(|e| e.to_string())?;
        let data = data.permute((2, 0, 1)).map_err(|e| e.to_string())?; // HWC to CHW
        let data = (data.to_dtype(candle_core::DType::F32).map_err(|e| e.to_string())? / 255.0).map_err(|e| e.to_string())?;
        
        // Normalization: mean=[0.5, 0.5, 0.5], std=[0.5, 0.5, 0.5]
        let mean = Tensor::new(&[0.5f32, 0.5, 0.5], &self.device).map_err(|e| e.to_string())?.reshape((3, 1, 1)).map_err(|e| e.to_string())?;
        let std = Tensor::new(&[0.5f32, 0.5, 0.5], &self.device).map_err(|e| e.to_string())?.reshape((3, 1, 1)).map_err(|e| e.to_string())?;
        let data = ((data - mean).map_err(|e| e.to_string())? / std).map_err(|e| e.to_string())?;
        
        let data = data.unsqueeze(0).map_err(|e| e.to_string())?; // Batch dimension
        
        let embeddings = model.get_image_features(&data).map_err(|e| e.to_string())?;
        let embeddings = embeddings.get(0).map_err(|e| e.to_string())?;
        
        // L2 normalize
        let norm = embeddings.sqr().map_err(|e| e.to_string())?.sum_all().map_err(|e| e.to_string())?.sqrt().map_err(|e| e.to_string())?;
        let embeddings = (embeddings / norm).map_err(|e| e.to_string())?;
        
        embeddings.to_vec1().map_err(|e| e.to_string())
    }

    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>, String> {
        let model = self.model.as_ref().ok_or("Model not loaded")?;
        let tokenizer = self.tokenizer.as_ref().ok_or("Tokenizer not loaded")?;
        
        let tokens = tokenizer.encode(text, true).map_err(|e| e.to_string())?;
        let tokens = tokens.get_ids();
        let tokens = Tensor::new(tokens, &self.device).map_err(|e| e.to_string())?.unsqueeze(0).map_err(|e| e.to_string())?;
        
        let embeddings = model.get_text_features(&tokens).map_err(|e| e.to_string())?;
        let embeddings = embeddings.get(0).map_err(|e| e.to_string())?;
        
        // L2 normalize
        let norm = embeddings.sqr().map_err(|e| e.to_string())?.sum_all().map_err(|e| e.to_string())?.sqrt().map_err(|e| e.to_string())?;
        let embeddings = (embeddings / norm).map_err(|e| e.to_string())?;
        
        embeddings.to_vec1().map_err(|e| e.to_string())
    }
}
