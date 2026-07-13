import { describe, it, expect, beforeEach } from 'vitest';
import { TestBed } from '@angular/core/testing';
import { signal, importProvidersFrom } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { GridControlsComponent } from './grid-controls.component';
import { createImageCollection } from '../../composables/image-collection.composable';
import { SearchResult } from '../../models/models';
import { APP_ICONS } from '../../app-icons';

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
    TestBed.configureTestingModule({
      imports: [GridControlsComponent],
      providers: [importProvidersFrom(LucideAngularModule.pick(APP_ICONS))],
    });
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
    // Local-time start-of-day, matching how the gallery groups days.
    expect(collection.dateRange().from).toBe(Math.floor(new Date(2026, 0, 1, 0, 0, 0, 0).getTime() / 1000));
    expect(collection.activeFilterCount()).toBe(1);
  });

  it('sets an inclusive end-of-day To bound and round-trips it to the input', () => {
    const fixture = TestBed.createComponent(GridControlsComponent);
    const collection = makeCollection();
    fixture.componentInstance.collection = collection;
    fixture.detectChanges();
    fixture.componentInstance['setTo']('2026-01-15');
    expect(collection.dateRange().to).toBe(Math.floor(new Date(2026, 0, 15, 23, 59, 59, 999).getTime() / 1000));
    expect(fixture.componentInstance['toInput']()).toBe('2026-01-15');
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
