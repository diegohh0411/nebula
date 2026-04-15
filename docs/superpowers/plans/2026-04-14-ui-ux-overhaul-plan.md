# Nebula UI/UX Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform Nebula's UI into a polished, "Google Photos" style experience with a justified grid, immersive transitions, and advanced navigation.

**Architecture:** Use the **View Transitions API** as the primary animation engine for shared-element transitions. Refactor the gallery to use a custom justified layout algorithm within the existing CDK Virtual Scroll infrastructure.

**Tech Stack:** Angular 20, Tailwind CSS, Lucide Icons, View Transitions API.

---

### Task 1: Infrastructure & Iconography

**Files:**
- Modify: `package.json`
- Modify: `src/app/app.config.ts`
- Create: `src/app/utils/view-transition.ts`

- [ ] **Step 1: Install dependencies**
Run: `npm install lucide-angular`

- [ ] **Step 2: Create View Transition utility**
Write the following to `src/app/utils/view-transition.ts`:
```typescript
export async function startViewTransition(callback: () => void | Promise<void>): Promise<void> {
  if (!(document as any).startViewTransition) {
    await callback();
    return;
  }
  return (document as any).startViewTransition(callback).finished;
}
```

- [ ] **Step 3: Register Lucide icons**
Modify `src/app/app.config.ts` to include `LucideAngularModule`:
```typescript
import { LucideAngularModule, Search, Info, X, ChevronLeft, ChevronRight } from 'lucide-angular';
// ... in providers array
importProvidersFrom(LucideAngularModule.pick({ Search, Info, X, ChevronLeft, ChevronRight })),
```

- [ ] **Step 4: Commit**
```bash
git add package.json src/app/app.config.ts src/app/utils/view-transition.ts
git commit -m "chore: install lucide icons and add view transition utility"
```

---

### Task 2: Justified Gallery Layout

**Files:**
- Create: `src/app/utils/justified-layout.ts`
- Modify: `src/app/services/photo.service.ts`
- Modify: `src/app/components/photo-grid/photo-grid.component.css`

- [ ] **Step 1: Implement justified layout utility**
Create `src/app/utils/justified-layout.ts` with logic to calculate image widths based on a target height and container width.

- [ ] **Step 2: Update PhotoService signals**
Modify `src/app/services/photo.service.ts` to use the justified layout logic in the `virtualRows` computed signal.

- [ ] **Step 3: Update CSS for flex-based rows**
Modify `src/app/components/photo-grid/photo-grid.component.css` to use `display: flex` and `flex-grow` instead of a fixed grid.

- [ ] **Step 4: Commit**
```bash
git add src/app/utils/justified-layout.ts src/app/services/photo.service.ts src/app/components/photo-grid/photo-grid.component.css
git commit -m "feat: implement justified gallery layout"
```

---

### Task 3: Immersive Lightbox Component

**Files:**
- Create: `src/app/components/lightbox/lightbox.component.ts`
- Create: `src/app/components/lightbox/lightbox.component.html`
- Create: `src/app/components/lightbox/lightbox.component.css`
- Modify: `src/app/components/gallery/gallery.component.html`
- Modify: `src/app/components/photo-grid/photo-grid.component.ts`

- [ ] **Step 1: Create Lightbox component**
Implement the full-screen overlay with Lucide icons (Close, Info, Search), horizontal navigation, and a sliding metadata sidebar.

- [ ] **Step 2: Add Shared-Element Transition logic**
In `PhotoGridComponent`, apply a unique `view-transition-name` to the clicked thumbnail before opening the lightbox.

- [ ] **Step 3: Integrate into Gallery**
Add `<app-lightbox>` to `gallery.component.html`, binding it to a `selectedImage` signal in `PhotoService`.

- [ ] **Step 4: Commit**
```bash
git add src/app/components/lightbox/ src/app/components/gallery/gallery.component.html src/app/components/photo-grid/
git commit -m "feat: add immersive lightbox with shared-element transitions"
```

---

### Task 4: Timeline Scrubber & Lasso Selection

**Files:**
- Create: `src/app/components/timeline-scrubber/timeline-scrubber.component.ts`
- Create: `src/app/components/timeline-scrubber/timeline-scrubber.component.html`
- Modify: `src/app/components/photo-grid/photo-grid.component.ts`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Implement Timeline Scrubber**
Create the vertical track component that maps drag position to dates in the virtual scroll viewport.

- [ ] **Step 2: Implement Lasso Selection**
In `PhotoGridComponent`, add pointer event listeners to track drag-selection and update the `selectedImages` signal in `PhotoService`.

- [ ] **Step 3: Commit**
```bash
git add src/app/components/timeline-scrubber/ src/app/components/photo-grid/ src/app/services/photo.service.ts
git commit -m "feat: add timeline scrubber and lasso selection"
```

---

### Task 5: Search Polish & Visual Search

**Files:**
- Modify: `src/app/components/photo-grid/photo-grid.component.html`
- Modify: `src/app/components/lightbox/lightbox.component.ts`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Add Similarity Score Badges**
Update `photo-grid.component.html` to conditionally show the score badge only when `photos.searchResults()` is not null.

- [ ] **Step 2: Implement Visual Search command**
Add a `searchByImage(image: Image)` method to `PhotoService` that invokes the Rust search command with an image ID instead of text.

- [ ] **Step 3: Connect "Find Similar" button**
In `LightboxComponent`, wire up the Lucide search icon to call `photos.searchByImage()`.

- [ ] **Step 4: Commit**
```bash
git add .
git commit -m "feat: add search similarity badges and visual search"
```
