import { TestBed, fakeAsync, tick } from '@angular/core/testing';
import { Subject } from 'rxjs';
import { PhotoService } from './photo.service';
import { TauriEventsService } from './tauri-events.service';
import { ImageUpdatedEvent, PipelineStats, Image, SearchResult } from '../models/models';

/**
 * Yield to the microtask queue repeatedly until `predicate` holds (or `max`
 * ticks elapse). subjectMatches is populated by a fire-and-forget promise chain
 * inside searchByText, which needs an unpredictable number of microtask turns to
 * settle (search → search_subjects → signal set). Draining microtasks until the
 * value lands is deterministic and avoids the wall-clock races that made these
 * tests flaky in CI.
 */
async function flushUntil(predicate: () => boolean, max = 50): Promise<void> {
  for (let i = 0; i < max && !predicate(); i++) {
    await Promise.resolve();
  }
}

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((path: string) => path),
}));

describe('PhotoService — imageUpdated$ order-agnostic contract', () => {
  let service: PhotoService;
  let imageUpdated$: Subject<ImageUpdatedEvent>;

  beforeEach(() => {
    imageUpdated$ = new Subject<ImageUpdatedEvent>();

    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$: new Subject(),
            imageAdded$: new Subject(),
            imageUpdated$,
            imageRemoved$: new Subject(),
            modelDownloadProgress$: new Subject(),
          },
        },
      ],
    });

    service = TestBed.inject(PhotoService);
    vi.spyOn(service as any, 'refreshImages').mockResolvedValue(undefined);
    vi.spyOn(service as any, 'refreshSearchResults').mockResolvedValue(undefined);
  });

  it('calls refreshImages after auditTime window expires', fakeAsync(() => {
    imageUpdated$.next({ image_id: 1 });
    expect((service as any).refreshImages).not.toHaveBeenCalled();
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);
  }));

  it('coalesces rapid emits (stage-2 then stage-1 within 2 s) into one refresh', fakeAsync(() => {
    imageUpdated$.next({ image_id: 1 }); // stage-2 "analysis complete" fires first
    tick(100);
    imageUpdated$.next({ image_id: 1 }); // stage-1 "preview ready" fires 100 ms later
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);
  }));

  it('fires a second refresh when stage-1 arrives after the 2 s audit window has elapsed', fakeAsync(() => {
    imageUpdated$.next({ image_id: 1 }); // stage-2 fires
    tick(2000); // first audit window expires → first refresh
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);

    imageUpdated$.next({ image_id: 1 }); // stage-1 thumbnail task fires very late
    tick(2000); // second audit window → second refresh (UI corrects itself)
    expect((service as any).refreshImages).toHaveBeenCalledTimes(2);
  }));

  it('does not assume event order — stage-2 before stage-1 eventually shows thumbnail', fakeAsync(() => {
    // Worst case: stage-2 fires, UI refreshes and may see thumbnail_path = null.
    // Then stage-1 thumbnail-write fires and UI refreshes again, picking up the thumbnail.
    imageUpdated$.next({ image_id: 42 });
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);

    imageUpdated$.next({ image_id: 42 });
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(2);
    // Both calls are unconditional: the second one will find thumbnail_path set.
  }));
});

describe('PhotoService — subjectMatches signal', () => {
  let service: PhotoService;
  let invoke: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    const { invoke: mockInvoke } = await import('@tauri-apps/api/core');
    invoke = mockInvoke as ReturnType<typeof vi.fn>;
    invoke.mockResolvedValue([]);

    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$: new Subject(),
            imageAdded$: new Subject(),
            imageUpdated$: new Subject(),
            imageRemoved$: new Subject(),
            modelDownloadProgress$: new Subject(),
          },
        },
      ],
    });

    service = TestBed.inject(PhotoService);
    vi.spyOn(service as any, 'refreshImages').mockResolvedValue(undefined);
  });

  it('searchByText sets subjectMatches from search_subjects response', async () => {
    const fakeMatch = { subject: { id: 1, name: 'Maria', thumbnail_face_id: null, type: 'person', added_at: 0 }, tags: [{ id: 1, name: 'Cabaña-21', added_at: 0 }] };
    // Stub the service method directly rather than discriminating on the invoke
    // mock by command name. The module-level invoke mock's per-call
    // implementation is not reliably the same instance the service imported in
    // every environment (a vitest module-mock identity quirk that made this
    // green locally but red in CI); spying the instance method is deterministic.
    vi.spyOn(service as any, 'searchSubjects').mockResolvedValue([fakeMatch]);

    await service.searchByText('cabana');
    await flushUntil(() => service.subjectMatches().length > 0);
    expect(service.subjectMatches()).toEqual([fakeMatch]);
  });

  it('clearSearch empties subjectMatches', async () => {
    const fakeMatch = { subject: { id: 1, name: 'Jose', thumbnail_face_id: null, type: 'person', added_at: 0 }, tags: [] };
    vi.spyOn(service as any, 'searchSubjects').mockResolvedValue([fakeMatch]);

    await service.searchByText('jose');
    await flushUntil(() => service.subjectMatches().length > 0);
    expect(service.subjectMatches().length).toBe(1);

    service.clearSearch();
    expect(service.subjectMatches()).toEqual([]);
  });
});

