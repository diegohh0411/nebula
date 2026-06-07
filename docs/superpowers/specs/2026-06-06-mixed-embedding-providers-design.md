# Mixed Embedding Provider Support Design

**Date:** 2026-06-06  
**Status:** Approved  
**Approach:** Provider Abstraction Layer (Approach 1)

## 1. Problem Statement

Currently, Nebula supports only offline ONNX embedding models (SigLIP variants) for semantic image search. This is great for offline use but limits embedding quality and flexibility. We want to add support for cloud-based multi-modal embedding providers (starting with OpenRouter/Gemini Embeddings) while maintaining full offline capability and ensuring the processing queue is resilient to network hiccups.

## 2. Goals

- Support multiple embedding providers: local ONNX (SigLIP) and cloud-based (OpenRouter/Gemini)
- Only one active provider at a time per embedding space
- Processing queue resilient to transient network failures (retry with exponential backoff)
- Permanently failed items have clear UX paths for retry
- Clean architecture: adding a new provider should be ~1 file + registration

## 3. Non-Goals

- Multiple concurrent embedding spaces/dimensions (out of scope)
- Adapter/projection layers between different embedding dimensions
- Streaming/chunked embedding processing
- Billing or usage tracking for API calls

## 4. Architecture

### 4.1 Provider Trait

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Human-readable identifier (e.g., "local_siglip_base", "openrouter_gemini")
    fn name(&self) -> &'static str;
    
    /// Vector dimensionality
    fn dimension(&self) -> usize;
    
    /// Whether this provider needs network connectivity
    fn requires_network(&self) -> bool;
    
    /// Check readiness (models cached, API key valid, etc.)
    async fn is_ready(&self) -> Result<bool, String>;
    
    /// Embed a batch of images
    async fn embed_images(
        &self, 
        images: Vec<DynamicImage>
    ) -> Result<Vec<Vec<f32>>, ProviderError>;
    
    /// Embed a single text query
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, ProviderError>;
}
```

### 4.2 Error Types

```rust
pub enum ProviderError {
    /// Transient: retry with backoff
    Network(reqwest::Error),
    /// Transient: rate limited, retry after delay
    RateLimited { retry_after_secs: u64 },
    /// Permanent: bad input, don't retry
    InvalidInput(String),
    /// Permanent: misconfigured (missing API key, etc.)
    Configuration(String),
    /// Permanent: unexpected response format
    UnexpectedResponse(String),
}
```

### 4.3 Provider Registry

A static registry maps provider names to factory functions:

```rust
lazy_static! {
    static ref PROVIDERS: HashMap<&'static str, Box<dyn Fn() -> Box<dyn EmbeddingProvider>>> = {
        let mut m = HashMap::new();
        m.insert("local_siglip_base", Box::new(|| Box::new(LocalOnnxProvider::new(SIGLIP_BASE))));
        m.insert("openrouter_gemini", Box::new(|| Box::new(OpenRouterProvider::new())));
        m
    };
}
```

### 4.4 Provider Implementations

#### LocalOnnxProvider
- Wraps existing `VisionEngine` + `ModelManager`
- `is_ready()` → `ModelManager::ensure_ready()` (downloads from HF if needed)
- `embed_images()` → `VisionEngine::embed_images_batch()` via `spawn_blocking`
- `embed_text()` → `VisionEngine::embed_text()` via `spawn_blocking`
- `requires_network()` → `false` (after initial download)

#### OpenRouterProvider
- New implementation using `reqwest`
- Configuration from settings:
  - `openrouter_api_key`
  - `openrouter_model_id` (default: `google/gemini-embedding-exp-03-07`)
- `is_ready()` → validates API key format + lightweight health check
- `embed_images()` → base64 encodes images, sends to `https://openrouter.ai/api/v1/embeddings`
- `embed_text()` → sends text to same endpoint
- `requires_network()` → `true`

**Retry logic (built into OpenRouterProvider):**
```rust
const MAX_RETRIES: u32 = 5;
const BASE_BACKOFF_SECS: u64 = 2;

// retry_after = min(BASE_BACKOFF_SECS * 2^attempt, 300) // 5 min cap
```

## 5. Database Schema

Modify initial migration (no ALTERs — we are alpha):

```sql
CREATE TABLE embedding_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id INTEGER NOT NULL REFERENCES images(id),
    pipeline TEXT NOT NULL CHECK(pipeline IN ('semantic', 'subject')),
    provider TEXT NOT NULL,  -- No default; set by Rust code
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    scheduled_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    permanent_error TEXT  -- NULL = transient, set = permanent failure reason
);
```

