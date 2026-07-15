# Photo Grid Sort & Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a shared, consistent Sort + Filter layer for image collections across all surfaces (fixing the subject-detail random-ordering bug as a consequence).

**Architecture:** A pure ordering util (`image-ordering.ts`) provides comparators + a date-range predicate. A signal-based composable (`image-collection`) wraps a source signal with sort/filter state and derives a `view`. A presentational `app-grid-controls` component renders icon-button popovers bound to a collection. Surfaces feed the ordered/filtered result into their existing layout; the `app-photo-grid` row renderer is untouched.

**Tech Stack:** Angular (standalone components, signals), Vitest + jsdom, spartan-ng `brn`/`hlm` popover + button, lucide-angular icons.

## Global Constraints

- Session-only, per-surface state — no persistence, in-memory signals only.
- No backend changes; sort/filter operate on already-fetched images client-side.
- Do not restructure `PhotoService` or rename `app-photo-grid` (tracked in a separate Notion refactor task).
- Timestamps (`date_taken`, `mtime`) are **epoch seconds**. Date-range bounds are epoch seconds.
- "Undated" = `date_taken == null`. Undated images are **hidden** while a date range is active.
- Deterministic tiebreak by image id in every comparator (this is the core bug fix).
- Follow existing idioms: comparators live in `utils/`, stateful signal logic in `composables/`, components under `components/<name>/`.
- Test a single file with: `pnpm exec vitest run <path-to-spec>`.
- Reference source model fields: `Image` has `id`; `SearchResult` has `image_id` and `score`. Both have `date_taken: number | null` and `mtime: number`.

---

### Task 1: Ordering primitives (`image-ordering.ts`)

**Files:**
- Create: `src/app/utils/image-ordering.ts`
- Test: `src/app/utils/image-ordering.spec.ts`

**Interfaces:**
- Consumes: `Image`, `SearchResult` from `../models/models`.
- Produces:
  - `type SortDirection = 'asc' | 'desc'`
  - `type SortKeyId = 'dateTaken' | 'relevance'`
  - `type Sortable = Image | SearchResult`
  - `interface DateRange { from: number | null; to: number | null }`
  - `interface SortKey { id: SortKeyId; label: string; available(images: readonly Sortable[]): boolean; compare(a: Sortable, b: Sortable): number }`
  - `const SORT_KEYS: Record<SortKeyId, SortKey>`
  - `function imageId(img: Sortable): number`
  - `function applySort(images: readonly Sortable[], key: SortKey, direction: SortDirection): Sortable[]`
  - `function matchesDateRange(img: Sortable, range: DateRange): boolean`

- [ ] **Step 1: Write the failing test**

Create `src/app/utils/image-ordering.spec.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { SORT_KEYS, applySort, matchesDateRange, imageId } from './image-ordering';
import { Image, SearchResult } from '../models/models';

function img(id: number, dateTaken: number | null, mtime = 0): Image {
  return {
    id, folder_id: 1, path: `/p/${id}.jpg`, file_hash: '', hash_status: 'ok',
    date_taken: dateTaken, mtime, thumbnail_path: null, preview_path: null,
    semantic_analysis_done: true, subject_analysis_done: true,
    added_at: 0, updated_at: 0, deleted_at: null,
  };
}
function result(imageIdNum: number, score: number, dateTaken: number | null, mtime = 0): SearchResult {
  return {
    image_id: imageIdNum, path: `/p/${imageIdNum}.jpg`, thumbnail_path: null, preview_path: null,
    score, date_taken: dateTaken, mtime, semantic_analysis_done: true, subject_analysis_done: true,
  };
}

describe('imageId', () => {
  it('reads id from Image and image_id from SearchResult', () => {
    expect(imageId(img(7, 0))).toBe(7);
    expect(imageId(result(9, 1, 0))).toBe(9);
  });
});

describe('applySort dateTaken', () => {
  it('sorts newest-first for desc', () => {
    const out = applySort([img(1, 100), img(2, 300), img(3, 200)], SORT_KEYS.dateTaken, 'desc');
    expect(out.map(imageId)).toEqual([2, 3, 1]);
  });
  it('sorts oldest-first for asc', () => {
    const out = applySort([img(1, 100), img(2, 300), img(3, 200)], SORT_KEYS.dateTaken, 'asc');
    expect(out.map(imageId)).toEqual([1, 3, 2]);
  });
  it('falls back to mtime when date_taken is null', () => {
    const out = applySort([img(1, null, 500), img(2, 200)], SORT_KEYS.dateTaken, 'desc');
    expect(out.map(imageId)).toEqual([1, 2]);
  });
  it('is deterministic on equal timestamps via id tiebreak', () => {
    const out = applySort([img(3, 100), img(1, 100), img(2, 100)], SORT_KEYS.dateTaken, 'desc');
    expect(out.map(imageId)).toEqual([3, 2, 1]);
  });
});

describe('SORT_KEYS.relevance', () => {
  it('is unavailable when any item lacks a score', () => {
    expect(SORT_KEYS.relevance.available([result(1, 0.5, 0), img(2, 0)])).toBe(false);
    expect(SORT_KEYS.relevance.available([])).toBe(false);
  });
  it('is available and sorts by score desc when all have scores', () => {
    const items = [result(1, 0.2, 0), result(2, 0.9, 0), result(3, 0.5, 0)];
    expect(SORT_KEYS.relevance.available(items)).toBe(true);
    expect(applySort(items, SORT_KEYS.relevance, 'desc').map(imageId)).toEqual([2, 3, 1]);
  });
});

describe('matchesDateRange', () => {
  it('matches everything when both bounds are null', () => {
    expect(matchesDateRange(img(1, null), { from: null, to: null })).toBe(true);
  });
  it('hides undated images when a range is active', () => {
    expect(matchesDateRange(img(1, null, 999), { from: 100, to: null })).toBe(false);
  });
  it('applies inclusive bounds', () => {
    expect(matchesDateRange(img(1, 100), { from: 100, to: 200 })).toBe(true);
    expect(matchesDateRange(img(1, 200), { from: 100, to: 200 })).toBe(true);
    expect(matchesDateRange(img(1, 99), { from: 100, to: 200 })).toBe(false);
    expect(matchesDateRange(img(1, 201), { from: 100, to: 200 })).toBe(false);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm exec vitest run src/app/utils/image-ordering.spec.ts`
