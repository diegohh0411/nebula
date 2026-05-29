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
  /// Separate vision-tower ONNX file (image encoder). When set, embed_image uses this.
  pub vision_file: Option<ModelFile>,
  /// Separate text-tower ONNX file (text encoder). When set, embed_text uses this.
  pub text_file: Option<ModelFile>,
  /// Input tensor name for the vision tower (e.g. "pixel_values").
  pub vision_input: &'static str,
  /// Output tensor name for the vision tower (e.g. "image_embeds").
  pub vision_output: &'static str,
  /// Input tensor name for the text tower (e.g. "input_ids").
  pub text_input: &'static str,
  /// Output tensor name for the text tower (e.g. "text_embeds").
  pub text_output: &'static str,
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
  /// Returns all files required for this model (model file + optional tokenizer + split towers)
  pub fn all_files(&self) -> Vec<&ModelFile> {
    let mut f = vec![&self.model_file];
    if let Some(ref t) = self.tokenizer_file {
        f.push(t);
    }
    if let Some(ref v) = self.vision_file {
        // Only add if different from model_file (avoid duplicate download)
        if v.filename != self.model_file.filename {
            f.push(v);
        }
    }
    if let Some(ref t) = self.text_file {
        f.push(t);
    }
    f
  }
}

pub const SIGLIP_BASE: ModelSpec = ModelSpec {
  // id is kept stable so stored user settings survive the hf_repo correction.
  id: "onnx-community/siglip2-base-patch16-224",
  hf_repo: "onnx-community/siglip2-base-patch16-224-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-224-split",
  model_file: ModelFile { filename: "vision_model.onnx", remote_path: Some("onnx/vision_model.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Standard",
  display_description: "Balanced quality and speed (86M params, split towers)",
  image_size: 224,
  vision_file: Some(ModelFile { filename: "vision_model.onnx", remote_path: Some("onnx/vision_model.onnx") }),
  text_file: Some(ModelFile { filename: "text_model.onnx", remote_path: Some("onnx/text_model.onnx") }),
  vision_input: "pixel_values",
  vision_output: "pooler_output",
  text_input: "input_ids",
  text_output: "pooler_output",
};

pub const SIGLIP_FAST: ModelSpec = ModelSpec {
  id: "onnx-community/siglip2-base-patch32-256-ONNX",
  hf_repo: "onnx-community/siglip2-base-patch32-256-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-256-split",
  // model_file points at the vision tower so "ready" checks pass with the same
  // logic used by SIGLIP_BASE; the text tower is declared separately below.
  model_file: ModelFile { filename: "vision_model_quantized.onnx", remote_path: Some("onnx/vision_model_quantized.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Fast",
  display_description: "Optimized for consumer CPUs — INT8 quantized, 32px patches (64 tokens vs 196)",
  image_size: 256,
  vision_file: Some(ModelFile { filename: "vision_model_quantized.onnx", remote_path: Some("onnx/vision_model_quantized.onnx") }),
  text_file: Some(ModelFile { filename: "text_model_quantized.onnx", remote_path: Some("onnx/text_model_quantized.onnx") }),
  // Confirmed by inspecting the ONNX graph: onnx-community exports all SigLIP2
  // variants with pooler_output (not image_embeds/text_embeds).
  vision_input: "pixel_values",
  vision_output: "pooler_output",
  text_input: "input_ids",
  text_output: "pooler_output",
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
  vision_file: None,
  text_file: None,
  vision_input: "",
  vision_output: "",
  text_input: "",
  text_output: "",
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
  vision_file: None,
  text_file: None,
  vision_input: "",
  vision_output: "",
  text_input: "",
  text_output: "",
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
  image_size: 0,
  vision_file: None,
  text_file: None,
  vision_input: "",
  vision_output: "",
  text_input: "",
  text_output: "",
};

pub const BUFFALO_S_PRESET: FaceIdPreset = FaceIdPreset {
    id: "blitz",
    name: "Blitz",
    description: "Maximum inference speed, for bulk processing",
    detector: &BUFFALO_S_DETECTION,
    embedder: &BUFFALO_S_RECOGNITION,
    gender_age: &BUFFALO_S_GENDER_AGE,
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
        assert_eq!(gender_age_model.id, "buffalo_s_gender_age");
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
        assert_eq!(preset.gender_age.id, "buffalo_s_gender_age");
    }

    #[test]
    fn standard_model_has_split_towers() {
        let s = &SIGLIP_BASE;
        assert!(s.vision_file.is_some(), "vision tower must be configured");
        assert!(s.text_file.is_some(), "text tower must be configured");
        assert_eq!(s.vision_input, "pixel_values");
        assert!(!s.vision_output.is_empty());
        assert!(!s.text_output.is_empty());
    }
}
