# Settings & Model Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a Settings route to allow users to switch between different vision embedding models and trigger a reindex.

**Architecture:** Use a new `settings` table in SQLite for persistence. Create a dedicated `settings.rs` Tauri module. Update `VisionEngine` to load models dynamically. Add an Angular `/settings` route with a categorized UI using Lucide icons and Spartan UI components.

**Tech Stack:** Rust (Tauri, SQLite, ORT), Angular (TypeScript, Spartan UI, Lucide Icons, Tailwind CSS).

---

### Task 1: Database Schema Update

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Update SQL initialization script**

Modify the database initialization logic to create a `settings` table and seed the default model.

```rust
// Inside src-tauri/src/db.rs (init_db or similar function)
let init_sql = "
    CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    INSERT OR IGNORE INTO settings (key, value) VALUES ('embedding_model', 'diegohh/siglip2-base-patch16-224');
";
// ... execution logic ...
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(db): add settings table and default embedding_model"
```

### Task 2: Settings Backend Module

**Files:**
- Create: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Implement settings commands**

Create `src-tauri/src/settings.rs` with the following commands:

```rust
use tauri::{command, AppHandle};
use crate::db;
use serde::Serialize;

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[command]
pub fn get_available_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "diegohh/siglip2-base-patch16-224".into(),
            name: "Standard".into(),
            description: "Balanced quality and speed (86M params)".into(),
        },
        ModelInfo {
            id: "onnx-community/siglip2-base-patch32-256-ONNX".into(),
            name: "Fast".into(),
            description: "Optimized for consumer CPUs with larger patches".into(),
        },
    ]
}

#[command]
pub fn get_setting(app: AppHandle, key: String) -> Result<Option<String>, String> {
    // Implement database read for the key
    unimplemented!()
}

#[command]
pub fn update_setting(app: AppHandle, key: String, value: String) -> Result<(), String> {
    // Implement database write for the key
    unimplemented!()
}
```

- [ ] **Step 2: Register commands in lib.rs**

```rust
// In src-tauri/src/lib.rs
mod settings;

// ... inside run() builder ...
.invoke_handler(tauri::generate_handler![
    settings::get_available_models,
    settings::get_setting,
    settings::update_setting,
    // ... existing commands ...
])
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/settings.rs src-tauri/src/lib.rs
git commit -m "feat(backend): implement settings module and commands"
```

### Task 3: Vision Engine Dynamic Loading

**Files:**
- Modify: `src-tauri/src/vision_engine.rs`

- [ ] **Step 1: Refactor VisionEngine to use dynamic repo**

Replace hardcoded `HF_REPO` with logic that reads from `settings` table or a passed parameter during `ensure_model_ready`.

```rust
// In src-tauri/src/vision_engine.rs
pub async fn ensure_model_ready(&self, app: &AppHandle, model_id: &str) -> Result<()> {
    // Use model_id to determine download path and sub-directory
    // ...
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/vision_engine.rs
git commit -m "refactor(vision): allow dynamic model loading in VisionEngine"
```

### Task 4: Angular Settings Component & Routing

**Files:**
- Create: `src/app/components/settings/settings.component.ts`
- Create: `src/app/components/settings/settings.component.html`
- Create: `src/app/components/settings/settings.component.css`
- Modify: `src/app/app.routes.ts`

- [ ] **Step 1: Add route to app.routes.ts**

```typescript
import { SettingsComponent } from "./components/settings/settings.component";

export const routes: Routes = [
  // ... existing routes ...
  { path: "settings", component: SettingsComponent },
];
```

- [ ] **Step 2: Scaffolding SettingsComponent**

Implement the UI with Lucide icons (`Settings`, `Cpu`) and the model list cards.

- [ ] **Step 3: Commit**

```bash
git add src/app/app.routes.ts src/app/components/settings/
git commit -m "feat(frontend): add settings route and component scaffolding"
```

### Task 4: Sidebar Navigation

**Files:**
- Modify: `src/app/components/sidebar/sidebar.component.html`
- Modify: `src/app/components/sidebar/sidebar.component.ts`

- [ ] **Step 1: Add Settings link to sidebar**

Add a bottom-aligned settings button using Lucide icon.

```html
<!-- At bottom of sidebar.component.html -->
<div class="sidebar-footer mt-auto p-4 border-t border-border">
  <a routerLink="/settings" class="flex items-center gap-2 text-muted-foreground hover:text-foreground">
    <hlm-icon name="lucideSettings" class="w-4 h-4" />
    <span>Settings</span>
  </a>
</div>
```

- [ ] **Step 2: Commit**

```bash
git add src/app/components/sidebar/
git commit -m "feat(ui): add settings link to sidebar"
```

### Task 5: Model Selection Logic & Reindexing

**Files:**
- Modify: `src/app/components/settings/settings.component.ts`
- Modify: `src-tauri/src/commands.rs` (or reindexing logic)

- [ ] **Step 1: Implement Type-to-Confirm Dialog**

Use Spartan UI dialog/popover to require "REINDEX" input before calling `update_setting`.

- [ ] **Step 2: Implement Backend Wipe & Reindex Trigger**

When `update_setting` is called for `embedding_model`, clear the `vectors` table and signal the indexer to restart.

- [ ] **Step 3: Commit**

```bash
git add src/app/components/settings/ src-tauri/src/
git commit -m "feat(settings): implement reindexing logic on model change"
```
