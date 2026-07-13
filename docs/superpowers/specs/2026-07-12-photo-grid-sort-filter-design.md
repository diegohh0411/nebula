# Photo Grid — Shared Sort & Filter Abstraction

**Date:** 2026-07-12
**Status:** Approved — ready for implementation planning
**Trigger:** Subject-detail photo grid renders images in no discernible order (they look random). See below for root cause.

---

## Problem

On the subject-detail page, a subject's photos appear in an arbitrary order. Root cause: `getSubjectPhotos()` returns `SearchResult[]` ordered by **face-match score**, and because many faces score 100%, the ties fall through to arbitrary backend row order — so the grid looks shuffled.

More broadly, **ordering and filtering of image collections is not abstracted**. Each surface reinvents (or omits) it:

- **Gallery** sorts by capture date and groups by day inside `PhotoService` (`groupByDay`, `dayGroups`, `virtualRows`).
- **Subject detail** builds justified rows straight from the unordered source — no sort at all.
- **Search / face-picker / merge** have their own incidental ordering.

There is no consistent, user-controllable Sort/Filter UX shared across routes.

### Naming clarification (important for readers)

The existing `app-photo-grid` component is a **single justified-row renderer**, *not* a grid. Collection-level concerns (ordering, grouping, layout) currently live in each consumer. This spec introduces the missing collection layer **above** the row renderer without changing `app-photo-grid` itself.

> A deeper structural cleanup toward an idiomatic Angular component/service architecture (splitting `PhotoService`, renaming the row renderer, unifying util/composable/service boundaries) is tracked separately in Notion: **"Restructure frontend toward idiomatic Angular component/service architecture"** (Refactor · Frontend · Noticed). This spec deliberately stays within the current conventions.

---

## Goals

- One consistent Sort + Filter UX for image collections across **all** surfaces, including the main gallery.
- Fix the subject-detail ordering bug as a natural consequence.
- Keep the abstraction extensible: surfaces can opt into extra sort keys / filters without reworking the core.

## Non-goals

- No backend changes. Ordering and filtering happen client-side over already-fetched images.
- No new persistence layer — choices are **session-only, per surface** (in-memory signals).
- No restructuring of `PhotoService` or renaming of components (tracked in the Notion refactor task).
- Filters beyond **date range** are out of scope for this pass (the filter registry is designed to accept more later).

---

## Architecture — three layers

The new concern sits **above** the row renderer and **below** each surface's own layout (day-grouping, virtual scroll).

| Layer | File | Kind | Responsibility |
|---|---|---|---|
| Ordering primitives | `src/app/utils/image-ordering.ts` | **new**, pure | Sort-key registry (comparators) + date-range predicate. No Angular. Unit-testable in isolation. |
| Collection model | `src/app/composables/image-collection.composable.ts` | **new**, signals | Holds `sort` + `dateRange` state; exposes `view = computed(filter → sort)`. |
| Controls UI | `src/app/components/grid-controls/` | **new**, presentational | Two icon buttons + popovers, bound to a collection instance. |
| Row renderer | `src/app/components/photo-grid/` | **unchanged** | Renders one justified row. |
| Surfaces | gallery, subject-detail, … | **modified** | Create a collection, drop in `<app-grid-controls>`, feed `collection.view()` into existing layout. |

**Why a `utils/` file for the pure logic:** follows the established `utils/justified-layout.ts` precedent (pure collection/layout math extracted from components) and keeps the composable focused on state/signals rather than comparator math. This is a convention choice, revisited by the Notion refactor task — not load-bearing.

---

## Layer 1 — `utils/image-ordering.ts` (pure)

```ts
export type SortDirection = 'asc' | 'desc';
export type SortKeyId = 'dateTaken' | 'relevance';

type Sortable = Image | SearchResult;

export interface SortKey {
  id: SortKeyId;
  label: string;                              // e.g. "Date taken", "Relevance"
  /** True when this key can meaningfully sort the given collection. */
  available(images: readonly Sortable[]): boolean;
  /** Total, deterministic comparator for direction 'desc' (natural order). */
  compare(a: Sortable, b: Sortable): number;
}

export interface DateRange {
  from: number | null;   // inclusive lower bound, same epoch unit as date_taken/mtime
  to: number | null;     // inclusive upper bound, same epoch unit as date_taken/mtime
}
```

**Built-in sort keys:**

- `dateTaken` — `available` always true. Orders by `date_taken ?? mtime` (descending = newest first for the natural order). **Deterministic tiebreak by image id.** This id tiebreak is what actually eliminates the "random" order — equal dates no longer fall through to backend order.
- `relevance` — `available` only when every item has a numeric `score` (i.e. `SearchResult` collections). Orders by `score` descending, tiebreak by image id. This is the **search-relevance carve-out**: it exists only where a score exists, so date-only surfaces never see it.

**Direction:** `applySort(images, key, direction)` returns a new array; `'asc'` reverses the comparator. Sort is stable and total (id tiebreak guarantees a single canonical order).

**Date-range filter:** `matchesDateRange(image, range)`:
- If both bounds are null → matches (no active range).
- Otherwise compares `date_taken ?? mtime` against the bounds.
- **Images with no capture date are hidden when a range is active.** (Decision: a range means "photos within this window"; an undated photo is not known to be in it. Undated photos reappear when the range is cleared.)

