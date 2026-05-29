# Settings Page Redesign — TT-2

## Overview

The settings page has three problems to fix in one pass: a first-run bug where no active model is highlighted, missing download-status indicators on model cards, and inconsistent naming between the two model sections.

---

## 1. Bug Fix: Active Model Not Highlighted on First Run

**Root cause.** `get_setting` returns `null` when a key has never been written to the database. The frontend falls back to a hardcoded string (`'diegohh/siglip2-base-patch16-224'`), but if that string doesn't exactly match any `model.id` returned by `get_available_models()`, no card highlights.

**Fix.** Move the default logic to the backend. `get_setting` will return the system default when the key is absent rather than `null`. The frontend drops its hardcoded fallbacks entirely and trusts the backend to always return a valid ID.

- `embedding_model` default: the `id` of the first entry in the available models list
- `subject_model` default: `"standard"`

The frontend `loadSettings()` simplifies to: set `currentModel` and `currentSubjectModel` directly from what `get_setting` returns, with no fallback string.

---

## 2. Model Card Download Status Indicators

**Current state.** `ModelInfo` carries only `id`, `name`, and `description`. There is no UI signal distinguishing models already on disk from those that would require a download.

**Backend changes.** Extend the `ModelInfo` struct with two new fields:

```rust
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub downloaded: bool,   // all required files present on disk
    pub size_bytes: u64,    // total size of all model files (sum across spec)
}
```

`downloaded` is computed by checking whether all files in the model spec exist in the model cache directory (reusing the same logic as `ModelManager::ensure_ready()`). `size_bytes` is the sum of declared file sizes from the model spec (not a live disk scan).

**Frontend card states.** Three distinct visual states per card:

| State | Visual treatment |
|---|---|
| Active | "Active" badge (existing behaviour) |
| Downloaded, not active | `HardDrive` icon (small, muted) |
| Not downloaded | `Download` arrow icon + formatted size label (e.g. "1.2 GB") |

Size formatting: bytes → human-readable string in the frontend (`< 1 GB` → MB, `≥ 1 GB` → GB, one decimal place).

The `ModelInfo` interface in TypeScript gains the same two fields: `downloaded: boolean` and `size_bytes: number`.

---

## 3. Naming and Icons

**Section renames.**

| Old name | New name |
|---|---|
| Vision Model | Smart Search |
| Face Analysis | People Recognition |

**Section header icons.** Both section headers gain a Lucide icon (already imported via `LucideAngularModule`):

- Smart Search → `Sparkles`
- People Recognition → `ScanFace`

**Scope.** Changes are purely in the settings component template and any user-facing strings. Internal variable names (`models`, `subjectModels`, `currentModel`, `currentSubjectModel`), Tauri command names, and database keys are unchanged.

---

## Architecture

No new services or components. All changes are contained in:

- `src-tauri/src/settings.rs` — `ModelInfo` struct extension, default-value logic in `get_setting`
- `src-tauri/src/models/` — populate `downloaded` and `size_bytes` when building `ModelInfo`
- `src/app/components/settings/settings.component.ts` — extend `ModelInfo` interface, simplify `loadSettings()`
- `src/app/components/settings/settings.component.html` — rename headers, add icons, add card status indicators
- `src/app/components/settings/settings.component.css` — any new styles for the download/size badge

---

## Error Handling

- If `get_setting` fails (Tauri error), the frontend logs and leaves the signal `null`; no active card is shown. This is acceptable — it signals a deeper problem.
- If `size_bytes` is 0 for a non-downloaded model, the size label is omitted rather than showing "0 B".

---

## Testing

- First-run scenario: clear the settings database, open the settings page, verify the default model card is highlighted for both sections.
- Download indicator: a model that exists on disk shows `HardDrive`; one that doesn't shows `Download` + size.
- Naming: both section headers show "Smart Search" / "People Recognition" with the correct icons.