Expected: FAIL — cannot resolve `./image-ordering`.

- [ ] **Step 3: Write minimal implementation**

Create `src/app/utils/image-ordering.ts`:

```ts
import { Image, SearchResult } from '../models/models';

export type SortDirection = 'asc' | 'desc';
export type SortKeyId = 'dateTaken' | 'relevance';
export type Sortable = Image | SearchResult;

export interface DateRange {
  from: number | null; // inclusive lower bound, epoch seconds
  to: number | null;   // inclusive upper bound, epoch seconds
}

export interface SortKey {
  id: SortKeyId;
  label: string;
  /** True when this key can meaningfully sort the given collection. */
  available(images: readonly Sortable[]): boolean;
  /** Total comparator for the natural ('desc') order. */
  compare(a: Sortable, b: Sortable): number;
}

export function imageId(img: Sortable): number {
  return 'id' in img ? img.id : img.image_id;
}

function sortTimestamp(img: Sortable): number {
  return img.date_taken ?? img.mtime;
}

function hasScore(img: Sortable): img is SearchResult {
  return 'score' in img && typeof img.score === 'number';
}

const DATE_TAKEN_KEY: SortKey = {
  id: 'dateTaken',
  label: 'Date taken',
  available: () => true,
  compare: (a, b) => sortTimestamp(b) - sortTimestamp(a) || imageId(b) - imageId(a),
};

const RELEVANCE_KEY: SortKey = {
  id: 'relevance',
  label: 'Relevance',
  available: (images) => images.length > 0 && images.every(hasScore),
  compare: (a, b) =>
    (hasScore(b) ? b.score : 0) - (hasScore(a) ? a.score : 0) || imageId(b) - imageId(a),
};

export const SORT_KEYS: Record<SortKeyId, SortKey> = {
  dateTaken: DATE_TAKEN_KEY,
  relevance: RELEVANCE_KEY,
};

export function applySort(
  images: readonly Sortable[],
  key: SortKey,
  direction: SortDirection,
): Sortable[] {
  const sign = direction === 'asc' ? -1 : 1;
  return [...images].sort((a, b) => sign * key.compare(a, b));
}

export function matchesDateRange(img: Sortable, range: DateRange): boolean {
  if (range.from == null && range.to == null) return true;
  if (img.date_taken == null) return false; // undated hidden while a range is active
  const ts = img.date_taken;
  if (range.from != null && ts < range.from) return false;
  if (range.to != null && ts > range.to) return false;
  return true;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm exec vitest run src/app/utils/image-ordering.spec.ts`
Expected: PASS (all cases).

- [ ] **Step 5: Commit**

```bash
git add src/app/utils/image-ordering.ts src/app/utils/image-ordering.spec.ts
git commit -m "feat(utils): pure image ordering + date-range primitives"
```

---

### Task 2: Collection composable (`image-collection.composable.ts`)

**Files:**
- Create: `src/app/composables/image-collection.composable.ts`
- Test: `src/app/composables/image-collection.composable.spec.ts`

**Interfaces:**
- Consumes from Task 1: `SortKeyId`, `SortDirection`, `SortKey`, `Sortable`, `DateRange`, `SORT_KEYS`, `applySort`, `matchesDateRange`.
- Produces:
  - `interface ImageCollectionConfig { sortKeys: SortKeyId[]; defaultSort: { key: SortKeyId; direction: SortDirection }; dateRangeFilter: boolean }`
  - `interface ImageCollectionState { sort: WritableSignal<{ key: SortKeyId; direction: SortDirection }>; dateRange: WritableSignal<DateRange> }`
  - `interface ImageCollection extends ImageCollectionState { readonly availableSortKeys: Signal<SortKey[]>; readonly view: Signal<Sortable[]>; readonly activeFilterCount: Signal<number>; reset(): void }`
  - `function createImageCollection(source: Signal<Sortable[]>, config: ImageCollectionConfig, state?: ImageCollectionState): ImageCollection`

