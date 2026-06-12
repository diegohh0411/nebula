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
  /// Approximate total download size in bytes across all model files
  pub size_bytes: u64,
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

pub const SIGLIP_FAST: ModelSpec = ModelSpec {
  id: "onnx-community/siglip2-base-patch32-256-ONNX",
  hf_repo: "onnx-community/siglip2-base-patch32-256-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-256-split",
  // model_file points at the vision tower so "ready" checks pass with the same
  // logic used by SIGLIP_BASE; the text tower is declared separately below.
  model_file: ModelFile { filename: "vision_model_quantized.onnx", remote_path: Some("onnx/vision_model_quantized.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Blitz",
  display_description: "Faster search with a small quality tradeoff. Good for large libraries and slower hardware.",
  image_size: 256,
  vision_file: Some(ModelFile { filename: "vision_model_quantized.onnx", remote_path: Some("onnx/vision_model_quantized.onnx") }),
  text_file: Some(ModelFile { filename: "text_model_quantized.onnx", remote_path: Some("onnx/text_model_quantized.onnx") }),
  // Confirmed by inspecting the ONNX graph: onnx-community exports all SigLIP2
  // variants with pooler_output (not image_embeds/text_embeds).
  vision_input: "pixel_values",
  vision_output: "pooler_output",
  text_input: "input_ids",
  text_output: "pooler_output",
  size_bytes: 169_000_000, // vision_model_quantized.onnx (~90 MB) + text_model_quantized.onnx (~70 MB) + tokenizer.json
};

pub const SIGLIP_BASE: ModelSpec = ModelSpec {
  id: "onnx-community/siglip2-base-patch16-224-ONNX",
  hf_repo: "onnx-community/siglip2-base-patch16-224-ONNX",
  model_type: ModelType::TextImageEmbedding,
  cache_dir: "siglip2-base-224-split",
  model_file: ModelFile { filename: "vision_model.onnx", remote_path: Some("onnx/vision_model.onnx") },
  tokenizer_file: Some(ModelFile { filename: "tokenizer.json", remote_path: None }),
  display_name: "Standard",
  display_description: "Best search accuracy. Recommended for most users.",
  image_size: 224,
  vision_file: Some(ModelFile { filename: "vision_model.onnx", remote_path: Some("onnx/vision_model.onnx") }),
  text_file: Some(ModelFile { filename: "text_model.onnx", remote_path: Some("onnx/text_model.onnx") }),
  vision_input: "pixel_values",
  vision_output: "pooler_output",
  text_input: "input_ids",
  text_output: "pooler_output",
  size_bytes: 660_000_000, // vision_model.onnx (~360 MB) + text_model.onnx (~270 MB) + tokenizer.json
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
  size_bytes: 20_000_000, // recognition.onnx (~19 MB)
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
  size_bytes: 4_000_000, // detection.onnx (~4 MB)
};

pub const BUFFALO_S_GENDER_AGE: ModelSpec = ModelSpec {
  id: "buffalo_s_gender_age",
  hf_repo: "public-data/insightface",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "insightface",
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
  size_bytes: 1_300_000, // genderage.onnx (~1.3 MB)
};

pub const BUFFALO_S_PRESET: FaceIdPreset = FaceIdPreset {
    id: "blitz",
    name: "Blitz",
    description: "Fastest face recognition. Ideal for large libraries.",
    detector: &BUFFALO_S_DETECTION,
    embedder: &BUFFALO_S_RECOGNITION,
    gender_age: &BUFFALO_S_GENDER_AGE,
    detector_input_size: (640, 640),
};

pub const ANTELOPE_V2_DETECTION: ModelSpec = ModelSpec {
  id: "antelopev2_detection",
  hf_repo: "immich-app/antelopev2",
  model_type: ModelType::FaceDetection,
  cache_dir: "antelopev2",
  model_file: ModelFile { filename: "detection.onnx", remote_path: Some("detection/model.onnx") },
  tokenizer_file: None,
  display_name: "Antelope V2 Detection",
  display_description: "High-accuracy SCRFD face detection model",
  image_size: 0,
  vision_file: None,
  text_file: None,
  vision_input: "",
  vision_output: "",
  text_input: "",
  text_output: "",
  size_bytes: 17_000_000,
};

pub const ANTELOPE_V2_RECOGNITION: ModelSpec = ModelSpec {
  id: "antelopev2_recognition",
  hf_repo: "immich-app/antelopev2",
  model_type: ModelType::FaceEmbedding,
  cache_dir: "antelopev2",
  model_file: ModelFile { filename: "recognition.onnx", remote_path: Some("recognition/model.onnx") },
  tokenizer_file: None,
  display_name: "Antelope V2 Recognition",
  display_description: "Maximum-accuracy glintr100 face recognition model",
  image_size: 0,
  vision_file: None,
  text_file: None,
  vision_input: "",
  vision_output: "",
  text_input: "",
  text_output: "",
  size_bytes: 261_000_000,
};

pub const ANTELOPE_V2_PRESET: FaceIdPreset = FaceIdPreset {
    id: "precision",
    name: "Standard",
    description: "Highest-accuracy face recognition. Best for challenging photos with tricky lighting or angles.",
    detector: &ANTELOPE_V2_DETECTION,
    embedder: &ANTELOPE_V2_RECOGNITION,
    gender_age: &BUFFALO_S_GENDER_AGE,
    detector_input_size: (640, 640),
};

pub const ALL_MODELS: &[&ModelSpec] = &[
    &SIGLIP_FAST, &SIGLIP_BASE,
    &BUFFALO_S_DETECTION, &BUFFALO_S_RECOGNITION, &BUFFALO_S_GENDER_AGE,
    &ANTELOPE_V2_DETECTION, &ANTELOPE_V2_RECOGNITION,
];
pub const ALL_PRESETS: &[&FaceIdPreset] = &[&BUFFALO_S_PRESET, &ANTELOPE_V2_PRESET];

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
        let preset = &BUFFALO_S_PRESET;
        let gender_age_model = preset.gender_age;
        assert_eq!(gender_age_model.id, "buffalo_s_gender_age");
        assert_eq!(gender_age_model.hf_repo, "public-data/insightface");
    }

    #[test]
    fn test_find_by_id_works() {
        let preset = FaceIdPreset::find_by_id("blitz").unwrap();
        assert_eq!(preset.id, "blitz");
        assert_eq!(preset.name, "Blitz");
        assert_eq!(preset.gender_age.id, "buffalo_s_gender_age");
    }

    #[test]
    fn blitz_smart_search_model_is_first_and_named_correctly() {
        let first = ALL_MODELS
            .iter()
            .find(|m| matches!(m.model_type, ModelType::TextImageEmbedding))
            .expect("at least one TextImageEmbedding model");
        assert_eq!(first.id, "onnx-community/siglip2-base-patch32-256-ONNX", "SIGLIP_FAST (Blitz) must be first");
        assert_eq!(first.display_name, "Blitz");
    }

    #[test]
    fn standard_smart_search_model_is_named_correctly() {
        let standard = ALL_MODELS
            .iter()
            .filter(|m| matches!(m.model_type, ModelType::TextImageEmbedding))
            .nth(1)
            .expect("second TextImageEmbedding model");
        assert_eq!(standard.id, "onnx-community/siglip2-base-patch16-224-ONNX");
        assert_eq!(standard.display_name, "Standard");
    }

    #[test]
    fn standard_smart_search_model_has_split_towers() {
        let s = &SIGLIP_BASE;
        assert!(s.vision_file.is_some(), "vision tower must be configured");
        assert!(s.text_file.is_some(), "text tower must be configured");
        assert_eq!(s.vision_input, "pixel_values");
        assert!(!s.vision_output.is_empty());
        assert!(!s.text_output.is_empty());
    }

    #[test]
    fn gender_age_model_uses_insightface_cache_dir() {
        assert_eq!(BUFFALO_S_GENDER_AGE.cache_dir, "insightface");
    }

    #[test]
    fn only_two_face_presets_registered() {
        assert_eq!(ALL_PRESETS.len(), 2);
    }

    #[test]
    fn all_presets_are_registered_and_findable() {
        for preset in ALL_PRESETS {
            let found = FaceIdPreset::find_by_id(preset.id);
            assert!(found.is_some(), "preset '{}' not findable by id", preset.id);
        }
        assert!(FaceIdPreset::find_by_id("blitz").is_some());
        assert!(FaceIdPreset::find_by_id("quality").is_none(), "quality preset must be removed");
        assert!(FaceIdPreset::find_by_id("precision").is_some());
    }

    #[test]
    fn standard_face_preset_is_named_standard() {
        let preset = FaceIdPreset::find_by_id("precision").unwrap();
        assert_eq!(preset.name, "Standard");
        assert_eq!(preset.embedder.hf_repo, "immich-app/antelopev2");
        assert_eq!(preset.detector.hf_repo, "immich-app/antelopev2");
        assert_eq!(preset.gender_age.id, "buffalo_s_gender_age");
        assert!(preset.embedder.size_bytes > 200_000_000);
    }

    #[test]
    fn buffalo_l_models_not_in_all_models() {
        let ids: Vec<_> = ALL_MODELS.iter().map(|m| m.id).collect();
        assert!(!ids.contains(&"buffalo_l_detection"), "buffalo_l must be removed");
        assert!(!ids.contains(&"buffalo_l_recognition"), "buffalo_l must be removed");
        assert!(ids.contains(&"antelopev2_detection"));
        assert!(ids.contains(&"antelopev2_recognition"));
    }
}
