# UI/UX Polish Pass — Design Spec

- **Date:** 2026-06-12
- **Status:** Approved (ready for implementation plan)
- **Scope:** Front-end only (`src/app/`), one epic
- **Notion epic:** https://app.notion.com/p/37de954db476813dbeb8f10bd1074daa
- **Implementer assumption:** A reasonably capable AI agent (not the weakest tier). Specs give file paths, root causes, and acceptance criteria; the agent is trusted to make sound local implementation choices within them.

## Context & Goal

A cluster of UI/UX papercuts surfaced while browsing a library of ~2,000 processed photos. This is a focused front-end refactor of the Angular app. The app already ships **Spartan-NG** (`@spartan-ng/helm`, `@spartan-ng/brain`, `components.json` → `importAlias: "@spartan-ng/helm"`, `componentsPath: src/app/libs/ui`) as its shared UI library; prefer it over hand-rolled inputs.

Deliver as **one branch / one PR** (small stack acceptable), commits sectioned per work item, `pnpm test` green before review. Do **not** merge to `main`.

## Non-Goals

- No backend/Rust changes unless a section explicitly requires data that isn't already exposed (none are expected — the needed numbers come through `PhotoService`).
- No unrelated refactoring beyond what each item needs.

## Conventions

Match the surrounding code: standalone components, `ChangeDetectionStrategy.OnPush`, Angular signals, Tailwind utilities / `@apply` in component CSS, `@if`/`@for` control flow. Preserve existing `data-testid` attributes; update `.spec.ts` only when behavior legitimately changes.

---

## 0. Fix pre-existing test failures (do first)

`pnpm test` currently reports **9 failing tests, all in `src/app/components/people-view/people-view.component.spec.ts`**, from a **single root cause**:

```
ReferenceError: IntersectionObserver is not defined
  at PeopleViewComponent.ngAfterViewInit (src/app/components/people-view/people-view.component.ts:48)
```

jsdom (the vitest `environment`) provides no `IntersectionObserver`, so every test that mounts `PeopleViewComponent` throws in `ngAfterViewInit` before its own assertions run. The component logic is fine; the **test environment** lacks the global.

**Fix:** Add a minimal global `IntersectionObserver` stub (and, defensively, `ResizeObserver` — `gallery`, `photo-grid`, and `lightbox` also use observers) in **`src/test-setup.ts`** so jsdom-mounted components don't crash. A no-op class with `observe`/`unobserve`/`disconnect`/`takeRecords` is sufficient.

**Acceptance:** `pnpm test` passes with **0 failures** before any feature work starts, and stays green through the rest of the epic.

---

## 1. Global layout & scrolling (foundation)

Each route currently solves scrolling on its own; several get it wrong. Centralize so the **sidebar is always fixed** and the **routed content column is the single vertical scroll owner**.

- **1a — Shell:** `src/app/app.component.ts` template (`flex h-screen overflow-hidden` → `<app-sidebar>` + `flex flex-col flex-1 min-w-0 h-full` column wrapping `<router-outlet />`). Each routed component fills the column (`h-full`, `min-h-0`, flex column) and scrolls internally; nothing relies on document/body scroll.
  - **Acceptance:** No page-level (document) scrollbar at any window size; sidebar never scrolls away; each route scrolls within the content column.
- **1b — Settings:** `settings.component.html` / `.css`. Root cause: `.settings-container` (`p-8 max-w-[640px] mx-auto`) has no height/overflow → content clipped; `.settings-content` has no `gap` → "Smart Search" and "People Recognition" sit flush. Add a scrollable region and consistent vertical rhythm (e.g. `.settings-content` → `flex flex-col gap-10` + bottom padding).
  - **Acceptance:** With both model sections expanded the user can scroll to the bottom of "People Recognition"; section spacing is even and matches header-to-first-section spacing.
- **1c — Timeline scrubber (the "double scrollbar"):** `timeline-scrubber/*`, used in `gallery.component.html`. Root cause: a `fixed right-0 top-16 bottom-0 w-8 bg-background/20 backdrop-blur-sm` overlay sits on top of the CDK virtual-scroll bar → two full-height bars. **Decision (A):** keep its date-jump value but reveal it only on hover/scroll-activity, narrow it, and drop the always-on full-height blurred background so it stops reading as a duplicate scrollbar and never overlaps the native one.
  - **Acceptance:** At rest the gallery shows a single usable scrollbar; no full-height blurred bar layered on top.

