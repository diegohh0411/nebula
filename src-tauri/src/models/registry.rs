pub struct ModelFile {
  /// Filename on disk
  pub filename: &'static str,
  /// Path within HuggingFace repo, set to None if same as the value of `filename`
  pub remote_path: Option<&'static str>,
}

pub enum ModelType {
  /// Used for general embedding of images & texts in the same vector space to enable Natural Language Search of pictures
  TextImageEmbedding,
  /// Used for face crop embedding in subject analysis
  FaceEmbedding,
  /// Used to find faces in images for subject analysis
  FaceDetection,
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
  /// Files that must be present for this model to be "ready"
  pub files: &'static [ModelFile],
  /// Display name for the UI
  pub display_name: &'static str,
  /// Display description for the UI
  pub display_description: &'static str,
}

pub const SIGLIP_FAST: ModelSpec = ModelSpec {
  id: "siglip_fast",
  hf_repo: "onnx-community/siglip2-base-patch16-naflex-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip_fast",
  files: &[
    ModelFile { filename: "vision_model_q4.onnx", remote_path: Some("onnx/vision_model_q4.onnx") },
    ModelFile { filename: "tokenizer.json", remote_path: None },
  ],
  display_name: "Fast",
  display_description: "Optimized for consumer CPUs with larger patches",
};

pub const BUFFALO_S_RECOGNITION: ModelSpec = ModelSpec {
  id: "buffalo_s_recognition",
  hf_repo: "immich-app/buffalo_s",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "buffalo_s",
  files: &[
    ModelFile { filename: "recognition.onnx", remote_path: Some("recognition/model.onnx") }
  ],
  display_name: "Recognition",
  display_description: "Buffalo S recognition model"
};

pub const BUFFALO_S_DETECTION: ModelSpec = ModelSpec {
  id: "buffalo_s_detection",
  hf_repo: "immich-app/buffalo_s",
  model_type: ModelType::FaceDetection,
  cache_dir: "buffalo_s",
  files: &[
    ModelFile { filename: "detection.onnx", remote_path: Some("detection/model.onnx") }
  ],
  display_name: "Detection",
  display_description: "Buffalo S detection model"
};

pub const ALL: &[&ModelSpec] = &[&SIGLIP_FAST, &BUFFALO_S_RECOGNITION, &BUFFALO_S_DETECTION];

impl ModelSpec {
  pub fn find_by_id(id: &str) -> Option<&'static ModelSpec> {
    ALL.iter().find(|m| m.id == id).copied()
  }
}