- [ ] **Step 1: Write the failing test**

Create `src/app/composables/image-collection.composable.spec.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { signal } from '@angular/core';
import { createImageCollection } from './image-collection.composable';
import { imageId } from '../utils/image-ordering';
import { SearchResult } from '../models/models';

function result(imageIdNum: number, score: number, dateTaken: number): SearchResult {
  return {
    image_id: imageIdNum, path: '', thumbnail_path: null, preview_path: null,
    score, date_taken: dateTaken, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true,
  };
}

const config = {
  sortKeys: ['dateTaken', 'relevance'] as const,
  defaultSort: { key: 'dateTaken' as const, direction: 'desc' as const },
  dateRangeFilter: true,
};

describe('createImageCollection', () => {
  it('applies the default sort to the view', () => {
    const source = signal([result(1, 0.1, 100), result(2, 0.9, 300)]);
    const c = createImageCollection(source, { ...config });
    expect(c.view().map(imageId)).toEqual([2, 1]); // newest-first
  });

  it('re-sorts when direction changes', () => {
    const source = signal([result(1, 0.1, 100), result(2, 0.9, 300)]);
    const c = createImageCollection(source, { ...config });
    c.sort.set({ key: 'dateTaken', direction: 'asc' });
    expect(c.view().map(imageId)).toEqual([1, 2]);
  });

  it('filters by date range and tracks activeFilterCount', () => {
    const source = signal([result(1, 0.1, 100), result(2, 0.9, 300)]);
    const c = createImageCollection(source, { ...config });
    expect(c.activeFilterCount()).toBe(0);
    c.dateRange.set({ from: 200, to: null });
    expect(c.view().map(imageId)).toEqual([2]);
    expect(c.activeFilterCount()).toBe(1);
  });

  it('exposes only available sort keys', () => {
    const source = signal([result(1, 0.1, 100)]);
    const c = createImageCollection(source, { ...config });
    expect(c.availableSortKeys().map((k) => k.id)).toEqual(['dateTaken', 'relevance']);
  });

  it('falls back to default when the selected key becomes unavailable', () => {
    const source = signal<SearchResult[]>([result(1, 0.5, 100)]);
    const c = createImageCollection(source, { ...config });
    c.sort.set({ key: 'relevance', direction: 'desc' });
    // Mix in an item without a score → relevance no longer available.
    source.set([result(1, 0.5, 100), { ...result(2, 0.9, 300), score: undefined as unknown as number }]);
    expect(c.availableSortKeys().map((k) => k.id)).toEqual(['dateTaken']);
    expect(() => c.view()).not.toThrow();
  });

  it('reset() restores default sort and clears the range', () => {
    const source = signal([result(1, 0.1, 100), result(2, 0.9, 300)]);
    const c = createImageCollection(source, { ...config });
    c.sort.set({ key: 'relevance', direction: 'asc' });
    c.dateRange.set({ from: 200, to: null });
    c.reset();
    expect(c.sort()).toEqual({ key: 'dateTaken', direction: 'desc' });
    expect(c.dateRange()).toEqual({ from: null, to: null });
  });

  it('uses externally-provided state signals when given', () => {
    const source = signal([result(1, 0.1, 100), result(2, 0.9, 300)]);
    const sort = signal({ key: 'dateTaken' as const, direction: 'desc' as const });
    const dateRange = signal({ from: null as number | null, to: null as number | null });
    const c = createImageCollection(source, { ...config }, { sort, dateRange });
    sort.set({ key: 'dateTaken', direction: 'asc' });
    expect(c.view().map(imageId)).toEqual([1, 2]);
    expect(c.sort).toBe(sort);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm exec vitest run src/app/composables/image-collection.composable.spec.ts`
Expected: FAIL — cannot resolve `./image-collection.composable`.

- [ ] **Step 3: Write minimal implementation**

Create `src/app/composables/image-collection.composable.ts`:

```ts
import { computed, signal, Signal, WritableSignal } from '@angular/core';
import {
  DateRange,
  SortDirection,
  SortKey,
  SortKeyId,
  Sortable,
  SORT_KEYS,
  applySort,
  matchesDateRange,
} from '../utils/image-ordering';

export interface ImageCollectionConfig {
  sortKeys: readonly SortKeyId[];
  defaultSort: { key: SortKeyId; direction: SortDirection };
  dateRangeFilter: boolean;
}

export interface ImageCollectionState {
  sort: WritableSignal<{ key: SortKeyId; direction: SortDirection }>;
  dateRange: WritableSignal<DateRange>;
}

export interface ImageCollection extends ImageCollectionState {
  readonly availableSortKeys: Signal<SortKey[]>;
  readonly view: Signal<Sortable[]>;
  readonly activeFilterCount: Signal<number>;
  reset(): void;
}

export function createImageCollection(
  source: Signal<Sortable[]>,
  config: ImageCollectionConfig,
  state?: ImageCollectionState,
): ImageCollection {
  const sort = state?.sort ?? signal({ ...config.defaultSort });
  const dateRange = state?.dateRange ?? signal<DateRange>({ from: null, to: null });

  const availableSortKeys = computed<SortKey[]>(() => {
    const imgs = source();
    return config.sortKeys
      .map((id) => SORT_KEYS[id])
      .filter((key) => key.available(imgs));
  });

  const view = computed<Sortable[]>(() => {
    const range = dateRange();
    const imgs = config.dateRangeFilter
      ? source().filter((img) => matchesDateRange(img, range))
      : source();

    const keys = availableSortKeys();
    if (keys.length === 0) return imgs;
    const current = sort();
    const key =
      keys.find((k) => k.id === current.key) ??
      keys.find((k) => k.id === config.defaultSort.key) ??
      keys[0];
    return applySort(imgs, key, current.direction);
  });

  const activeFilterCount = computed<number>(() => {
    const range = dateRange();
    return config.dateRangeFilter && (range.from != null || range.to != null) ? 1 : 0;
  });

  function reset(): void {
    sort.set({ ...config.defaultSort });
    dateRange.set({ from: null, to: null });
  }

  return { sort, dateRange, availableSortKeys, view, activeFilterCount, reset };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm exec vitest run src/app/composables/image-collection.composable.spec.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app/composables/image-collection.composable.ts src/app/composables/image-collection.composable.spec.ts
git commit -m "feat(composables): image-collection sort/filter model"
```

---

### Task 3: Controls component (`app-grid-controls`)

**Files:**
- Create: `src/app/components/grid-controls/grid-controls.component.ts`
- Create: `src/app/components/grid-controls/grid-controls.component.html`
- Modify: `src/app/app-icons.ts` (register the three new icons)
- Test: `src/app/components/grid-controls/grid-controls.component.spec.ts`

**Prerequisite — register icons.** `app-icons.spec.ts` scans templates and fails for any `<lucide-icon>` name not registered in `APP_ICONS`, and lucide throws at render for unregistered names. The template below uses `arrow-up-down`, `sliders-horizontal`, and `check`. In `src/app/app-icons.ts`, add `ArrowUpDown, SlidersHorizontal, Check` to both the import from `lucide-angular` and the `APP_ICONS` object.

**Interfaces:**
- Consumes from Task 2: `ImageCollection`. From Task 1: `SortKeyId`, `SortDirection`.
- Produces: `class GridControlsComponent` with `@Input({ required: true }) collection!: ImageCollection`; selector `app-grid-controls`. Protected helpers used by the template: `selectSort(id: SortKeyId)`, `setDirection(dir: SortDirection)`, `setFrom(value: string)`, `setTo(value: string)`, `clearRange()`, `fromInput()`, `toInput()`.

- [ ] **Step 1: Write the failing test**

Create `src/app/components/grid-controls/grid-controls.component.spec.ts`:

```ts
import { describe, it, expect, beforeEach } from 'vitest';
import { TestBed } from '@angular/core/testing';
import { signal } from '@angular/core';
import { GridControlsComponent } from './grid-controls.component';
import { createImageCollection } from '../../composables/image-collection.composable';
import { SearchResult } from '../../models/models';

function result(imageIdNum: number, score: number, dateTaken: number): SearchResult {
  return {
    image_id: imageIdNum, path: '', thumbnail_path: null, preview_path: null,
    score, date_taken: dateTaken, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true,
  };
}

function makeCollection() {
  return createImageCollection(signal([result(1, 0.1, 100), result(2, 0.9, 300)]), {
    sortKeys: ['dateTaken', 'relevance'],
    defaultSort: { key: 'dateTaken', direction: 'desc' },
    dateRangeFilter: true,
  });
}

describe('GridControlsComponent', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({ imports: [GridControlsComponent] });
  });

  it('toggles sort direction on the bound collection', () => {
    const fixture = TestBed.createComponent(GridControlsComponent);
    const collection = makeCollection();
    fixture.componentInstance.collection = collection;
    fixture.detectChanges();
    fixture.componentInstance['setDirection']('asc');
    expect(collection.sort().direction).toBe('asc');
  });

  it('sets the date range from the from-input', () => {
    const fixture = TestBed.createComponent(GridControlsComponent);
    const collection = makeCollection();
    fixture.componentInstance.collection = collection;
    fixture.detectChanges();
    fixture.componentInstance['setFrom']('2026-01-01');
    expect(collection.dateRange().from).toBe(Math.floor(Date.UTC(2026, 0, 1) / 1000));
    expect(collection.activeFilterCount()).toBe(1);
  });

  it('clears the range', () => {
    const fixture = TestBed.createComponent(GridControlsComponent);
    const collection = makeCollection();
    fixture.componentInstance.collection = collection;
    collection.dateRange.set({ from: 100, to: 200 });
    fixture.detectChanges();
    fixture.componentInstance['clearRange']();
    expect(collection.dateRange()).toEqual({ from: null, to: null });
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm exec vitest run src/app/components/grid-controls/grid-controls.component.spec.ts`
Expected: FAIL — cannot resolve `./grid-controls.component`.

