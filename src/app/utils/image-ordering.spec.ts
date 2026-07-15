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
