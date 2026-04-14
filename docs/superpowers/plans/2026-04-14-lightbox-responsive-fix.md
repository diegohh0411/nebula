# Lightbox Layout and Resizing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix lightbox image overflow by loading original full-res images within a responsive flexbox container that shrinks dynamically when the metadata sidebar is toggled.

**Architecture:** Refactor the lightbox into a horizontal flexbox where the main content area and sidebar are siblings. Use `min-width: 0` and `min-height: 0` to force flex items to respect container boundaries rather than expanding to the image's intrinsic size.

**Tech Stack:** Angular 20, Tailwind CSS, Vanilla CSS (for transitions).

---

### Task 1: Refactor Lightbox HTML Structure

**Files:**
- Modify: `src/app/components/lightbox/lightbox.component.html`

- [ ] **Step 1: Update the template to use a nested `main-view` wrapper**

```html
<div class="lightbox-overlay" (click)="close()">
  <!-- Main View Area (Image + Toolbar) -->
  <div class="main-view">
    <div class="lightbox-toolbar" (click)="$event.stopPropagation()">
      <div class="filename" [title]="image ? filename(image) : ''">
        {{ image ? filename(image) : 'Loading...' }}
      </div>
      
      <div class="actions">
        <button class="icon-btn" (click)="findSimilar()" title="Find similar images">
          <lucide-icon name="search" size="20"></lucide-icon>
        </button>
        <button class="icon-btn" (click)="toggleSidebar()" title="Toggle details">
          <lucide-icon name="info" size="20"></lucide-icon>
        </button>
        <button class="icon-btn" (click)="close()" title="Close">
          <lucide-icon name="x" size="20"></lucide-icon>
        </button>
      </div>
    </div>

    <!-- Main Content -->
    <div class="lightbox-content" (click)="$event.stopPropagation()">
      <button class="nav-btn prev" (click)="prev()" title="Previous">
        <lucide-icon name="chevron-left" size="32"></lucide-icon>
      </button>

      <div class="image-container">
        @if (image) {
          <img
            [src]="originalUrl(image)"
            [alt]="filename(image)"
            class="main-image"
            [style.view-transition-name]="'photo-' + ('id' in image ? image.id : image.image_id)"
            [style.background-image]="'url(' + thumbUrl(image) + ')'"
            style="background-size: contain; background-repeat: no-repeat; background-position: center;"
          />
        }
      </div>

      <button class="nav-btn next" (click)="next()" title="Next">
        <lucide-icon name="chevron-right" size="32"></lucide-icon>
      </button>
    </div>
  </div>

  <!-- Info Sidebar -->
  @if (showSidebar()) {
    <div class="info-sidebar" (click)="$event.stopPropagation()">
      <div class="sidebar-header">
        <h3>Details</h3>
        <button class="icon-btn" (click)="toggleSidebar()">
          <lucide-icon name="x" size="20"></lucide-icon>
        </button>
      </div>

      <div class="sidebar-content" *ngIf="image">
        <div class="meta-item">
          <label>File Path</label>
          <div class="value break-all">{{ image.path }}</div>
        </div>
        <div class="meta-item">
          <label>Dimensions</label>
          <div class="value">Original Resolution</div>
        </div>
        <!-- Add more metadata here as needed -->
      </div>
    </div>
  }
</div>
```

- [ ] **Step 2: Commit changes**

```bash
git add src/app/components/lightbox/lightbox.component.html
git commit -m "feat(lightbox): restructure HTML for responsive flexbox layout"
```

---

### Task 2: Implement Responsive CSS Flexbox Logic

**Files:**
- Modify: `src/app/components/lightbox/lightbox.component.css`

- [ ] **Step 1: Update CSS for flexbox containment and resizing**

```css
.lightbox-overlay {
  @apply fixed inset-0 z-50 bg-black/95 flex flex-row overflow-hidden;
  backdrop-filter: blur(8px);
}

.main-view {
  @apply flex-1 min-w-0 flex flex-col h-full relative;
}

.lightbox-toolbar {
  @apply flex items-center justify-between px-6 py-4 bg-gradient-to-b from-black/60 to-transparent flex-shrink-0;
}

.filename {
  @apply text-sm font-medium text-white/90 truncate max-w-[50%];
}

.actions {
  @apply flex items-center gap-2;
}

.icon-btn {
  @apply p-2 rounded-full text-white/70 hover:text-white hover:bg-white/10 transition-colors border-0 bg-transparent cursor-pointer;
}

.lightbox-content {
  @apply flex-1 flex items-center justify-between px-4 relative min-h-0;
}

.image-container {
  @apply flex-1 h-full flex items-center justify-center p-8 min-w-0 min-h-0;
}

.main-image {
  @apply max-w-full max-h-full object-contain shadow-2xl rounded-sm;
  transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.nav-btn {
  @apply p-4 rounded-full text-white/40 hover:text-white hover:bg-white/5 transition-all
         disabled:opacity-20 disabled:pointer-events-none border-0 bg-transparent cursor-pointer;
}

/* Info Sidebar */
.info-sidebar {
  @apply relative top-0 right-0 bottom-0 w-80 bg-background border-l border-border
         shadow-2xl flex flex-col z-50 flex-shrink-0;
  animation: slide-in 0.3s ease-out;
}

@keyframes slide-in {
  from { transform: translateX(100%); }
  to { transform: translateX(0); }
}

.sidebar-header {
  @apply flex items-center justify-between p-4 border-b border-border flex-shrink-0;
}

.sidebar-header h3 {
  @apply text-sm font-semibold uppercase tracking-wider text-muted-foreground;
}

.sidebar-content {
  @apply p-6 flex flex-col gap-6 overflow-y-auto;
}

.meta-item label {
  @apply text-xs font-medium text-muted-foreground uppercase tracking-tight mb-1 block;
}

.meta-item .value {
  @apply text-sm text-foreground leading-relaxed;
}

.break-all {
  word-break: break-all;
}
```

- [ ] **Step 2: Run build to verify CSS syntax**

Run: `pnpm run build`
Expected: SUCCESS

- [ ] **Step 3: Commit changes**

```bash
git add src/app/components/lightbox/lightbox.component.css
git commit -m "feat(lightbox): implement sidebar-aware resizing and overflow prevention"
```

---

### Task 3: Verification and Polish

- [ ] **Step 1: Verify layout constraints manually (mental check)**
  - `main-view` has `min-w-0` -> allows shrinking when sidebar is present.
  - `image-container` has `min-h-0` -> allows respecting vertical constraints.
  - `main-image` has `max-width: 100%` and `max-height: 100%` -> remains contained.

- [ ] **Step 2: Push changes**

```bash
git push origin feat/ui-ux-overhaul
```