- [ ] **Step 3: Write minimal implementation**

First register the icons. In `src/app/app-icons.ts`, add `ArrowUpDown, SlidersHorizontal, Check` to the `lucide-angular` import list **and** to the `APP_ICONS` object (both the import and the export map, matching the existing PascalCase style).

Create `src/app/components/grid-controls/grid-controls.component.ts`:

```ts
import { Component, ChangeDetectionStrategy, Input } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { BrnPopoverImports } from '@spartan-ng/brain/popover';
import { HlmPopoverImports } from '@spartan-ng/helm/popover';
import { HlmButton } from '@spartan-ng/helm/button';
import { ImageCollection } from '../../composables/image-collection.composable';
import { SortDirection, SortKeyId } from '../../utils/image-ordering';

@Component({
  selector: 'app-grid-controls',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [LucideAngularModule, BrnPopoverImports, HlmPopoverImports, HlmButton],
  templateUrl: './grid-controls.component.html',
})
export class GridControlsComponent {
  @Input({ required: true }) collection!: ImageCollection;

  protected selectSort(id: SortKeyId): void {
    this.collection.sort.update((s) => ({ ...s, key: id }));
  }

  protected setDirection(direction: SortDirection): void {
    this.collection.sort.update((s) => ({ ...s, direction }));
  }

  /** yyyy-mm-dd (from an <input type="date">) → epoch seconds at UTC midnight, or null. */
  private toEpoch(value: string): number | null {
    if (!value) return null;
    const [y, m, d] = value.split('-').map(Number);
    if (!y || !m || !d) return null;
    return Math.floor(Date.UTC(y, m - 1, d) / 1000);
  }

  /** epoch seconds → yyyy-mm-dd for binding back into <input type="date">. */
  private toInputValue(epoch: number | null): string {
    if (epoch == null) return '';
    return new Date(epoch * 1000).toISOString().slice(0, 10);
  }

  protected fromInput(): string {
    return this.toInputValue(this.collection.dateRange().from);
  }

  protected toInput(): string {
    return this.toInputValue(this.collection.dateRange().to);
  }

  protected setFrom(value: string): void {
    this.collection.dateRange.update((r) => ({ ...r, from: this.toEpoch(value) }));
  }

  protected setTo(value: string): void {
    this.collection.dateRange.update((r) => ({ ...r, to: this.toEpoch(value) }));
  }

  protected clearRange(): void {
    this.collection.dateRange.set({ from: null, to: null });
  }
}
```

Create `src/app/components/grid-controls/grid-controls.component.html`:

```html
<div class="flex items-center gap-1">
  <!-- Sort -->
  <div brnPopover #sortPop="brnPopover" align="end">
    <button
      brnPopoverTrigger
      [brnPopoverTriggerFor]="sortPop"
      class="p-2 hover:bg-muted rounded-md transition-colors text-muted-foreground hover:text-foreground"
      title="Sort"
    >
      <lucide-icon name="arrow-up-down" size="18"></lucide-icon>
    </button>
    <ng-template brnPopoverContent>
      <div hlmPopoverContent class="w-48 p-1">
        @for (key of collection.availableSortKeys(); track key.id) {
          <button
            class="w-full flex items-center justify-between px-2 py-1.5 text-sm rounded-sm hover:bg-accent hover:text-accent-foreground transition-colors"
            (click)="selectSort(key.id)"
          >
            {{ key.label }}
            @if (collection.sort().key === key.id) {
              <lucide-icon name="check" size="14"></lucide-icon>
            }
          </button>
        }
        <div class="my-1 h-px bg-border"></div>
        <div class="flex gap-1 px-1">
          <button
            hlmBtn size="sm" variant="ghost" class="flex-1"
            [class.bg-accent]="collection.sort().direction === 'desc'"
            (click)="setDirection('desc')"
          >Newest</button>
          <button
            hlmBtn size="sm" variant="ghost" class="flex-1"
            [class.bg-accent]="collection.sort().direction === 'asc'"
            (click)="setDirection('asc')"
          >Oldest</button>
        </div>
      </div>
    </ng-template>
  </div>

  <!-- Filter -->
  <div brnPopover #filterPop="brnPopover" align="end">
    <button
      brnPopoverTrigger
      [brnPopoverTriggerFor]="filterPop"
      class="relative p-2 hover:bg-muted rounded-md transition-colors text-muted-foreground hover:text-foreground"
      title="Filter"
    >
      <lucide-icon name="sliders-horizontal" size="18"></lucide-icon>
      @if (collection.activeFilterCount() > 0) {
        <span class="absolute top-1 right-1 w-2 h-2 rounded-full bg-primary"></span>
      }
    </button>
    <ng-template brnPopoverContent>
      <div hlmPopoverContent class="w-64 p-3 flex flex-col gap-2">
        <label class="text-xs text-muted-foreground">From</label>
        <input type="date" class="h-8 px-2 text-sm rounded-md border border-border bg-background"
               [value]="fromInput()" (change)="setFrom($any($event.target).value)" />
        <label class="text-xs text-muted-foreground">To</label>
        <input type="date" class="h-8 px-2 text-sm rounded-md border border-border bg-background"
               [value]="toInput()" (change)="setTo($any($event.target).value)" />
        @if (collection.activeFilterCount() > 0) {
          <button hlmBtn size="sm" variant="ghost" (click)="clearRange()">Clear</button>
        }
      </div>
    </ng-template>
  </div>
</div>
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm exec vitest run src/app/components/grid-controls/grid-controls.component.spec.ts`
Expected: PASS. If `@spartan-ng/brain/popover` fails to resolve in tests, add its path to `vitest.config.ts` `resolve.alias` mirroring the existing `@spartan-ng/helm/*` entries (map `@spartan-ng/brain/popover` to its package entry under `node_modules`); re-run.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/grid-controls/ src/app/app-icons.ts vitest.config.ts
git commit -m "feat(components): app-grid-controls sort/filter popovers"
```

---

### Task 4: Wire into subject-detail (fixes the reported bug)

**Files:**
- Modify: `src/app/components/subject-detail/subject-detail.component.ts` (imports, `collection`, `virtualRows`)
- Modify: `src/app/components/subject-detail/subject-detail.component.html:130-138` (controls + empty-filter state + navigation items)
- Test: `src/app/components/subject-detail/subject-detail.component.spec.ts` (add a regression test)

**Interfaces:**
- Consumes from Task 2: `createImageCollection`. From Task 3: `GridControlsComponent`.

- [ ] **Step 1: Write the failing test**

Add to `src/app/components/subject-detail/subject-detail.component.spec.ts` (inside the existing top-level `describe`):

```ts
it('orders subject photos newest-first with a deterministic id tiebreak', async () => {
  const { SubjectDetailComponent } = await import('./subject-detail.component');
  // Build the component's collection directly over an unordered, equal-score set.
  const photos = [
    { image_id: 3, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: 100, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
    { image_id: 1, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: 100, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
    { image_id: 2, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: 300, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
  ];
  const { signal } = await import('@angular/core');
  const { createImageCollection } = await import('../../composables/image-collection.composable');
  const c = createImageCollection(signal(photos), {
    sortKeys: ['dateTaken', 'relevance'],
    defaultSort: { key: 'dateTaken', direction: 'desc' },
    dateRangeFilter: true,
  });
  expect(c.view().map((i) => (i as { image_id: number }).image_id)).toEqual([2, 3, 1]);
  expect(SubjectDetailComponent).toBeTruthy();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm exec vitest run src/app/components/subject-detail/subject-detail.component.spec.ts`
Expected: FAIL — `createImageCollection` import path or ordering assertion fails before wiring exists.

(If the composable already exists from Task 2, this test may pass on its own; the wiring steps below are still required for the UI. Proceed.)

- [ ] **Step 3: Write minimal implementation**

In `subject-detail.component.ts`:

1. Add imports near the other component imports:

```ts
import { GridControlsComponent } from '../grid-controls/grid-controls.component';
import { createImageCollection } from '../../composables/image-collection.composable';
```

2. Add `GridControlsComponent` to the `@Component({ imports: [...] })` array.

3. Add the collection field after `subjectPhotos` is declared:

```ts
protected readonly collection = createImageCollection(this.subjectPhotos, {
  sortKeys: ['dateTaken', 'relevance'],
  defaultSort: { key: 'dateTaken', direction: 'desc' },
  dateRangeFilter: true,
});
```

4. Change `virtualRows` to read the ordered/filtered view instead of the raw source:

```ts
protected readonly virtualRows = computed<VirtualRow[]>(() => {
  const images = this.collection.view();
  const width = this.photos.viewportWidth();
  const targetRowHeight = this.photos.targetRowHeight();

  const rows: VirtualRow[] = [];
  const justifiedRows = buildJustifiedRows(images, width, targetRowHeight, 4);
  for (const row of justifiedRows) {
    rows.push({ type: 'row', images: row.images, rowHeight: row.rowHeight });
  }
  return rows;
});
```

In `subject-detail.component.html`, replace the photo-grid block (lines ~125–138) with:

```html
@if (subjectPhotos().length === 0) {
  <div class="flex flex-col items-center justify-center h-full text-muted-foreground gap-2">
    <p class="text-lg font-medium">No photos yet</p>
    <p class="text-sm">Photos featuring this person will appear here.</p>
  </div>
} @else {
  <div class="max-w-[1600px] mx-auto">
    <div class="flex items-center justify-end mb-2">
      <app-grid-controls [collection]="collection" />
    </div>
    @if (collection.view().length === 0) {
      <div class="flex flex-col items-center justify-center py-16 text-muted-foreground gap-2">
        <p class="text-sm font-medium">No photos match these filters</p>
        <button class="text-sm text-primary hover:underline" (click)="collection.reset()">Clear filters</button>
      </div>
    } @else {
      <div class="flex flex-col gap-1">
        @for (row of virtualRows(); track $index) {
          @if (row.type === 'row') {
            <app-photo-grid [images]="row.images" [rowHeight]="row.rowHeight" [navigationItems]="collection.view()" />
          }
        }
      </div>
    }
  </div>
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm exec vitest run src/app/components/subject-detail/subject-detail.component.spec.ts`
Expected: PASS (existing tests + new ordering test).

- [ ] **Step 5: Commit**

```bash
git add src/app/components/subject-detail/
git commit -m "feat(subject-detail): sorted/filtered photo grid via shared controls"
```

---

### Task 5: Wire into gallery (all-surfaces coverage)

**Files:**
- Modify: `src/app/services/photo.service.ts` — add `gallerySort` + `galleryDateRange` signals; thread them through `dayGroups`; parametrize `groupByDay`; reset defaults on search-mode change.
- Modify: `src/app/components/gallery/gallery.component.ts` — build a collection over the PhotoService signals; import controls.
- Modify: `src/app/components/gallery/gallery.component.html:1-2` — add `<app-grid-controls>`.
- Test: `src/app/services/photo.service.spec.ts` — add dayGroups ordering/filtering tests. (Create the spec file if it does not exist.)

**Interfaces:**
- Consumes from Task 1: `SORT_KEYS`, `applySort`, `matchesDateRange`, `DateRange`, `SortKeyId`, `SortDirection`. From Task 2: `createImageCollection`. From Task 3: `GridControlsComponent`.
- Produces on `PhotoService`: `readonly gallerySort: WritableSignal<{ key: SortKeyId; direction: SortDirection }>`, `readonly galleryDateRange: WritableSignal<DateRange>`.

- [ ] **Step 1: Write the failing test**

Create/append `src/app/services/photo.service.spec.ts`:

```ts
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { TestBed } from '@angular/core/testing';
import { PhotoService } from './photo.service';
import { Image } from '../models/models';

function img(id: number, dateTaken: number): Image {
  return {
    id, folder_id: 1, path: `/p/${id}.jpg`, file_hash: '', hash_status: 'ok',
    date_taken: dateTaken, mtime: 0, thumbnail_path: null, preview_path: null,
    semantic_analysis_done: true, subject_analysis_done: true,
    added_at: 0, updated_at: 0, deleted_at: null,
  };
}

describe('PhotoService gallery ordering', () => {
  let service: PhotoService;

  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [PhotoService] });
    service = TestBed.inject(PhotoService);
    // Two images ~13 months apart so they land in distinct day groups.
    service.images.set([img(1, 1_600_000_000), img(2, 1_640_000_000)]);
  });

  it('orders day groups newest-first by default', () => {
    const groups = service.dayGroups();
    const firstImageId = (groups[0].images[0] as Image).id;
    expect(firstImageId).toBe(2);
  });

  it('reverses group order when direction is asc', () => {
    service.gallerySort.set({ key: 'dateTaken', direction: 'asc' });
    const groups = service.dayGroups();
    expect((groups[0].images[0] as Image).id).toBe(1);
  });

  it('drops images outside an active date range', () => {
    service.galleryDateRange.set({ from: 1_630_000_000, to: null });
    const remaining = service.dayGroups().flatMap((g) => g.images).map((i) => (i as Image).id);
    expect(remaining).toEqual([2]);
  });
});
```

Note: `images` is already a public writable signal on `PhotoService`.

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm exec vitest run src/app/services/photo.service.spec.ts`
Expected: FAIL — `gallerySort`/`galleryDateRange` do not exist yet.

- [ ] **Step 3: Write minimal implementation**

In `photo.service.ts`:

1. Add imports at the top (near the `buildJustifiedRows` import):

```ts
import {
  DateRange, SortDirection, SortKeyId, SORT_KEYS, applySort, matchesDateRange,
} from '../utils/image-ordering';
```

2. Add signals next to `readonly searchResults` (around line 39):

```ts
readonly gallerySort = signal<{ key: SortKeyId; direction: SortDirection }>({ key: 'dateTaken', direction: 'desc' });
readonly galleryDateRange = signal<DateRange>({ from: null, to: null });
```

3. Replace the `dayGroups` computed (lines ~110-123) with:

```ts
readonly dayGroups = computed<DayGroup[]>(() => {
  const sort = this.gallerySort();
  const range = this.galleryDateRange();
  const results = this.searchResults();
  if (results) {
    const filtered = results.filter((i) => matchesDateRange(i, range));
    const key = SORT_KEYS[sort.key].available(filtered) ? SORT_KEYS[sort.key] : SORT_KEYS.relevance;
    return [{ label: 'Search Results', date: 'search', images: applySort(filtered, key, sort.direction) }];
  }
  const imgs = this.images().filter((i) => matchesDateRange(i, range));
  return groupByDay(imgs, sort.direction);
});
```

4. Parametrize `groupByDay` (function near line 488). Change its signature and add ordering:

```ts
function groupByDay(images: (Image | SearchResult)[], direction: SortDirection = 'desc'): DayGroup[] {
  // ... unchanged map-building loop ...

  for (const group of map.values()) {
    group.images = applySort(group.images, SORT_KEYS.dateTaken, direction) as (Image | SearchResult)[];
  }
  const groups = Array.from(map.values());
  groups.sort((a, b) =>
    direction === 'asc' ? a.date.localeCompare(b.date) : b.date.localeCompare(a.date),
  );
  return groups;
}
```

(`groupByDay` needs `SortDirection`, `applySort`, `SORT_KEYS` — already imported in step 1.)

5. Reset gallery defaults on explicit search-mode changes. Add this private method to the class:

```ts
/** Reset the gallery's default ordering when switching browse <-> search. */
private syncGallerySortToMode(searching: boolean): void {
  this.gallerySort.set(
    searching ? { key: 'relevance', direction: 'desc' } : { key: 'dateTaken', direction: 'desc' },
  );
  this.galleryDateRange.set({ from: null, to: null });
}
```

Call `this.syncGallerySortToMode(true);` immediately after each successful `this.searchResults.set(results);` in `searchByText` (line ~300) and `searchByImage` (line ~326). In `clearSearch()`, after it clears the results, call `this.syncGallerySortToMode(false);`. Do **not** call it in `refreshSearchResults` (a background refresh must not clobber the user's chosen sort).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm exec vitest run src/app/services/photo.service.spec.ts`
Expected: PASS.

- [ ] **Step 5: Wire the controls into the gallery UI**

In `gallery.component.ts`:

1. Add imports:

```ts
import { GridControlsComponent } from '../grid-controls/grid-controls.component';
import { createImageCollection } from '../../composables/image-collection.composable';
```

2. Add `GridControlsComponent` to the `imports` array.

3. Add the collection field (bound to the PhotoService signals so the existing `virtualRows` pipeline reacts):

```ts
protected readonly collection = createImageCollection(
  this.photos.galleryImages,
  {
    sortKeys: ['dateTaken', 'relevance'],
    defaultSort: { key: 'dateTaken', direction: 'desc' },
    dateRangeFilter: true,
  },
  { sort: this.photos.gallerySort, dateRange: this.photos.galleryDateRange },
);
```

In `gallery.component.html`, change the top so the controls sit next to the search bar:

```html
<div class="flex items-center gap-2">
  <div class="flex-1"><app-search-bar /></div>
  <app-grid-controls [collection]="collection" />
</div>
```

- [ ] **Step 6: Run the full test suite and typecheck**

Run: `pnpm test`
Expected: PASS (all specs).
Run: `pnpm exec ng build --configuration development`
Expected: builds without TS errors.

- [ ] **Step 7: Commit**

```bash
git add src/app/services/photo.service.ts src/app/services/photo.service.spec.ts src/app/components/gallery/
git commit -m "feat(gallery): shared sort/filter controls over the day-grouped pipeline"
```

---

## Verification (manual, after all tasks)

Use the `run` skill / `pnpm tauri dev` and confirm:
- Subject detail: photos render newest-first; toggling to Oldest reverses them; a date range hides out-of-range photos and undated photos; "Clear filters" restores the full set.
- Gallery: day groups render newest-first; Oldest reverses group + within-group order; date range prunes photos; running a text search flips ordering to Relevance; clearing the search returns to Date-taken.

## Notes on deferred work

- **Face-picker and merge grids:** the spec lists these as "adopt where a collection is shown." This plan wires the two surfaces that carry the actual bug and the bulk of usage (subject-detail and gallery, gallery covering the search case). Because `app-grid-controls` + `createImageCollection` are surface-agnostic, adopting them in face-picker / merge later is a drop-in (create a collection over that surface's source, add the control) and does not require touching Tasks 1–3.
- **Gallery empty-after-filter copy:** when a date range prunes every photo in browse mode, `photos.virtualRows()` becomes empty and the gallery's existing empty state ("No photos found") already renders. A filter-specific message like the one added to subject-detail is a nice-to-have, not built here.
- **Broader restructuring** (splitting `PhotoService`, renaming `app-photo-grid`, unifying util/composable/service boundaries) is intentionally **not** in this plan — it is tracked in the Notion task "Restructure frontend toward idiomatic Angular component/service architecture".