Helper to normalize a sort timestamp lives here too: `sortTimestamp(img) = img.date_taken ?? img.mtime`.

---

## Layer 2 — `image-collection.composable.ts` (signals)

A factory that wraps a source signal with sort + filter state and derives the view.

```ts
export interface ImageCollectionConfig {
  sortKeys: SortKey[];                          // which keys this surface offers
  defaultSort: { key: SortKeyId; direction: SortDirection };
  filters: { dateRange: boolean };              // which filters this surface offers
}

export interface ImageCollection {
  // state
  readonly sort: WritableSignal<{ key: SortKeyId; direction: SortDirection }>;
  readonly dateRange: WritableSignal<DateRange>;
  // derived
  readonly availableSortKeys: Signal<SortKey[]>; // config.sortKeys filtered by key.available(source)
  readonly view: Signal<Sortable[]>;             // filter(source) → sort
  readonly activeFilterCount: Signal<number>;    // 0 or 1 for now (date range)
  // actions
  reset(): void;                                 // back to defaults
}

export function createImageCollection(
  source: Signal<Sortable[]>,
  config: ImageCollectionConfig,
): ImageCollection;
```

- `view` = `computed(() => applySort(source().filter(matchesDateRange(range)), key, direction))`.
- If the current `sort.key` becomes unavailable (source changed, e.g. relevance key on a now-scoreless set), fall back to `defaultSort` (or the first available key).
- In-memory only; nothing persisted.
- No `TestBed` required to test — plain signals.

---

## Layer 3 — `app-grid-controls` (presentational)

Compact icon buttons at the top-right of a surface's grid area, reusing `libs/ui/popover`, `libs/ui/button`, and `lucide` icons. Bound to an `ImageCollection` instance via an input.

- **Sort button** (`arrow-up-down`): opens a popover listing `availableSortKeys()`; selecting a key sets it; a newest ↔ oldest toggle sets direction. Hidden entirely if fewer than one meaningful control (e.g. a single key with no direction relevance) — but `dateTaken` always has a direction, so it always renders.
- **Filter button** (`sliders-horizontal`): opens a popover with date-range **from**/**to** inputs and a **Clear** action. Shows a small **active-state dot** on the icon when `activeFilterCount() > 0`.
- Purely presentational: reads/writes the collection's signals, contains no ordering math.

---

## Per-surface integration

| Surface | Source | Default sort | Notes |
|---|---|---|---|
| **Subject detail** | `subjectPhotos()` (`SearchResult[]`, has score) | `dateTaken` desc | **Fixes the bug.** `relevance` also offered (score exists). Controls sit top-right in the header. Feed `collection.view()` into `buildJustifiedRows`. |
| **Gallery** | `images()` / search results | `dateTaken` desc (relevance when searching) | Feed `collection.view()` into the existing `groupByDay → virtualRows` pipeline. Day grouping, virtual scroll, timeline scrubber, and lasso all stay. Direction flips group + within-group order; date range prunes pre-grouping. Controls sit near the search bar. |
| **Search results** | `SearchResult[]` | `relevance` (preserves current ranking) | Relevance carve-out keeps existing UX; `dateTaken` additionally offered. |
| **Face-picker / merge** | as applicable | `dateTaken` desc | Adopt the same controls where a collection is shown. |

**Empty-after-filter state:** when `view()` is empty because a filter excluded everything, show a dedicated message ("No photos match these filters") with a **Clear filters** action, distinct from the existing "No photos yet" empty state.

---

## Edge cases

- **Equal / null dates:** `date_taken ?? mtime`; both null sorts to the end; id tiebreak guarantees deterministic order (the core fix).
- **Null-date + active range:** hidden while the range is active; reappears when cleared.
- **Sort key becomes unavailable:** collection falls back to default/first-available key.
- **Empty filtered result:** dedicated empty state with Clear action.
- **Search ↔ browse transitions (gallery):** when results appear/disappear, `availableSortKeys` recomputes; if relevance was active and results clear, fall back to `dateTaken`.

---

## Testing

**`image-ordering` (pure unit tests):**
- `dateTaken` asc/desc ordering.
- Null `date_taken` falls back to `mtime`; both-null sorts last.
- **Id tiebreak determinism** — equal dates produce a single canonical order (regression test for the reported bug).
- `relevance` availability (present only when all items have a score) and ordering.
- `matchesDateRange`: both-null passes all; bounds inclusive; **null-date hidden when range active**.

**`image-collection.composable` (signal tests, no TestBed):**
- `view` reflects sort + filter composition.
- `defaultSort` applied initially; `reset()` restores it.
- Unavailable current key falls back correctly.
- `activeFilterCount` tracks the date range.

**`grid-controls` (component tests):**
- Popovers open; key selection and direction toggle mutate the collection.
- Active-filter dot appears when a range is set; Clear removes it.

**Integration:**
- Subject-detail renders photos in date order (regression for the bug).
- Gallery still day-groups and honors direction + range.

---

## Out of scope / future

- Additional filters (processing status, match-confidence threshold) — filter registry accepts them later.
- Additional sort keys (filename, date added) — sort-key registry accepts them later.
- Persistence of choices across restarts.
- Broader component/service restructuring — tracked in the Notion refactor task referenced above.
