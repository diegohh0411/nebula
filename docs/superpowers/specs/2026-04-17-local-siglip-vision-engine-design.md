# Design Spec: Local SigLIP 2 Vision Engine

This document outlines the architecture for replacing cloud-based Gemini embeddings with a local, private SigLIP 2 implementation using ONNX Runtime (ORT).

## 1. Context & Goals
Currently, Nebula uses Google's Gemini API for global image embeddings and text-to-image search. While face recognition is local, general visual search requires an internet connection and an API key.

### Goals:
*   **Privacy:** 100% offline visual search and indexing.
*   **Performance:** Optimized CPU inference for "Standard" SigLIP 2 models (~1.5GB).
*   **Consistency:** Unified management of all AI models (Face-ID + SigLIP).
*   **Sovereignty:** No reliance on external APIs or keys.

## 2. Architecture: The `VisionEngine` Service

We will refactor the current fragmented AI logic into a unified `VisionEngine`.

### Core Components:
*   **`VisionEngine` Struct:** A thread-safe, long-lived service managed as Tauri App State.
*   **Lazy Loading:** Models are loaded into RAM only when the first inference task (indexing or search) is triggered.
*   **Session Management:**
    *   **Face-ID Session:** Existing ArcFace model for person recognition.
    *   **SigLIP Image Encoder:** For generating library embeddings.
    *   **SigLIP Text Encoder:** For converting search queries into vectors.

### Preprocessing Pipeline:
SigLIP 2 requires specific image preprocessing before inference:
1.  **Resize:** Scale to the model's expected input size (e.g., 224x224 or 384x384).
2.  **Center Crop:** Maintain aspect ratio while focusing on the center.
3.  **Normalization:** Map pixel values to the mean/std-dev expected by the SigLIP 2 training set.

## 3. Implementation Details

### Model Source
We will use `hf-hub` to download quantized (INT8/FP16) SigLIP 2 ONNX models from Hugging Face on the first run. The models will be cached in the app's data directory.

### Integration with `ort`
We will configure `ort` with:
*   **Intra-op threads:** Scaled to the user's CPU core count for faster single-image processing.
*   **Inter-op threads:** Limited to prevent system-wide lag during heavy indexing.
*   **Execution Providers:** Default to CPU, but prepared for future OpenVINO/DirectML/CoreML extensions.

## 4. Migration & Data Flow

### Database Reset
Since Gemini vectors and SigLIP vectors are in different "vector spaces" (different dimensions and training), a migration is required:
1.  **Reset Status:** Set `images.embed_status = 'pending'` for all records.
2.  **Clear Embeddings:** Set `images.embedding = NULL`.
3.  **Dimension Update:** The search logic must be updated to handle the new vector length (likely 768 or 1152).

### Indexing Loop
The `embedder.rs` background worker will be updated to:
1.  Acquire a permit from the `VisionEngine`.
2.  Preprocess the image locally.
3.  Run the SigLIP Image Encoder.
4.  Store the result in the `images` table.

## 5. Success Criteria
*   User can perform text-to-image search while Airplane Mode is on.
*   Search results for complex queries (e.g., "a person wearing a blue hat") are accurate.
*   Indexing speed on a standard CPU is at least 1-2 images per second.
*   The app does not crash or exhaust RAM on 8GB systems during indexing.
