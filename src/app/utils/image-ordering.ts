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
