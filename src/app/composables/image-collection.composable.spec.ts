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