---

## 2. Search results — unify scroll & restyle the People section

- **2a — People row scrolls with the grid:** `gallery.component.html`. Root cause: the `.subjects-row` ("People") block renders **outside** `<cdk-virtual-scroll-viewport>`, so it stays pinned. Move it to participate in the same scroll container (e.g. first virtual/header row inside the viewport). The result count may stay pinned; the People cards must scroll.
  - **Acceptance:** Scrolling results moves the People cards up and out of view with the photos; no separately-pinned People strip.
- **2b — People cards use the real photo + the new card design:** see **Shared component** below.

---

## 3. Subject card avatars not loading (Tags view)

`tags-view.component.html` (`.subject-cards-grid`) hardcodes `<span class="text-2xl">👤</span>` and never loads the face crop — same defect as the search card. Use the new shared card (below), which loads crops and falls back to the placeholder only when there's no crop.

- **Acceptance:** Selecting a tag shows its subjects with real face photos in the new card style; no perpetual `👤` when a crop exists.

---

## Shared component: `SubjectPersonCard`

Factor the person card into **one reusable component** consumed by both the gallery search "People" row (§2b) and the Tags view (§3), so styling stays in one place. Input: a `SubjectMatch` (and an optional remove/secondary action for the Tags view). It loads the face crop and renders the selected design.

**Crop loading** — reuse the proven pattern from `people-view`: `PhotoService.getFaceCrop(subject.thumbnail_face_id)` → `PhotoService.thumbnailUrl(path)`, cached in a `Map<number, string>` signal. Render `👤` fallback only when there is no `thumbnail_face_id` or the crop fails.

**Design — "A · Gradient scrim"** (selected via visual mockup):

- **Shape:** ~3:4 portrait tile, `rounded-2xl` (16px), `overflow-hidden`, subtle border (`border-white/10` in dark) and soft shadow. Larger and more spacious than the current tiny avatar+label.
- **Image:** full-bleed face crop, `object-cover`, fills the tile.
- **Scrim:** bottom gradient overlay, `linear-gradient(to top, rgba(8,8,11,0.92) 0%, rgba(8,8,11,0.45) 28%, transparent 55%)` — name stays legible without darkening the whole photo. (Map `rgba(8,8,11,…)` to the theme `background` token where practical so it adapts to light mode.)
- **Name:** bottom-left, ~16px / weight 600, white, slight negative tracking, truncates on overflow. Show "Unnamed" when `subject.name` is null.
- **Tags:** small frosted pills above the name (`bg-white/16` + `backdrop-blur`, ~10px/600). Cap visible tags (e.g. 2) with a `+N` overflow.
- **Motion:** gentle hover lift (`translateY(-4px)`) + image scale (~`scale-105`) with a short ease; respects `prefers-reduced-motion`.
- **Interaction & a11y:** the whole card is a button/link (navigates to the subject, preserving existing `navigateToSubject` / `routerLink` behavior); visible focus ring; keyboard-activatable. Tags-view variant keeps its "Remove" affordance (e.g. on hover, not overlapping the nav target).

**Acceptance:** A searched person and a tagged subject both render as a larger card with their actual face photo filling it and their name overlaid on the scrim; the two surfaces look identical because they share the component; clicking navigates to the subject; placeholder shows only when no crop exists.

---

## 4. People naming bugs

- **4a — Single-click focus:** `editable-text.component.ts`, consumed by `people-view` (`[startEditing]`), `subject-detail`, `tags-view`. Root cause: focus is applied inside an `afterNextRender({ read })` callback **registered once in the constructor**, which fires only after the first render — later edit-mode entries never re-focus, so the browser only focuses on a second (native) click. Fix: focus the input on **every** edit-mode entry (both the `startEditing` setter and `startEdit()` click path) using a per-transition mechanism (e.g. an `effect()` that focuses when `isEditing()` flips true and the ref exists, or a per-edit `afterNextRender`). Caret select-all or caret-at-end is fine.
  - **Acceptance:** One click on "+ Add a name" (and anywhere `editable-text` is used) places the cursor ready to type; `editable-text.component.spec.ts` still passes (extend it to assert single-click focus).
