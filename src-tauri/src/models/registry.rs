pub enum ModelType {
  /// Used for general embedding of images & texts in the same vector space to enable Natural Language Search of pictures
  TextImageEmbedding,
  /// Used for face crop embedding in subject analysis
  FaceEmbedding,
  /// Used to find faces in images for subject analysis
  FaceDetection,
}

pub struct ModelFile {
  /// Filename on disk
  pub filename: &'static str,
  /// Path within HuggingFace repo, set to None if same as the value of `filename`
  pub remote_path: Option<&'static str>,
}

pub struct ModelSpec {
  /// ID for internal Nebula reference
  pub id: &'static str,
  /// HuggingFace repo ID (may differ from id if you use aliases)
  pub hf_repo: &'static str,
  /// Type of ML model, e.g. ModelType::FaceDetection
  pub model_type: ModelType,
  /// Subdirectory under <data_dir>/models/ to store files
  pub cache_dir: &'static str,
  /// Main model file that must be present for this model to be "ready"
  pub model_file: ModelFile,
  /// Optional tokenizer file (for text/image models)
  pub tokenizer_file: Option<ModelFile>,
  /// Display name for the UI
  pub display_name: &'static str,
  /// Display description for the UI
  pub display_description: &'static str,
}

pub struct FaceIdPreset {
    /// Internal ID for the preset
    pub id: &'static str,
    /// Display name for the UI
    pub name: &'static str,
    /// Description explaining when to use this preset
    pub description: &'static str,
    /// Face detection model specification
    pub detector: &'static ModelSpec,
    /// Face embedding model specification
    pub embedder: &'static ModelSpec,
    /// Gender/age estimation model specification (optional - set to None to skip gender/age inference)
    pub gender_age: Option<&'static ModelSpec>,
    /// Input size (width, height) for the detector model
    pub detector_input_size: (u32, u32),
}

impl ModelSpec {
  /// Returns all files required for this model (model file + optional tokenizer)
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

pub const BUFFALO_S_GENDER_AGE: ModelSpec = ModelSpec {
  id: "buffalo_s_gender_age",
  hf_repo: "public-data/insightface",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "buffalo_s",
  model_file: ModelFile { filename: "genderage.onnx", remote_path: Some("models/buffalo_l/genderage.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo S Gender/Age",
  display_description: "Gender and age estimation model",
};

pub const BUFFALO_S_PRESET: FaceIdPreset = FaceIdPreset {
    id: "blitz",
    name: "Blitz",
    description: "Maximum inference speed, for bulk processing",
    detector: &BUFFALO_S_DETECTION,
    embedder: &BUFFALO_S_RECOGNITION,
    gender_age: Some(&BUFFALO_S_GENDER_AGE),
    detector_input_size: (640, 640),
};

pub const ALL_MODELS: &[&ModelSpec] = &[&SIGLIP_BASE, &SIGLIP_FAST, &BUFFALO_S_RECOGNITION, &BUFFALO_S_DETECTION, &BUFFALO_S_GENDER_AGE];
pub const ALL_PRESETS: &[&FaceIdPreset] = &[&BUFFALO_S_PRESET];

impl ModelSpec {
  /// Find a model specification by its ID
  pub fn find_by_id(id: &str) -> Option<&'static ModelSpec> {
    ALL_MODELS.iter().find(|m| m.id == id).copied()
  }
}

impl FaceIdPreset {
  /// Find a face ID preset by its ID
  pub fn find_by_id(id: &str) -> Option<&'static FaceIdPreset> {
    ALL_PRESETS.iter().find(|p| p.id == id).copied()
  }
}
