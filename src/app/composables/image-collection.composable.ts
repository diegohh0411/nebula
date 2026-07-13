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
