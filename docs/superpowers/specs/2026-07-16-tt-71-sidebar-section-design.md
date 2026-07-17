# Design: `SidebarSection` Component (TT-71)

## Problem Statement
The sidebar (`src/app/components/sidebar/sidebar.component.html`) hand-rolls every
section, producing visual inconsistencies:

- The first nav group (All Photos / People / Tags / Reports) has **no** section
  header or separator, while the **Folders** group gets both a `.sidebar-divider`
  and a `.sidebar-title--sub` label. One section has a separator, the other does not.
- Section titles are split between two style variants (`.sidebar-title` for the
  top "Library" header bar and `.sidebar-title--sub` for "Folders").
- **Add Folder** lives as a dashed `.btn-add-folder` button in the footer, which
  does not follow any section-scoped convention.

The item-level abstraction (`SidebarItemComponent`, from TT-72) already exists in
`src/app/components/ui/sidebar-item/`. This task adds the **section-level**
abstraction and rebuilds the sidebar on top of it.

## Proposed Solution
Introduce a reusable, presentational `SidebarSection` component that owns section
title + separator + spacing uniformly, and exposes a right-aligned action slot for
section-level actions. Rebuild `sidebar.component.html` so **every** group renders
through it.

## Technical Details

### 1. Structure
- **Path**: `src/app/components/ui/sidebar-section/` (mirrors `sidebar-item/`).
- **Type**: standalone Angular component, `ChangeDetectionStrategy.OnPush`.
- **Selector**: `app-sidebar-section`.

### 2. API
- `@Input() title?: string` — optional section header. When present, renders the
  section title (the current `.sidebar-title--sub` treatment). When absent, no
  header row is rendered.
- `@Input() divider = false` — when `true`, renders a top separator above the
  section. Used to separate consecutive sections; the first section leaves it
  `false`. Centralizing the divider here replaces the ad-hoc `.sidebar-divider`.
- **Action slot**: `<ng-content select="[sidebarSectionAction]">` — projected
  into the header row, aligned to the right of the title (e.g. the Add Folder
  plus button). Only rendered when a title row exists.
- **Default slot**: `<ng-content>` — the section's items (`app-sidebar-item`
  entries, empty-state text, etc.).

### 3. Template (shape)
```
@if (divider) { <div class="sidebar-section-divider"></div> }
@if (title) {
  <div class="sidebar-section-header">
    <span class="sidebar-section-title">{{ title }}</span>
    <ng-content select="[sidebarSectionAction]"></ng-content>
  </div>
}
<ng-content></ng-content>
```

### 4. Styling
- New `sidebar-section.component.css` owns: header row layout (title left, action
  right via `justify-between` / `ml-auto`), the section-title typography (moved
  from `.sidebar-title--sub`), the divider, and consistent top spacing.
- The action button styling (icon-only plus) reuses existing muted-foreground
  hover conventions; a small `.sidebar-section-action` class in the sidebar (or a
  shared button style) keeps it consistent with other affordances.

## Sidebar Rebuild
Two sections, both via `SidebarSection`:

1. **Library** — `title="Library"`, no divider (first section). Contains the
   All Photos, People, Tags, Reports `app-sidebar-item` entries. The current
   fixed top "Library" header bar (`.sidebar-header`) is **removed**; its label
   moves into this section title.
2. **Folders** — `title="Folders"`, `divider`. Its action slot holds an
   icon-only **plus** button wired to `addFolder()`. Contains the per-folder
   `app-sidebar-item` entries and the "No folders yet" empty state.

**Footer** keeps the Settings `app-sidebar-item` only. The dashed
`.btn-add-folder` button is **removed** (relocated into the Folders action slot).

No changes to `sidebar.component.ts` logic are required beyond template wiring:
`addFolder()`, `removeFolder()`, `selectFolder()`, active-state helpers, and the
remove-folder dialog all stay as-is.

## Acceptance Criteria (from TT-71)
- [ ] All sidebar sections render via `SidebarSection`; headers/separators are
  visually consistent across groups.
- [ ] `SidebarSection` exposes an action-button slot for section-level actions.
- [ ] **Add Folder** is a section action button on the Folders header; the dashed
  footer button is removed.
- [ ] No regression to active state, folder counts, or the remove-folder affordance.

## Migration Plan
1. Generate `SidebarSectionComponent` (+ css + spec) under `ui/sidebar-section/`.
2. Move section-title / divider styles out of `sidebar.component.css` into the
   new component; delete `.sidebar-header`, `.sidebar-title`, `.sidebar-divider`,
   `.sidebar-title--sub`, and `.btn-add-folder` from `sidebar.component.css`.
3. Rebuild `sidebar.component.html` around two `app-sidebar-section` groups + the
   Settings footer, with the Add Folder plus button in the Folders action slot.
4. Import `SidebarSectionComponent` in `sidebar.component.ts`.
5. Verify visual consistency and no regressions (active state, counts, remove,
   add folder) via the dev server and `npm run build`.

## Testing
- Unit spec for `SidebarSectionComponent`: renders title when provided, omits the
  header when not; renders the action slot content; applies the divider when
  `divider` is true.
- Manual/dev-server verification of the assembled sidebar against the acceptance
  criteria.