describe('PhotoService — processing speed resilience & ETA', () => {
  let service: PhotoService;
  let pipelineStats$: Subject<PipelineStats>;

  beforeEach(() => {
    pipelineStats$ = new Subject<PipelineStats>();
    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$,
            imageAdded$: new Subject(),
            imageUpdated$: new Subject(),
            imageRemoved$: new Subject(),
            modelDownloadProgress$: new Subject(),
          },
        },
      ],
    });
    service = TestBed.inject(PhotoService);
  });

  it('passes through a zero speed mid-processing (no samples yet)', () => {
    pipelineStats$.next({ total_pending: 100, images_per_sec: 8 });
    expect(service.pipelineStats().images_per_sec).toBe(8);

    // Backend sampler refreshes every second; 0 means "no samples yet", not a missed heartbeat.
    pipelineStats$.next({ total_pending: 120, images_per_sec: 0 });
    expect(service.pipelineStats().images_per_sec).toBe(0); // passed through
    expect(service.pipelineStats().total_pending).toBe(120); // count updates
  });

  it('clears the speed once processing finishes (pending 0)', () => {
    pipelineStats$.next({ total_pending: 100, images_per_sec: 8 });
    pipelineStats$.next({ total_pending: 0, images_per_sec: 0 });
    expect(service.pipelineStats().images_per_sec).toBe(0);
  });

  it('computes etaSeconds as remaining / speed', () => {
    pipelineStats$.next({ total_pending: 120, images_per_sec: 8 });
    expect(service.etaSeconds()).toBe(15);
  });

  it('returns 0 etaSeconds when speed is zero', () => {
    pipelineStats$.next({ total_pending: 0, images_per_sec: 0 });
    expect(service.etaSeconds()).toBe(0);
  });
});

describe('PhotoService — lightbox navigation', () => {
  let service: PhotoService;

  const img = (id: number): Image => ({
    id,
    folder_id: 1,
    path: `/img/${id}.jpg`,
    file_hash: '',
    hash_status: 'ok',
    date_taken: null,
    mtime: 0,
    thumbnail_path: null,
    preview_path: null,
    semantic_analysis_done: true,
    subject_analysis_done: true,
    added_at: 0,
    updated_at: 0,
    deleted_at: null,
  });

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$: new Subject(),
            imageAdded$: new Subject(),
            imageUpdated$: new Subject(),
            imageRemoved$: new Subject(),
            modelDownloadProgress$: new Subject(),
          },
        },
      ],
    });
    service = TestBed.inject(PhotoService);
  });

  it('openLightbox stores the image and its source list', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[1], items);
    expect(service.selectedImage()).toBe(items[1]);
    expect(service.lightboxItems()).toBe(items);
  });

  it('navigateLightbox moves forward within the supplied list', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[0], items);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(2);
  });

  it('navigateLightbox wraps from last to first', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[2], items);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(1);
  });

  it('navigateLightbox wraps from first to last going backward', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[0], items);
    service.navigateLightbox(-1);
    expect((service.selectedImage() as Image).id).toBe(3);
  });

  it('navigateLightbox is a no-op when the source list is empty', () => {
    service.openLightbox(img(1), []);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(1);
  });

  it('navigateLightbox is a no-op when the current image is not in the list', () => {
    const items = [img(1), img(2)];
    service.openLightbox(img(99), items);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(99);
  });

  it('closeLightbox clears the source list', () => {
    const items = [img(1), img(2)];
    service.openLightbox(items[0], items);
    service.closeLightbox();
    expect(service.selectedImage()).toBeNull();
    expect(service.lightboxItems()).toEqual([]);
  });

  it('galleryImages flattens dayGroups in visual order (search results)', () => {
    const results: SearchResult[] = [
      { image_id: 10, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: null, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
      { image_id: 11, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: null, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
    ];
    service.searchResults.set(results);
    expect(service.galleryImages().map((i) => ('id' in i ? i.id : i.image_id))).toEqual([10, 11]);
  });
});
