# Design: `SidebarItem` Reusable Component

## Problem Statement
The current sidebar implementation uses ad-hoc styling for nav buttons (`folder-item` class), which has led to inconsistent padding and the need for per-item CSS overrides (e.g., `!rounded-md` on the Settings link).

## Proposed Solution
Extract a reusable `SidebarItemComponent` to centralize styling, ensure consistent layout, and eliminate per-item overrides.

## Technical Details

### 1. Structure
*   **Path**: `src/app/components/ui/sidebar-item/`
*   **Type**: Angular Component

### 2. Styling
*   Move existing `.folder-item` styles from `src/app/components/sidebar/sidebar.component.css` to `src/app/components/ui/sidebar-item/sidebar-item.component.css`.
*   Ensure `:host` correctly applies the layout (flex, items-center, gap, padding, rounded, transition).

### 3. API
*   `@Input() isActive: boolean`: Governs active state styling.
*   `@Input() routerLink: string | any[]`: Optional, for navigation items.
*   The component will support both button-based (click) and link-based (routerLink) interaction.

## Migration Plan
1.  Generate `SidebarItemComponent`.
2.  Migrate styles to the new component.
3.  Refactor `sidebar.component.html` to use `<app-sidebar-item>` wrapper for:
    *   "All Photos" entry
    *   "People" entry
    *   "Tags" entry
    *   "Per-folder" entries
    *   "Settings" entry
4.  Remove redundant `!rounded-md` and other overrides in `sidebar.component.css` and `sidebar.component.html`.
5.  Verify visual consistency.
JSON
