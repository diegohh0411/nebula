# Settings Page Redesign (TT-2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the first-run active-model bug, add download-status indicators to model cards, and rename the two settings sections with consumer-friendly names and icons.

**Architecture:** `ModelInfo` gains `downloaded: bool` and `size_bytes: u64` fields computed at call time from the filesystem; `get_setting` returns a real default instead of null so the frontend never needs hardcoded fallback IDs; the frontend renders three distinct card states (active, downloaded, needs-download) and updated section labels/icons.

**Tech Stack:** Rust/Tauri backend, Angular 17+ standalone components, Lucide Angular icons, Tailwind CSS via `hlm` utility classes.

---

## File Map

| File | Change |
|---|---|
| `src/app/app.config.ts` | Add `Sparkles`, `ScanFace`, `HardDrive`, `Download` to icon pick list |
| `src-tauri/src/models/registry.rs` | Add `size_bytes: u64` field to `ModelSpec` struct + values for all 5 constants |
| `src-tauri/src/settings.rs` | Extend `ModelInfo`; add `downloaded`/`size_bytes` helpers; change `get_setting` return type from `Option<String>` to `String` with real defaults |
| `src/app/components/settings/settings.component.ts` | Update `ModelInfo` interface, drop hardcoded fallbacks in `loadSettings`, add `formatBytes` |
| `src/app/components/settings/settings.component.html` | Rename sections, swap icons, add three-state card indicator, update modal title |
| `src/app/components/settings/settings.component.css` | Add `.download-chip` style |

---

## Task 1: Register new Lucide icons

**Files:**
- Modify: `src/app/app.config.ts`

- [ ] **Step 1: Add icons to the import and pick list**

Open `src/app/app.config.ts`. The current line 4 and the `LucideAngularModule.pick(...)` call look like:

```typescript
import { LucideAngularModule, Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus, Settings, Cpu, AlertTriangle } from 'lucide-angular';
```
```typescript
importProvidersFrom(LucideAngularModule.pick({ Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus, Settings, Cpu, AlertTriangle })),
```

Replace both with:

```typescript
import { LucideAngularModule, Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus, Settings, Cpu, AlertTriangle, Sparkles, ScanFace, HardDrive, Download } from 'lucide-angular';
```
```typescript
importProvidersFrom(LucideAngularModule.pick({ Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus, Settings, Cpu, AlertTriangle, Sparkles, ScanFace, HardDrive, Download })),
```

- [ ] **Step 2: Check for TypeScript errors**

```bash
cd /home/user/nebula && npx tsc --noEmit 2>&1 | tail -10
```

Expected: no output (no errors).

- [ ] **Step 3: Commit**

```bash
git add src/app/app.config.ts
git commit -m "feat(ui): register Sparkles, ScanFace, HardDrive, Download icons"
```

---

## Task 2: Add `size_bytes` to `ModelSpec`

**Files:**
- Modify: `src-tauri/src/models/registry.rs`

- [ ] **Step 1: Add the field to the struct**

In `src-tauri/src/models/registry.rs`, the `ModelSpec` struct ends with `pub text_output: &'static str`. Add one more field directly after it:

```rust
  /// Approximate total download size in bytes across all model files
  pub size_bytes: u64,
```

- [ ] **Step 2: Populate `size_bytes` on `SIGLIP_BASE`**

`SIGLIP_BASE` currently ends with `text_output: "pooler_output",`. Add:

```rust
  size_bytes: 660_000_000, // vision_model.onnx (~360 MB) + text_model.onnx (~270 MB) + tokenizer.json
```

> **Note for implementer:** Verify the actual HuggingFace file sizes before shipping. Run `curl -sI https://huggingface.co/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/onnx/vision_model.onnx | grep content-length` for each file and sum them.

- [ ] **Step 3: Populate `size_bytes` on `SIGLIP_FAST`**

After `SIGLIP_FAST`'s `text_output: "pooler_output",` add:

```rust
  size_bytes: 169_000_000, // vision_model_quantized.onnx (~90 MB) + text_model_quantized.onnx (~70 MB) + tokenizer.json
```

- [ ] **Step 4: Populate `size_bytes` on the three Buffalo models**

After each `text_output: "",`:

```rust
// BUFFALO_S_RECOGNITION
  size_bytes: 20_000_000, // recognition.onnx (~19 MB)

// BUFFALO_S_DETECTION
  size_bytes: 4_000_000, // detection.onnx (~4 MB)

// BUFFALO_S_GENDER_AGE
  size_bytes: 1_300_000, // genderage.onnx (~1.3 MB)
```

- [ ] **Step 5: Build to confirm it compiles**

```bash
cd /home/user/nebula && cargo build -p nebula 2>&1 | tail -5
```

Expected: `Finished dev [unoptimized + debuginfo] target(s)`.

- [ ] **Step 6: Run existing registry tests**

```bash
cd /home/user/nebula && cargo test -p nebula models::registry 2>&1 | tail -10
```

Expected: all existing tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/models/registry.rs
git commit -m "feat(models): add size_bytes to ModelSpec"
```

---

## Task 3: Backend — extend `ModelInfo`, add download helpers, fix `get_setting` defaults

**Files:**
- Modify: `src-tauri/src/settings.rs`

The bug: `get_setting` returns `None` when a key has never been written. The frontend fell back to hardcoded strings (`'diegohh/siglip2-base-patch16-224'` and `'standard'`) that don't match any actual model ID (`'onnx-community/siglip2-base-patch16-224-ONNX'` and `'blitz'`), so no card was ever highlighted.

- [ ] **Step 1: Write two unit tests that pin the expected defaults**

Append to `src-tauri/src/settings.rs` (before the final `}`):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn default_embedding_model_matches_first_text_image_model() {
        let first = crate::models::registry::ALL_MODELS
            .iter()
            .find(|m| matches!(m.model_type, crate::models::registry::ModelType::TextImageEmbedding))
            .map(|m| m.id);
        assert_eq!(first, Some("onnx-community/siglip2-base-patch16-224-ONNX"));
    }

    #[test]
    fn default_subject_model_matches_first_preset() {
        let first = crate::models::registry::ALL_PRESETS.first().map(|p| p.id);
        assert_eq!(first, Some("blitz"));
    }
}
```

- [ ] **Step 2: Run the tests — they should pass against existing data**

```bash
cd /home/user/nebula && cargo test -p nebula settings 2>&1 | tail -10
```

Expected: `test settings::tests::default_embedding_model_matches_first_text_image_model ... ok` and `test settings::tests::default_subject_model_matches_first_preset ... ok`.

- [ ] **Step 3: Replace `settings.rs` with the full updated version**

Replace the entire contents of `src-tauri/src/settings.rs` with:

```rust
use tauri::{command, State};
use sqlx::Row;
use serde::Serialize;
use crate::AppState;
use crate::models::registry::{ModelSpec, FaceIdPreset, ModelType};

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub downloaded: bool,
    pub size_bytes: u64,
}

fn spec_downloaded(state: &AppState, spec: &ModelSpec) -> bool {
    let dir = state.model_manager.model_dir(spec);
    spec.all_files().iter().all(|f| dir.join(f.filename).exists())
}

fn preset_downloaded(state: &AppState, preset: &FaceIdPreset) -> bool {
    spec_downloaded(state, preset.detector)
        && spec_downloaded(state, preset.embedder)
        && spec_downloaded(state, preset.gender_age)
}

fn preset_size_bytes(preset: &FaceIdPreset) -> u64 {
    preset.detector.size_bytes + preset.embedder.size_bytes + preset.gender_age.size_bytes
}

#[command]
pub fn get_available_models(state: State<'_, AppState>) -> Vec<ModelInfo> {
    crate::models::registry::ALL_MODELS
        .iter()
        .filter(|m| matches!(m.model_type, ModelType::TextImageEmbedding))
        .map(|m| ModelInfo {
            id: m.id.to_string(),
            name: m.display_name.to_string(),
            description: m.display_description.to_string(),
            downloaded: spec_downloaded(&state, m),
            size_bytes: m.size_bytes,
        })
        .collect()
}

#[command]
pub fn get_available_subject_models(state: State<'_, AppState>) -> Vec<ModelInfo> {
    crate::models::registry::ALL_PRESETS
        .iter()
        .map(|p| ModelInfo {
            id: p.id.to_string(),
            name: p.name.to_string(),
            description: p.description.to_string(),
            downloaded: preset_downloaded(&state, p),
            size_bytes: preset_size_bytes(p),
        })
        .collect()
}

#[command]
pub async fn get_setting(state: State<'_, AppState>, key: String) -> Result<String, String> {
    let pool = &state.pool;
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(&key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(r) = row {
        return Ok(r.get("value"));
    }

    match key.as_str() {
        "embedding_model" => crate::models::registry::ALL_MODELS
            .iter()
            .find(|m| matches!(m.model_type, ModelType::TextImageEmbedding))
            .map(|m| m.id.to_string())
            .ok_or_else(|| "No embedding models registered".to_string()),
        "subject_model" => crate::models::registry::ALL_PRESETS
            .first()
            .map(|p| p.id.to_string())
            .ok_or_else(|| "No subject model presets registered".to_string()),
        _ => Err(format!("No default for unknown setting key: {}", key)),
    }
}

#[command]
pub async fn update_setting(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let pool = &state.pool;

    if key == "embedding_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            let spec = crate::models::registry::ModelSpec::find_by_id(&value)
                .ok_or_else(|| format!("Unknown model: {}", value))?;
            state.model_manager.ensure_ready(&app, spec).await.map_err(|e| e.to_string())?;
            crate::db::reset_all_embeddings(pool).await.map_err(|e| e.to_string())?;
            if let Ok(mut idx) = state.index.write() {
                *idx = Box::new(crate::vector_index::FlatIndex::new(768));
            }
            let idx_path = state.data_dir.join("nebula.idx");
            let _ = std::fs::remove_file(idx_path);
        }
    }

    if key == "subject_model" {
        let current = crate::db::get_setting(pool, &key).await.unwrap_or(None);
        if current.as_ref() != Some(&value) {
            crate::db::reset_all_subject_data(pool).await.map_err(|e| e.to_string())?;
        }
    }

    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_embedding_model_matches_first_text_image_model() {
        let first = crate::models::registry::ALL_MODELS
            .iter()
            .find(|m| matches!(m.model_type, crate::models::registry::ModelType::TextImageEmbedding))
            .map(|m| m.id);
        assert_eq!(first, Some("onnx-community/siglip2-base-patch16-224-ONNX"));
    }

    #[test]
    fn default_subject_model_matches_first_preset() {
        let first = crate::models::registry::ALL_PRESETS.first().map(|p| p.id);
        assert_eq!(first, Some("blitz"));
    }
}
```

- [ ] **Step 4: Build to confirm it compiles**

```bash
cd /home/user/nebula && cargo build -p nebula 2>&1 | tail -5
```

Expected: `Finished dev [unoptimized + debuginfo] target(s)`.

- [ ] **Step 5: Run the settings tests**

```bash
cd /home/user/nebula && cargo test -p nebula settings 2>&1 | tail -10
```

Expected: both tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(settings): extend ModelInfo with downloaded+size_bytes, fix get_setting defaults"
```

---

## Task 4: Frontend TypeScript — update `ModelInfo` interface and `loadSettings`

**Files:**
- Modify: `src/app/components/settings/settings.component.ts`

- [ ] **Step 1: Update the `ModelInfo` interface (lines 18–22)**

Replace:

```typescript
interface ModelInfo {
  id: string;
  name: string;
  description: string;
}
```

With:

```typescript
interface ModelInfo {
  id: string;
  name: string;
  description: string;
  downloaded: boolean;
  size_bytes: number;
}
```

- [ ] **Step 2: Replace `loadSettings` (lines 102–115) — drop the hardcoded fallbacks**

Replace:

```typescript
  async loadSettings() {
    try {
      const model = await invoke<string | null>('get_setting', { key: 'embedding_model' });
      this.currentModel.set(model || 'diegohh/siglip2-base-patch16-224');
    } catch (e) {
      console.error('Failed to load settings:', e);
    }
    try {
      const subjectModel = await invoke<string | null>('get_setting', { key: 'subject_model' });
      this.currentSubjectModel.set(subjectModel || 'standard');
    } catch (e) {
      console.error('Failed to load subject model setting:', e);
    }
  }
