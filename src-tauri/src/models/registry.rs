pub enum ModelType {
  TextImageEmbedding,
  FaceEmbedding,
  FaceDetection,
}

pub struct ModelFile {
  pub filename: &'static str,
  pub remote_path: Option<&'static str>,
}

pub struct ModelSpec {
  pub id: &'static str,
  pub hf_repo: &'static str,
  pub model_type: ModelType,
  pub cache_dir: &'static str,
  pub model_file: ModelFile,
  pub tokenizer_file: Option<ModelFile>,
  pub display_name: &'static str,
  pub display_description: &'static str,
}

pub struct FaceIdPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub detector: &'static ModelSpec,
    pub embedder: &'static ModelSpec,
    pub detector_input_size: (u32, u32),
}

impl ModelSpec {
  pub fn all_files(&self) -> Vec<&ModelFile> {
    let mut f = vec![&self.model_file];
    if let Some(ref t) = self.tokenizer_file {
      f.push(t);
    }
    f
  }
}

pub const SIGLIP_BASE: ModelSpec = ModelSpec {
  id: "diegohh/siglip2-base-patch16-224",
  hf_repo: "diegohh/siglip2-base-patch16-224",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-224",
  model_file: ModelFile { filename: "model.onnx", remote_path: None },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Standard",
  display_description: "Balanced quality and speed (86M params)",
};

pub const SIGLIP_FAST: ModelSpec = ModelSpec {
  id: "onnx-community/siglip2-base-patch32-256-ONNX",
  hf_repo: "onnx-community/siglip2-base-patch32-256-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-256",
  model_file: ModelFile { filename: "model_fp16.onnx", remote_path: Some("onnx/model_fp16.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Fast",
  display_description: "Optimized for consumer CPUs with larger patches",
};

pub const BUFFALO_S_RECOGNITION: ModelSpec = ModelSpec {
  id: "buffalo_s_recognition",
  hf_repo: "immich-app/buffalo_s",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "buffalo_s",
  model_file: ModelFile { filename: "recognition.onnx", remote_path: Some("recognition/model.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo S Recognition",
  display_description: "Lightweight face recognition model",
};

pub const BUFFALO_S_DETECTION: ModelSpec = ModelSpec {
  id: "buffalo_s_detection",
  hf_repo: "immich-app/buffalo_s",
  model_type: ModelType::FaceDetection,
  cache_dir: "buffalo_s",
  model_file: ModelFile { filename: "detection.onnx", remote_path: Some("detection/model.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo S Detection",
  display_description: "Lightweight face detection model",
};

pub const BUFFALO_S_PRESET: FaceIdPreset = FaceIdPreset {
    id: "blitz",
    name: "Blitz",
    description: "Maximum inference speed, for bulk processing",
    detector: &BUFFALO_S_DETECTION,
    embedder: &BUFFALO_S_RECOGNITION,
    detector_input_size: (640, 640),
};

pub const ALL_MODELS: &[&ModelSpec] = &[&SIGLIP_BASE, &SIGLIP_FAST, &BUFFALO_S_RECOGNITION, &BUFFALO_S_DETECTION];
pub const ALL_PRESETS: &[&FaceIdPreset] = &[&BUFFALO_S_PRESET];

impl ModelSpec {
  pub fn find_by_id(id: &str) -> Option<&'static ModelSpec> {
    ALL_MODELS.iter().find(|m| m.id == id).copied()
  }
}

impl FaceIdPreset {
  pub fn find_by_id(id: &str) -> Option<&'static FaceIdPreset> {
    ALL_PRESETS.iter().find(|p| p.id == id).copied()
  }
}
