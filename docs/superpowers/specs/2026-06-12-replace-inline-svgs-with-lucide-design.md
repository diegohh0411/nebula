# Design Spec: Replace Inline SVG Icons with Lucide Icons (TT-31)

## Overview
Replace all remaining inline SVG icons in the Nebula frontend application with `lucide-angular` components to ensure consistency, improve maintainability, and reduce code duplication.

## Goals
- Identify all instances of inline `<svg>` in the codebase.
- Map them to the appropriate Lucide icons.
- Update components to use `<lucide-icon>`.
- Verify the UI remains visually consistent.
- Ensure no functional regressions (using existing tests).

## Components to Update
- `src/app/components/sidebar/sidebar.component.html` (remaining)
- `src/app/components/search-bar/search-bar.component.html`
- `src/app/components/gallery/gallery.component.html`
- `src/app/components/photo-grid/photo-grid.component.html`

## Technical Approach
1. For each component:
   - Identify the inline `<svg>` and its purpose.
   - Find the corresponding icon in [Lucide](https://lucide.dev/).
   - Import `LucideAngularModule` in the component (if not already imported).
   - Replace `<svg>` with `<lucide-icon name="..." size="...">`.
   - Map SVG attributes (`stroke`, `fill`, etc.) to Lucide component properties or CSS if required.
2. Run tests after each component update.
3. Commit progress.
4. After all replacements, perform a final visual smoke test and run full test suite.
5. Create a PR.