```

With:

```typescript
  async loadSettings() {
    try {
      const model = await invoke<string>('get_setting', { key: 'embedding_model' });
      this.currentModel.set(model);
    } catch (e) {
      console.error('Failed to load embedding_model setting:', e);
    }
    try {
      const subjectModel = await invoke<string>('get_setting', { key: 'subject_model' });
      this.currentSubjectModel.set(subjectModel);
    } catch (e) {
      console.error('Failed to load subject_model setting:', e);
    }
  }
```

- [ ] **Step 3: Add `formatBytes` method after `loadSettings`**

Insert after the closing `}` of `loadSettings`:

```typescript
  formatBytes(bytes: number): string {
    if (bytes === 0) return '';
    if (bytes < 1_073_741_824) {
      return `${(bytes / 1_048_576).toFixed(1)} MB`;
    }
    return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
  }
```

- [ ] **Step 4: Check for TypeScript errors**

```bash
cd /home/user/nebula && npx tsc --noEmit 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/settings/settings.component.ts
git commit -m "feat(settings): update ModelInfo interface, simplify loadSettings, add formatBytes"
```

---

## Task 5: Frontend HTML — rename sections, update icons, add card status indicators

**Files:**
- Modify: `src/app/components/settings/settings.component.html`

Complete replacement. Tasks 5 and 6 must be done together before testing — the HTML uses `.download-chip` which Task 6 defines in CSS.

- [ ] **Step 1: Replace the full template**

Replace the entire contents of `src/app/components/settings/settings.component.html` with:

```html
<div class="settings-container">
  <header class="settings-header">
    <lucide-icon name="settings" class="header-icon"></lucide-icon>
    <h1>Settings</h1>
  </header>

  <div class="settings-content">
    <section class="settings-section">
      <div class="section-title">
        <lucide-icon name="sparkles" class="section-icon"></lucide-icon>
        <h2>Smart Search</h2>
      </div>
      <p class="section-description">Select the model used to understand and search your photos by meaning.</p>

      <div class="model-list">
        @for (model of models(); track model.id) {
          <div
            hlmCard
            class="cursor-pointer transition-all hover:border-ring"
            [class.active-card]="currentModel() === model.id"
            (click)="selectVisionModel(model.id)"
          >
            <div hlmCardHeader>
              <div class="flex items-center justify-between">
                <h3 hlmCardTitle>{{ model.name }}</h3>
                <div class="flex items-center gap-2">
                  @if (currentModel() === model.id) {
                    <span class="text-xs font-medium px-2 py-0.5 rounded-full border border-border bg-muted text-muted-foreground">Active</span>
                  } @else if (model.downloaded) {
                    <lucide-icon name="hard-drive" [size]="14" class="text-muted-foreground"></lucide-icon>
                  } @else {
                    <span class="download-chip">
                      <lucide-icon name="download" [size]="12"></lucide-icon>
                      {{ formatBytes(model.size_bytes) }}
                    </span>
                  }
                </div>
              </div>
              <p hlmCardDescription>{{ model.description }}</p>
            </div>
            <div hlmCardContent>
              <p class="model-id">{{ model.id }}</p>
            </div>
          </div>
        }
      </div>
    </section>

    <section class="settings-section">
      <div class="section-title">
        <lucide-icon name="scan-face" class="section-icon"></lucide-icon>
        <h2>People Recognition</h2>
      </div>
      <p class="section-description">Select the model used to detect and recognize people in your photos.</p>

      <div class="model-list">
        @for (model of subjectModels(); track model.id) {
          <div
            hlmCard
            class="cursor-pointer transition-all hover:border-ring"
            [class.active-card]="currentSubjectModel() === model.id"
            (click)="selectSubjectModel(model.id)"
          >
            <div hlmCardHeader>
              <div class="flex items-center justify-between">
                <h3 hlmCardTitle>{{ model.name }}</h3>
                <div class="flex items-center gap-2">
                  @if (currentSubjectModel() === model.id) {
                    <span class="text-xs font-medium px-2 py-0.5 rounded-full border border-border bg-muted text-muted-foreground">Active</span>
                  } @else if (model.downloaded) {
                    <lucide-icon name="hard-drive" [size]="14" class="text-muted-foreground"></lucide-icon>
                  } @else {
                    <span class="download-chip">
                      <lucide-icon name="download" [size]="12"></lucide-icon>
                      {{ formatBytes(model.size_bytes) }}
                    </span>
                  }
                </div>
              </div>
              <p hlmCardDescription>{{ model.description }}</p>
            </div>
          </div>
        }
      </div>
    </section>
  </div>

  @if (isConfirming()) {
    <div class="modal-backdrop" (click)="cancelSelection()">
      <div hlmCard class="modal-content" (click)="$event.stopPropagation()">
        <div hlmCardHeader>
          <div class="flex items-center gap-2 text-destructive">
            <lucide-icon name="alert-triangle" size="20"></lucide-icon>
            <h2 hlmCardTitle class="text-destructive">Change {{ pendingSection() === 'vision' ? 'Smart Search' : 'People Recognition' }} Model?</h2>
          </div>
          <p hlmCardDescription>This will trigger a full reindex of your library.</p>
        </div>

        <div hlmCardContent class="space-y-4">
          @if (isProcessing()) {
            <div class="space-y-3">
              <div class="flex justify-between text-xs font-medium">
                <span>{{ processingPhase() === 'downloading' ? 'Downloading model...' : 'Reindexing library...' }}</span>
                @if (processingPhase() === 'downloading') {
                  <span>{{ downloadProgress() | number:'1.0-0' }}%</span>
                }
              </div>
              @if (processingPhase() === 'downloading') {
                <div class="progress-bar-container">
                  <div class="progress-bar-fill" [style.width.%]="downloadProgress() ?? 0"></div>
                </div>
                <p class="text-[10px] text-muted-foreground truncate">{{ currentDownloadFile() }}</p>
              }
            </div>
          } @else {
            <div class="bg-muted/50 p-3 rounded text-sm space-y-2 border border-border">
              @if (pendingSection() === 'vision') {
                <p>&#8226; Semantic embeddings and the search index will be rebuilt for your library.</p>
                <p>&#8226; Large libraries may take a long time to re-process.</p>
              } @else {
                <p>&#8226; All face and subject data will be wiped and rebuilt.</p>
                <p>&#8226; Large libraries may take a long time to re-process.</p>
              }
            </div>

            <div class="space-y-2">
              <p class="text-xs font-medium">Type <span class="font-bold">REINDEX</span> to confirm:</p>
              <input
                type="text"
                [value]="confirmInputValue()"
                (input)="confirmInputValue.set($any($event.target).value)"
                placeholder="REINDEX"
                class="confirm-input"
                autofocus
              >
            </div>
          }
        </div>

        <div hlmCardFooter class="flex justify-end gap-2">
          <button hlmBtn variant="secondary" (click)="cancelSelection()" [disabled]="isProcessing()">
            Cancel
          </button>
          <button
            hlmBtn
            variant="destructive"
            [disabled]="confirmInputValue() !== 'REINDEX' || isProcessing()"
            (click)="confirmSelection()"
          >
            Confirm Reindex
          </button>
        </div>
      </div>
    </div>
  }
</div>
```

- [ ] **Step 2: Do NOT commit yet — continue to Task 6 to add the CSS class first**

---

## Task 6: Frontend CSS — add `.download-chip` style

**Files:**
- Modify: `src/app/components/settings/settings.component.css`

- [ ] **Step 1: Append the download-chip class**

At the end of `src/app/components/settings/settings.component.css`, add:

```css
.download-chip {
  @apply flex items-center gap-1 text-xs text-muted-foreground px-2 py-0.5 rounded-full border border-border;
}
```

- [ ] **Step 2: Check for TypeScript errors**

```bash
cd /home/user/nebula && npx tsc --noEmit 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 3: Commit HTML and CSS together**

```bash
git add src/app/components/settings/settings.component.html src/app/components/settings/settings.component.css
git commit -m "feat(settings): rename sections, update icons, add card download status indicators"
```

---

## Task 7: Push the branch

- [ ] **Step 1: Push**

```bash
git push -u origin claude/epic-cannon-BztzV
```

Expected: branch pushed, no errors.