- **4b — Name reverts then catches up:** `subject-detail.component.ts` → `saveName()` does `this.detail.update(d => { d.subject.name = name; return d; })` — mutates and returns the **same object reference**, so the signal (referential equality) doesn't notify; the template updates only on unrelated change detection. Fix: update immutably, e.g. `this.detail.update(d => d ? { ...d, subject: { ...d.subject, name } } : d)` (and any sibling nested-mutation-into-signal here).
  - **Acceptance:** Editing a name and pressing Enter / blurring immediately updates the detail header; no navigation round-trip, no flash back to the old value.

---

## 5. Shared, consistent text inputs (Spartan adoption)

Text inputs are inconsistent and unpolished — the "Add tag" field in `subject-detail`, the bespoke `tags-view` "New tag name…" input, `settings` `.confirm-input`, and `editable-text`. Spartan is installed but **no shared input/form-field helm component exists yet** (only button, card, command, icon, popover). Generate a shared **helm input** under `src/app/libs/ui` via the Spartan CLI and adopt it for at least the subject-detail "Add tag" and tags-view "new tag" fields (keep REINDEX `.confirm-input` consistent). Normalize spacing so inputs and adjacent buttons align.

- **Acceptance:** The shared input is used in subject-detail "Add tag" and tags-view "new tag"; inputs share height, radius, focus ring, and spacing across those views; the "Add tag" area no longer looks cramped.

---

## 6. Collapsible, relative-date photo groups (gallery)

`gallery.component.html` (`.day-header` rows) + the row source `PhotoService.virtualRows()` (`src/app/services/photo.service.ts`). Day-group headers already exist (`row.type === 'header'`, `row.label`).

- **Relative labels:** Today, Yesterday, This Week, Last Week, then month-year for older groups (Today/Yesterday correct relative to current date); keep them as sticky header rows.
- **Collapsible:** clicking a header toggles collapse for that group; track collapsed group keys in a signal and omit that group's image rows from the virtual rows while keeping the header (with a chevron/count affordance). Persisting collapse across navigation is nice-to-have.
- **Acceptance:** Photos group under relative-date headers; clicking a header collapses/expands its photos; collapsing several quickly hides date ranges; virtual scrolling still performs well.

---

## 7. Sidebar & header polish

- **7a — Remove app title from header:** `search-bar.component.html` `<span class="app-name">` (star SVG + "Nebula") and its `.app-name`/`.app-icon` CSS. Header becomes just the search input + existing processing badge; fix leading padding.
  - **Acceptance:** Gallery top bar shows only the search field (+ badge); no "✦ Nebula" label.
- **7b — Honest sidebar grouping:** `sidebar.component.html` header `<span class="sidebar-title">Folders</span>` precedes non-folder nav (All Photos, People, Tags) then the divider and real folders. **Decision (A):** drop the single "Folders" title; present a top "Library" group (All Photos / People / Tags) and a **"Folders"** section heading placed **above the actual folder list** (after the divider).
  - **Acceptance:** "Folders" labels only the real folder entries; All Photos / People / Tags are not presented as folders.

---

## 8. Processing progress: inference speed + ETA

Mostly front-end. `search-bar.component.html` `.embed-badge` renders `Processing {total_pending} images · {images_per_sec} img/s` **only when `images_per_sec >= 0.1`**, so right after adding a folder it shows nothing and there's no ETA.

- Always indicate active processing when there's a backlog ("Processing N images…" immediately), then append "· X img/s" once a rate is known, using the `effective_rate` helper (commit TT-64) that holds last-known throughput so the rate doesn't blink out.
- Add **estimated time remaining** = `total_pending / effective_rate`, formatted human-friendly (e.g. "~3 min left"); hide ETA only when no rate is known yet.
- **Acceptance:** Right after adding a folder the badge shows processing started; once a rate is known it shows img/s and a counting-down ETA.

---

## Testing strategy

- §0 must land first and keep `pnpm test` green throughout.
- Extend `editable-text.component.spec.ts` for single-click focus (§4a); add/adjust `subject-detail` spec for the immutable name update (§4b) if feasible.
- Add a focused spec for `SubjectPersonCard` (crop-loaded image vs placeholder fallback, navigation).
- Keep existing `data-testid` hooks intact for `people-view` and others.

## Sequencing (high level — detailed steps live in the implementation plan)

0. Fix test env (§0) → 1. Global scrolling foundation (§1) → 2. Sidebar/header polish (§7) → 3. Naming bugs (§4) → 4. `SubjectPersonCard` + adopt in search & tags (§2/§3) → 5. Shared input (§5) → 6. Collapsible date groups (§6) → 7. Processing ETA (§8).
