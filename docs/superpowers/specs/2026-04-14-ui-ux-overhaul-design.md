# Nebula — UI/UX Overhaul Design Spec
**Date:** 2026-04-14  
**Status:** Approved  
**Supersedes:** Partial sections of `2026-04-13-photo-manager-design.md` related to Gallery UI.

---

## Overview
This spec defines the visual and interaction overhaul of Nebula to achieve a "Google Photos" level of polish. The focus is on seamless browsing, immersive photo viewing, and high-performance navigation for libraries exceeding 50,000 images.

---

## User Experience (UX) Patterns

### 1. Justified Gallery Layout
- **Behavior:** Photos maintain their native aspect ratio. Rows have a fixed target height (e.g., 220px), and images within the row scale their widths to fit.
- **Responsiveness:** The last image in a row may expand to fill remaining space or stay left-aligned based on a threshold to prevent extreme stretching.
- **Performance:** Continued use of Angular CDK Virtual Scroll, refactored to handle variable-width justified rows.

### 2. Immersive Lightbox
- **Transition:** Uses the **View Transitions API** for a "Shared Element" effect. When a thumbnail is clicked, it visually expands from its grid position to full-screen.
- **Controls:**
  - **Toolbar:** Discreet top-right Lucide icons for:
    - `Search` (Visual Search/Find Similar)
    - `Info` (Toggle Metadata Sidebar)
    - `X` (Close/Dismiss)
  - **Navigation:** Horizontal swiping (touch/mouse drag) and Keyboard (Left/Right arrows).
  - **Dismissal:** Vertical "flick" or swipe-down gesture to return to the grid.

### 3. Timeline Scrubber
- **Visual:** A thin vertical track on the far right of the gallery.
- **Interaction:** Dragging the handle jumps the virtual scroll to the corresponding date.
- **Feedback:** A floating "Date Bubble" (e.g., "July 2025") appears next to the handle during scrubbing.

### 4. Search & Selection
- **Lasso Selection:** Click-and-drag across the grid to select multiple photos.
- **Similarity Badges:** In search results *only*, thumbnails display a small, semi-transparent percentage badge (e.g., "98%") in the top-left corner using Lucide icons for context.
- **Visual Search:** Clicking the `Search` icon in the lightbox triggers a search using the current image's embedding as the query.

### 5. Metadata Sidebar
- **Behavior:** A right-hand sliding panel that reveals EXIF data, file path, and embedding status.
- **Persistence:** Can stay open while the user navigates between photos in the lightbox for "inspect mode."

---

## Technical Strategy

### 1. Animation Engine: View Transitions API
- **Primary:** All major transitions (Grid → Lightbox, Sidebar Open/Close, View Switching) will use `document.startViewTransition()`.
- **Implementation:** Assign unique `view-transition-name` properties to active elements (like the clicked image) to enable shared-element motion.
- **Fallback:** Standard CSS transitions/transforms for low-level interactions (hover states). Angular Animations used only if complex state-orchestration exceeds View Transition capabilities.

### 2. Iconography
- **Library:** `lucide-angular`.
- **Usage:** All buttons, status indicators, and sidebar labels will use Lucide icons for a consistent, lightweight aesthetic.

### 3. Layout Engineering
- **Justified Grid:** Custom implementation or a lightweight utility to calculate row distributions based on image aspect ratios stored in the database.
- **Virtual Scrolling:** `CdkVirtualScrollViewport` with `fixedSize` rows (since justified rows in a single day group will share a height).

---

## Implementation Phases

1.  **Infrastructure:** Add `lucide-angular`, set up View Transition utility.
2.  **The Grid:** Implement the justified layout logic and refactor virtual scrolling.
3.  **The Lightbox:** Create the immersive viewer with shared-element transitions.
4.  **Navigation & Selection:** Add the timeline scrubber and lasso selection.
5.  **Final Polish:** Integrate the metadata sidebar and similarity badges for search.