**Default provider:** Set by Rust code when enqueuing:
```rust
let provider = get_setting("embedding_provider")
    .unwrap_or_else(|| DEFAULT_PROVIDER.to_string());
```

## 6. Pipeline & Queue Behavior

### 6.1 Normal Processing

1. Pull from queue: `SELECT ... WHERE provider = ? AND scheduled_at <= now()`
2. Process batch through active provider
3. On success: remove from queue, write embeddings, update vector index
4. On transient error (Network, RateLimited):
   - Increment `attempts`
   - Set `scheduled_at = now() + backoff`
   - Item remains in queue for retry
5. On permanent error (InvalidInput, Configuration, UnexpectedResponse):
   - Set `permanent_error` to error reason
   - Item is skipped by normal processing

### 6.2 Offline Handling

- Before starting pipeline, if `provider.requires_network()`:
  - Check network connectivity (lightweight HTTP HEAD to a reliable endpoint)
  - If offline: don't start pipeline, show "Waiting for network..." status
  - Poll connectivity every 30 seconds
- When network returns: resume pipeline normally
- Already-enqueued items with cloud provider are simply not processed while offline (they stay in queue with future `scheduled_at`)

### 6.3 Provider Switch

1. User selects new provider in Settings
2. Backend:
   - Stop current pipeline
   - Clear `embedding` column in `images` table
   - Clear in-memory vector index
   - `reset_all_embeddings()` re-enqueues all images with new provider name
   - Start pipeline with new provider
3. Old provider items in queue are ignored (filtered by provider name)

## 7. Permanently Failed Items — Retry UX

### 7.1 Auto-Retry on App Restart

- On app startup, check for items with `permanent_error` where the error type might be recoverable
- If the provider is now ready (e.g., API key was added), clear `permanent_error` and reset `attempts = 0`
- Items are naturally picked up by the pipeline

### 7.2 Manual Retry in Settings

New "Failed Embeddings" section in Settings UI:
- Shows count of permanently failed items
- Breakdown by reason: "5 × Invalid image", "2 × API configuration error"
- **"Retry All"** button: clears `permanent_error` and resets `attempts = 0` for all failed items
- **"Clear Failed"** button: removes failed items from queue (user acknowledges they'll never be embedded)

### 7.3 Per-Image Retry in Lightbox

- In the photo detail/lightbox view, if an image has no embedding:
  - Show indicator: "Embedding failed: [reason]"
  - Button: **"Retry embedding"**
  - Clicking it clears `permanent_error` for that image's queue item and resets `attempts = 0`

## 8. Settings & UI Changes

### 8.1 New Settings

- `embedding_provider`: `"local_siglip_base"` | `"openrouter_gemini"`
- `openrouter_api_key`: string (Tauri secure storage)
- `openrouter_model_id`: `"google/gemini-embedding-exp-03-07"` (default, overridable)

### 8.2 Frontend Changes

**Settings page (`settings.component.ts`):**
- Provider selector with two sections:
  - **Local (Offline):** SigLIP Base, SigLIP Fast
  - **Cloud (Requires internet):** Gemini Embedding (via OpenRouter)
- When cloud provider selected:
  - Show API key input (if not saved)
  - Show model ID (editable, with default)
- Show provider status: "Ready", "Downloading models...", "Waiting for network..."
- **New:** "Failed Embeddings" section with retry/clear buttons

**Lightbox/Detail view:**
- Show embedding status indicator
- Retry button for failed embeddings

## 9. File Structure

```
src-tauri/src/
├── providers/
│   ├── mod.rs              # EmbeddingProvider trait, ProviderError, registry
│   ├── local_onnx.rs       # LocalOnnxProvider implementation
│   └── openrouter.rs       # OpenRouterProvider implementation
├── pipeline/
│   ├── mod.rs              # Updated to use provider trait
│   └── embed_actor.rs      # Updated to use provider trait
├── db.rs                   # Updated queue schema, provider filtering
├── settings.rs             # New provider settings
└── commands.rs             # Search command uses active provider
```

## 10. Testing Strategy

- **Unit tests:** Mock `EmbeddingProvider` trait, test retry logic with mock HTTP responses
- **Integration tests:** Test provider switch triggers re-indexing
- **Offline simulation:** Mock network failures, verify backoff and retry behavior
- **Error injection:** Simulate rate limits, bad API keys, malformed responses

## 11. Open Questions

- Should we support OpenRouter text embeddings only first, then add image embeddings later? (No — we're doing multi-modal from the start)
- What should happen if a user switches providers while items are actively processing? (Stop pipeline, re-enqueue remaining with new provider)
- Do we need a queue item priority system so manual retries are processed first? (Future enhancement)
