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
  /// Input image resolution for embedding models
  pub image_size: usize,
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
    /// Gender/age estimation model specification
    pub gender_age: &'static ModelSpec,
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

pub const SIGLIP_FAST: ModelSpec = ModelSpec {
  id: "onnx-community/siglip2-base-patch32-256-ONNX",
  hf_repo: "onnx-community/siglip2-base-patch32-256-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-patch32-256",
  model_file: ModelFile { filename: "model_q4.onnx", remote_path: Some("onnx/model_q4.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Fast",
  display_description: "Optimized for consumer CPUs with larger patches",
  image_size: 256,
};

pub const BUFFALO_L_RECOGNITION: ModelSpec = ModelSpec {
  id: "buffalo_l_recognition",
  hf_repo: "immich-app/buffalo_l",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "buffalo_l",
  model_file: ModelFile { filename: "recognition.onnx", remote_path: Some("recognition/model.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo L Recognition",
  display_description: "Industry standard face recognition model",
  image_size: 0,
};

pub const BUFFALO_L_DETECTION: ModelSpec = ModelSpec {
  id: "buffalo_l_detection",
  hf_repo: "immich-app/buffalo_l",
  model_type: ModelType::FaceDetection,
  cache_dir: "buffalo_l",
  model_file: ModelFile { filename: "detection.onnx", remote_path: Some("detection/model.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo L Detection",
  display_description: "Industry standard face detection model",
  image_size: 0,
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
  image_size: 0,
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
  image_size: 0,
};

pub const BUFFALO_L_GENDER_AGE: ModelSpec = ModelSpec {
  id: "buffalo_l_gender_age",
  hf_repo: "public-data/insightface",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "buffalo_l",
  model_file: ModelFile { filename: "genderage.onnx", remote_path: Some("models/buffalo_l/genderage.onnx") },
  tokenizer_file: None,
  display_name: "Buffalo L Gender/Age",
  display_description: "Gender and age estimation model",
  image_size: 0,
};

pub const BUFFALO_L_PRESET: FaceIdPreset = FaceIdPreset {
    id: "standard",
    name: "Standard",
    description: "Industry standard, best precision",
    detector: &BUFFALO_L_DETECTION,
    embedder: &BUFFALO_L_RECOGNITION,
    gender_age: &BUFFALO_L_GENDER_AGE,
    detector_input_size: (640, 640),
};

pub const BUFFALO_S_PRESET: FaceIdPreset = FaceIdPreset {
    id: "blitz",
    name: "Blitz",
    description: "Maximum inference speed, for bulk processing",
    detector: &BUFFALO_S_DETECTION,
    embedder: &BUFFALO_S_RECOGNITION,
    gender_age: &BUFFALO_L_GENDER_AGE,
    detector_input_size: (640, 640),
};

pub const ALL_MODELS: &[&ModelSpec] = &[&SIGLIP_FAST, &BUFFALO_L_RECOGNITION, &BUFFALO_L_DETECTION, &BUFFALO_S_RECOGNITION, &BUFFALO_S_DETECTION, &BUFFALO_L_GENDER_AGE];
pub const ALL_PRESETS: &[&FaceIdPreset] = &[&BUFFALO_L_PRESET, &BUFFALO_S_PRESET];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_faceid_preset_gender_age_is_not_option() {
        // Verify that gender_age is not an Option type
        let preset = &BUFFALO_S_PRESET;

        // This should compile and work without unwrapping or matching on Option
        let gender_age_model = preset.gender_age;

        // Verify it's the expected model
        assert_eq!(gender_age_model.id, "buffalo_l_gender_age");
        assert_eq!(gender_age_model.hf_repo, "public-data/insightface");
    }

    #[test]
    fn test_find_by_id_works() {
        let preset = FaceIdPreset::find_by_id("blitz");
        assert!(preset.is_some());

        let preset = preset.unwrap();
        assert_eq!(preset.id, "blitz");
        assert_eq!(preset.name, "Blitz");

        // Verify gender_age is accessible directly (not an Option)
        assert_eq!(preset.gender_age.id, "buffalo_l_gender_age");
    }
}
