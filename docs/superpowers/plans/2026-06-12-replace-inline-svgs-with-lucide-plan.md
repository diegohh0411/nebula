# Replace Inline SVG Icons with Lucide Icons Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all inline SVG icons in `sidebar`, `search-bar`, `gallery`, and `photo-grid` components with `lucide-angular` components.

**Architecture:** Update component imports to include `LucideAngularModule`, then replace inline `<svg>` markup with `<lucide-icon>` components, mapping sizes and attributes accordingly.

**Tech Stack:** Angular, Lucide Angular (`lucide-angular`).

---

### Task 1: Update Sidebar Component

**Files:**
- Modify: `src/app/components/sidebar/sidebar.component.html`
- Modify: `src/app/components/sidebar/sidebar.component.ts`

- [ ] **Step 1: Replace inline SVGs in `sidebar.component.html`**
  - "All Photos": Use `<lucide-icon name="images" size="14"></lucide-icon>`
  - "People": Use `<lucide-icon name="users" size="14"></lucide-icon>`
  - "Tags": Use `<lucide-icon name="tag" size="14"></lucide-icon>`
  - "Folder": Use `<lucide-icon name="folder" size="14"></lucide-icon>`
  - "Remove folder": Use `<lucide-icon name="x" size="12"></lucide-icon>`
  - "Add Folder": Use `<lucide-icon name="plus" size="14"></lucide-icon>`

- [ ] **Step 2: Run tests to verify sidebar changes**
  - Run: `pnpm test`
  - Expected: PASS

- [ ] **Step 3: Commit**
  - Commit: `feat: replace inline SVG icons with Lucide icons in Sidebar`

### Task 2: Update Search Bar Component

**Files:**
- Modify: `src/app/components/search-bar/search-bar.component.html`
- Modify: `src/app/components/search-bar/search-bar.component.ts`

- [ ] **Step 1: Import `LucideAngularModule` in `search-bar.component.ts`**

- [ ] **Step 2: Replace inline SVGs in `search-bar.component.html`**
  - Search icon (width 12): `<lucide-icon name="search" size="12"></lucide-icon>`
  - Search icon (width 14): `<lucide-icon name="search" size="14"></lucide-icon>`
  - Clear icon (width 14): `<lucide-icon name="x" size="14"></lucide-icon>`

- [ ] **Step 3: Run tests to verify search-bar changes**
  - Run: `pnpm test`
  - Expected: PASS

- [ ] **Step 4: Commit**
  - Commit: `feat: replace inline SVG icons with Lucide icons in Search Bar`

### Task 3: Update Gallery Component

**Files:**
- Modify: `src/app/components/gallery/gallery.component.html`
- Modify: `src/app/components/gallery/gallery.component.ts`

- [ ] **Step 1: Import `LucideAngularModule` in `gallery.component.ts`**

- [ ] **Step 2: Replace inline SVG in `gallery.component.html`**
  - Chevron: Use `<lucide-icon name="chevron-down" size="16"></lucide-icon>` (size approximation, adjust if needed)

- [ ] **Step 3: Run tests to verify gallery changes**
  - Run: `pnpm test`
  - Expected: PASS

- [ ] **Step 4: Commit**
  - Commit: `feat: replace inline SVG icons with Lucide icons in Gallery`

### Task 4: Update Photo Grid Component

**Files:**
- Modify: `src/app/components/photo-grid/photo-grid.component.html`
- Modify: `src/app/components/photo-grid/photo-grid.component.ts`

- [ ] **Step 1: Import `LucideAngularModule` in `photo-grid.component.ts`**

- [ ] **Step 2: Replace inline SVG in `photo-grid.component.html`**
  - Placeholder: Use `<lucide-icon name="image" size="24"></lucide-icon>`

- [ ] **Step 3: Run tests to verify photo-grid changes**
  - Run: `pnpm test`
  - Expected: PASS

- [ ] **Step 4: Commit**
  - Commit: `feat: replace inline SVG icons with Lucide icons in Photo Grid`
