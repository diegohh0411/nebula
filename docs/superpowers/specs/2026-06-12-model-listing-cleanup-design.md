# Model Listing Cleanup — Design

**Date:** 2026-06-12
**Branch:** fix-model-listing

## Goal

Reduce the model listings to one fast ("Blitz") and one standard ("Standard") option per inference type, give every model a user-friendly name and description, make Blitz the default for both inference types, and add a download-progress modal for subject/face model presets (currently missing).

## Scope

Three files change:

| File | What changes |
|---|---|
| `src-tauri/src/models/registry.rs` | Names, descriptions, ordering, removals |
| `src-tauri/src/settings/commands.rs` | Download subject preset models before DB reset |
| `src/app/components/settings/settings.component.ts` | Fix initial processing phase for both sections |

---

## 1. Registry (`registry.rs`)

### 1a. Smart Search models (TextImageEmbedding)

**`SIGLIP_FAST`** — becomes the Blitz option:
- `display_name`: `"Blitz"`
- `display_description`: `"Faster search with a small quality tradeoff. Good for large libraries and slower hardware."`

**`SIGLIP_BASE`** — stays the Standard option:
- `display_name`: `"Standard"` (no change)
- `display_description`: `"Best search accuracy. Recommended for most users."`

### 1b. Gender/age model

**`BUFFALO_S_GENDER_AGE`**:
- `cache_dir`: `"insightface"` (was `"buffalo_s"`)

Reason: this model is shared by all face presets (including Antelope V2) and its repo is `public-data/insightface`. Storing it in `buffalo_s/` is confusing when only the Antelope V2 preset is active. No migration; this is an alpha app and the ~1.3 MB file re-downloads on next use.

### 1c. Face presets

**`BUFFALO_S_PRESET`** — stays the Blitz option:
- `name`: `"Blitz"` (no change)
- `description`: `"Fastest face recognition. Ideal for large libraries."`

**`ANTELOPE_V2_PRESET`** — becomes the Standard option:
- `name`: `"Standard"` (was `"Precision"`)
- `description`: `"Highest-accuracy face recognition. Best for challenging photos with tricky lighting or angles."`

**`BUFFALO_L_PRESET`**, **`BUFFALO_L_DETECTION`**, **`BUFFALO_L_RECOGNITION`** — deleted entirely. No migration needed (alpha).

### 1d. List ordering

**`ALL_MODELS`** reordered — Blitz first, Standard second, face models after:
```
[SIGLIP_FAST, SIGLIP_BASE, BUFFALO_S_DETECTION, BUFFALO_S_RECOGNITION, BUFFALO_S_GENDER_AGE, ANTELOPE_V2_DETECTION, ANTELOPE_V2_RECOGNITION]
```

**`ALL_PRESETS`** — remove `BUFFALO_L_PRESET`; Blitz remains first:
```
[BUFFALO_S_PRESET, ANTELOPE_V2_PRESET]
```

### 1e. Default setting side-effect

`get_setting` for `"embedding_model"` returns the first `TextImageEmbedding` model in `ALL_MODELS`. After reordering, the default becomes `SIGLIP_FAST` ("Blitz"). This is intentional.

### 1f. Test updates

- `default_embedding_model_matches_first_text_image_model`: update expected ID to `onnx-community/siglip2-base-patch32-256-ONNX`.
- `all_face_models_in_all_models`: remove assertions for `buffalo_l_detection` / `buffalo_l_recognition`.
- `quality_preset_uses_buffalo_l_models`: delete.
- `precision_preset_uses_antelopev2_models`: delete (or repurpose for the new "Standard" preset).
- `all_presets_are_registered_and_findable`: remove `"precision"` assertion, replace with `"quality"` removed and both remaining IDs checked.

---

## 2. Subject model download (`settings/commands.rs`)

**Problem:** `update_setting` for `subject_model` skips `ensure_ready`, so face preset model files download silently in the background pipeline with no UI feedback. Switching to a preset whose ~280 MB files aren't cached gives the user no indication anything is happening.

**Fix:** Before resetting the DB, download all component models sequentially:

```rust
if key == "subject_model" {
    let current = ...;
    if current.as_ref() != Some(&value) {
        let preset = FaceIdPreset::find_by_id(&value)
            .ok_or_else(|| format!("Unknown preset: {}", value))?;
        state.model_manager.ensure_ready(&app, preset.detector).await...?;
        state.model_manager.ensure_ready(&app, preset.embedder).await...?;
        state.model_manager.ensure_ready(&app, preset.gender_age).await...?;
        people::repo::reset_all_subject_data(pool).await...?;
    }
}
```

`ensure_ready` already emits `model_download_progress` events per file. The frontend subscription handles multi-file downloads correctly: a `done=false` chunk event on the next file resets `processingPhase` back to `'downloading'` automatically.

No new event types needed.

---

## 3. Frontend processing phase (`settings.component.ts`)

**Problem:** `confirmSelection` hardcodes `processingPhase = 'reindexing'` for subject models, bypassing the download UI entirely.

**Fix:** For both sections, check the `downloaded` flag of the selected model before setting the initial phase:

```typescript
// vision section
const model = this.models().find(m => m.id === modelId);
this.processingPhase.set(model?.downloaded ? 'reindexing' : 'downloading');

// subject section
const model = this.subjectModels().find(m => m.id === modelId);
this.processingPhase.set(model?.downloaded ? 'reindexing' : 'downloading');
```

This also fixes the same latent issue for vision models: when switching to an already-cached SigLIP model the UI no longer briefly flashes "Downloading model…".

No HTML template changes needed.

---

## Non-goals

- No DB migration for users with `subject_model = "quality"` (alpha; acceptable breakage).
- No UI changes to remove the HuggingFace repo ID shown in Smart Search cards.
- No renaming of `BUFFALO_S_DETECTION` / `BUFFALO_S_RECOGNITION` cache dirs (those are legitimately stored in `buffalo_s/`).
