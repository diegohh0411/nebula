# Design Spec: Sidebar-Aware Responsive Lightbox

Implement a responsive layout for the Nebula Photo Manager lightbox that prevents original full-resolution images from overflowing the viewport and ensures the image area resizes dynamically when the metadata sidebar is toggled.

## 1. Problem Statement
- **Image Overflow**: Loading original full-resolution images causes them to exceed viewport boundaries because their intrinsic size is larger than the available flexbox container.
- **Sidebar Overlap**: The metadata sidebar currently overlays the image using `fixed` positioning, which can obscure parts of the photo.
- **Lack of Reflow**: Toggling the sidebar does not trigger a resize of the main image area.

## 2. Proposed Architecture

### 2.1 Component Structure (`lightbox.component.html`)
The lightbox will be refactored into a top-level horizontal flex container:

```html
<div class="lightbox-overlay" (click)="close()">
  <!-- Main View Area (Image + Toolbar) -->
  <div class="main-view">
    <div class="lightbox-toolbar" (click)="$event.stopPropagation()">
      <!-- ... filename and actions ... -->
    </div>

    <div class="lightbox-content" (click)="$event.stopPropagation()">
      <!-- ... navigation buttons ... -->
      <div class="image-container">
        <img class="main-image" ... />
      </div>
      <!-- ... navigation buttons ... -->
    </div>
  </div>

  <!-- Sidebar (Conditional) -->
  @if (showSidebar()) {
    <div class="info-sidebar" (click)="$event.stopPropagation()">
      <!-- ... sidebar content ... -->
    </div>
  }
</div>
```

### 2.2 Layout & Styling Constraints (`lightbox.component.css`)
- **`lightbox-overlay`**: Set to `display: flex; flex-direction: row;`.
- **`main-view`**: Set to `flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden;`.
  - The `min-width: 0` is critical to allow the flex item to shrink below its content's minimum size.
- **`image-container`**: Set to `flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; padding: 2rem;`.
  - The `min-height: 0` ensures the container respects the available vertical space.
- **`main-image`**: Continue using `max-width: 100%; max-height: 100%; object-contain: contain;`.
  - Added: `transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);` for smooth resizing when the sidebar toggles.
- **`info-sidebar`**: Change from `fixed` to `relative` (or just remove the `fixed` positioning since it's now a flex sibling). Set a fixed width (e.g., `w-80` or `20rem`).

## 3. Data Flow & Behavior
- **Initialization**: Lightbox opens, `main-view` occupies 100% width.
- **Toggle Sidebar**: 
  - `showSidebar()` signal becomes `true`.
  - Angular inserts `.info-sidebar` into the flexbox.
  - Flexbox recalculates widths; `.main-view` shrinks to accommodate the sidebar.
  - The `main-image` automatically scales down due to `max-width: 100%` and `object-contain`.
- **Navigation**: Transitioning between images works as before, with the current view dimensions maintained.

## 4. Verification Plan
- **Overflow Check**: Open a 4K+ resolution image; verify it is fully contained within the viewport with visible margins.
- **Sidebar Resize**: Toggle the "Details" sidebar; verify the image smoothly shrinks and remains fully visible in the remaining space.
- **Responsiveness**: Resize the browser window while the lightbox is open; verify the image adjusts its scale correctly.
- **Consistency**: Ensure view transitions still work smoothly with the new nested flex structure.
