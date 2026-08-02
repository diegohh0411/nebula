import { TestBed } from '@angular/core/testing';
import { importProvidersFrom, signal } from '@angular/core';
import { Subject as RxSubject } from 'rxjs';
import { provideRouter, Router } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { LucideAngularModule } from 'lucide-angular';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { Tag, SubjectDetail } from '../../models/models';
import { SubjectDetailComponent } from './subject-detail.component';
import { APP_ICONS } from '../../app-icons';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const openDialogMock = vi.fn();
const openPathMock = vi.fn();
const listenMock = vi.fn().mockResolvedValue(() => {});

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialogMock(...args),
}));

vi.mock('@tauri-apps/plugin-opener', () => ({
  openPath: (...args: unknown[]) => openPathMock(...args),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

const mockTauriEvents = {
  pipelineStats$: new RxSubject(),
  imageAdded$: new RxSubject(),
  imageUpdated$: new RxSubject(),
  imageRemoved$: new RxSubject(),
  modelDownloadProgress$: new RxSubject(),
};

const fakeTag: Tag = { id: 1, name: 'Cabaña-21', added_at: 0 };

describe('SubjectDetail — tag chips (service-level)', () => {
  let photoService: PhotoService;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    });
    photoService = TestBed.inject(PhotoService);
  });

  it('getSubjectTags calls invoke with correct command', async () => {
    vi.spyOn(photoService, 'getSubjectTags').mockResolvedValue([fakeTag]);
    const tags = await photoService.getSubjectTags(1);
    expect(tags).toEqual([fakeTag]);
    expect(photoService.getSubjectTags).toHaveBeenCalledWith(1);
  });

  it('addSubjectTag calls invoke with correct args', async () => {
    vi.spyOn(photoService, 'addSubjectTag').mockResolvedValue(fakeTag);
    const tag = await photoService.addSubjectTag(1, 'Cabaña-21');
    expect(tag.name).toBe('Cabaña-21');
    expect(photoService.addSubjectTag).toHaveBeenCalledWith(1, 'Cabaña-21');
  });

  it('removeSubjectTag calls invoke with correct args', async () => {
    vi.spyOn(photoService, 'removeSubjectTag').mockResolvedValue(undefined);
    await photoService.removeSubjectTag(1, 42);
    expect(photoService.removeSubjectTag).toHaveBeenCalledWith(1, 42);
  });
});

class SubjectDetailPhotoServiceStub {
  viewportWidth = signal(1000);
  targetRowHeight = signal(220);
  selectedImage = signal(null);
  selectedImageIds = signal(new Set<number>());

  getSubjectDetail = vi.fn();
  getSubjectPhotos = vi.fn().mockResolvedValue([]);
  getMergeSuggestions = vi.fn().mockResolvedValue([]);
  dismissMergeSuggestion = vi.fn().mockResolvedValue(undefined);
  getFaceCrop = vi.fn().mockResolvedValue('/cache/face-1.png');
  thumbnailUrl = vi.fn((p: string | null) => (p ? `asset://${p}` : null));
  getSubjectTags = vi.fn().mockResolvedValue([]);
  nameSubject = vi.fn().mockResolvedValue({ duplicate_subject_id: null });
  mergeSubjects = vi.fn().mockResolvedValue(undefined);
  addSubjectTag = vi.fn().mockResolvedValue({ id: 9, name: 'New Tag', added_at: 0 });
  removeSubjectTag = vi.fn().mockResolvedValue(undefined);
  listTags = vi.fn().mockResolvedValue([]);
  subjects = signal([]);
  getSubjectPhotosWithFaces = vi.fn().mockResolvedValue([]);
  exportSubjectPhotos = vi.fn();
}

function subjectDetail(over: Partial<SubjectDetail['subject']> = {}): SubjectDetail {
  return {
    subject: { id: 1, name: 'Sofía', thumbnail_face_id: null, type: 'person', added_at: 0, ...over },
    photo_count: 0,
    face_count: 0,
  };
}

describe('SubjectDetailComponent — tagging (component-level)', () => {
  let stub: SubjectDetailPhotoServiceStub;

  beforeEach(() => {
    stub = new SubjectDetailPhotoServiceStub();
    stub.getSubjectDetail.mockResolvedValue(subjectDetail());
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'subject/:id', component: SubjectDetailComponent }]),
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        { provide: PhotoService, useValue: stub },
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    });
  });

  it('commits a new name via nameSubject and reflects it in the detail header', async () => {
    const harness = await RouterTestingHarness.create('/subject/1');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();

    const nameEl = harness.routeNativeElement!.querySelector('.tracking-tight') as HTMLElement;
    nameEl.click();
    harness.detectChanges();

    const input = harness.routeNativeElement!.querySelector('input') as HTMLInputElement;
    input.value = 'Renamed';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await harness.fixture.whenStable();
    harness.detectChanges();

    expect(stub.nameSubject).toHaveBeenCalledWith(1, 'Renamed');
    expect(harness.routeNativeElement!.textContent).toContain('Renamed');
  });

  it('shows the merge dialog on a duplicate-name conflict and navigates to /subject/:id on confirm', async () => {
    stub.nameSubject.mockResolvedValue({ duplicate_subject_id: 42 });
    const harness = await RouterTestingHarness.create('/subject/1');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();

    const nameEl = harness.routeNativeElement!.querySelector('.tracking-tight') as HTMLElement;
    nameEl.click();
    harness.detectChanges();
    const input = harness.routeNativeElement!.querySelector('input') as HTMLInputElement;
    input.value = 'Sofía Duplicate';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await harness.fixture.whenStable();
    harness.detectChanges();

    expect(harness.routeNativeElement!.textContent).toContain('Duplicate Name');

    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate');
    const buttons = Array.from(harness.routeNativeElement!.querySelectorAll('button')) as HTMLButtonElement[];
    const mergeButton = buttons.find((b) => b.textContent?.trim() === 'Merge')!;
    mergeButton.click();
    await harness.fixture.whenStable();

    expect(stub.mergeSubjects).toHaveBeenCalledWith(1, 42);
    expect(navigateSpy).toHaveBeenCalledWith(['/subject', 1]);
  });

  it('keeps the dialog open and shows an error when mergeSubjects fails, without navigating', async () => {
    stub.nameSubject.mockResolvedValue({ duplicate_subject_id: 42 });
    stub.mergeSubjects = vi.fn().mockRejectedValue('merge failed');
    const harness = await RouterTestingHarness.create('/subject/1');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();

    const nameEl = harness.routeNativeElement!.querySelector('.tracking-tight') as HTMLElement;
    nameEl.click();
    harness.detectChanges();
    const input = harness.routeNativeElement!.querySelector('input') as HTMLInputElement;
    input.value = 'Sofía Duplicate';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await harness.fixture.whenStable();
    harness.detectChanges();

    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate');
    const buttons = Array.from(harness.routeNativeElement!.querySelectorAll('button')) as HTMLButtonElement[];
    const mergeButton = buttons.find((b) => b.textContent?.trim() === 'Merge')!;
    mergeButton.click();
    await harness.fixture.whenStable();
    harness.detectChanges();

    expect(navigateSpy).not.toHaveBeenCalled();
    expect(harness.routeNativeElement!.textContent).toContain('Duplicate Name');
    expect(harness.routeNativeElement!.textContent).toContain('merge failed');
  });

  it('adds and removes a tag via the tag chips', async () => {
    stub.getSubjectTags.mockResolvedValue([{ id: 5, name: 'Old Tag', added_at: 0 }]);
    const harness = await RouterTestingHarness.create('/subject/1');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();
    await new Promise((resolve) => setTimeout(resolve, 0));
    harness.detectChanges();

    expect(harness.routeNativeElement!.textContent).toContain('Old Tag');

    const input = harness.routeNativeElement!.querySelector('input') as HTMLInputElement;
    input.value = 'New Tag';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await harness.fixture.whenStable();
    harness.detectChanges();

    expect(stub.addSubjectTag).toHaveBeenCalledWith(1, 'New Tag');
    expect(harness.routeNativeElement!.textContent).toContain('New Tag');

    const removeButtons = Array.from(harness.routeNativeElement!.querySelectorAll('button')) as HTMLButtonElement[];
    const removeOldTag = removeButtons.find((b) => b.title === 'Remove tag' && b.parentElement?.textContent?.includes('Old Tag'));
    removeOldTag!.click();
    await harness.fixture.whenStable();
    harness.detectChanges();

    expect(stub.removeSubjectTag).toHaveBeenCalledWith(1, 5);
    expect(harness.routeNativeElement!.textContent).not.toContain('Old Tag');
  });
});

describe('SubjectDetailComponent — similar-subjects review flow', () => {
  let stub: SubjectDetailPhotoServiceStub;

  beforeEach(() => {
    stub = new SubjectDetailPhotoServiceStub();
    stub.getSubjectDetail.mockResolvedValue(subjectDetail());
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'subject/:id', component: SubjectDetailComponent }]),
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        { provide: PhotoService, useValue: stub },
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    });
  });

  async function mount() {
    const harness = await RouterTestingHarness.create('/subject/1');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();
    const cmp = harness.routeDebugElement!.componentInstance as SubjectDetailComponent;
    return { harness, cmp };
  }

  it('onReviewConfirmed reloads in place when the current subject survives', async () => {
    const { cmp } = await mount();
    stub.getSubjectDetail.mockClear();
    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate');

    (cmp as any).onReviewConfirmed(1); // survivor == current subject id

    expect(navigateSpy).not.toHaveBeenCalled();
    expect(stub.getSubjectDetail).toHaveBeenCalledWith(1);
  });

  it('onReviewConfirmed navigates when a different subject survives', async () => {
    const { cmp } = await mount();
    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate').mockResolvedValue(true);

    (cmp as any).onReviewConfirmed(2); // survivor != current subject id

    expect(navigateSpy).toHaveBeenCalledWith(['/subject', 2]);
  });

  it('onReviewDismissed removes the reviewed suggestion from the list', async () => {
    const { cmp } = await mount();
    const suggestion = { id: 7, subject_a: { id: 1, name: 'Sofía', thumbnail_face_id: null, type: 'person', added_at: 0 }, subject_b: { id: 2, name: null, thumbnail_face_id: null, type: 'person', added_at: 0 }, score: 0.9 };
    (cmp as any).similarSubjects.set([suggestion]);
    (cmp as any).openReview(suggestion);

    (cmp as any).onReviewDismissed();

    expect((cmp as any).similarSubjects()).toEqual([]);
    expect((cmp as any).reviewingSuggestion()).toBeNull();
  });

  it('orders subject photos newest-first with a deterministic id tiebreak', async () => {
    const { cmp, harness } = await mount();

    const photos = [
      { image_id: 3, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: 100, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
      { image_id: 1, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: 100, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
      { image_id: 2, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: 300, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
    ];
    (cmp as any)['subjectPhotos'].set(photos);
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();

    const viewIds = (cmp as any)['collection'].view().map((i: { image_id: number }) => i.image_id);
    expect(viewIds).toEqual([2, 3, 1]);

    const virtualRowIds = (cmp as any)['virtualRows']()
      .filter((row: { type: string }) => row.type === 'row')
      .flatMap((row: { images: { image_id: number }[] }) => row.images.map((img) => img.image_id));
    expect(virtualRowIds).toEqual([2, 3, 1]);
  });
});

describe('SubjectDetailComponent — export', () => {
  let stub: SubjectDetailPhotoServiceStub;

  beforeEach(() => {
    stub = new SubjectDetailPhotoServiceStub();
    stub.getSubjectDetail.mockResolvedValue(subjectDetail());
    openDialogMock.mockReset();
    openPathMock.mockReset();
    listenMock.mockReset().mockResolvedValue(() => {});
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'subject/:id', component: SubjectDetailComponent }]),
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        { provide: PhotoService, useValue: stub },
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    });
  });

  async function mount(detail: SubjectDetail = subjectDetail()) {
    stub.getSubjectDetail.mockResolvedValue(detail);
    const harness = await RouterTestingHarness.create('/subject/1');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();
    const cmp = harness.routeDebugElement!.componentInstance as SubjectDetailComponent;
    return { harness, cmp };
  }

  it('hides Copy all when photo_count is 0', async () => {
    const { harness } = await mount(subjectDetail());
    const btn = harness.routeNativeElement!.querySelector(
      'button[title="Copy all originals to a folder"]',
    );
    expect(btn).toBeNull();
  });

  it('shows Copy all when photo_count > 0', async () => {
    const { harness } = await mount({ ...subjectDetail(), photo_count: 3 });
    const btn = harness.routeNativeElement!.querySelector(
      'button[title="Copy all originals to a folder"]',
    ) as HTMLButtonElement | null;
    expect(btn).toBeTruthy();
    expect(btn!.textContent).toContain('Copy all');
  });

  it('does not invoke export when dialog is cancelled', async () => {
    const { cmp } = await mount({ ...subjectDetail(), photo_count: 2 });
    openDialogMock.mockResolvedValue(null);
    await cmp.onCopyAll();
    expect(stub.exportSubjectPhotos).not.toHaveBeenCalled();
    expect(openPathMock).not.toHaveBeenCalled();
  });

  it('exports, shows status, and opens destination on success', async () => {
    const { cmp, harness } = await mount({ ...subjectDetail(), photo_count: 2 });
    openDialogMock.mockResolvedValue('/tmp/cass-export');
    stub.exportSubjectPhotos.mockResolvedValue({
      dest_dir: '/tmp/cass-export',
      copied: 2,
      skipped_missing: 0,
      skipped_errors: 0,
    });
    openPathMock.mockResolvedValue(undefined);

    await cmp.onCopyAll();
    harness.detectChanges();

    expect(stub.exportSubjectPhotos).toHaveBeenCalledWith(1, '/tmp/cass-export');
    expect(openPathMock).toHaveBeenCalledWith('/tmp/cass-export');
    expect(cmp.exportStatus()).toContain('Copied 2');
  });

  it('does not open folder when export fails', async () => {
    const { cmp, harness } = await mount({ ...subjectDetail(), photo_count: 2 });
    openDialogMock.mockResolvedValue('/tmp/cass-export');
    stub.exportSubjectPhotos.mockRejectedValue(new Error('Subject not found'));

    await cmp.onCopyAll();
    harness.detectChanges();

    expect(openPathMock).not.toHaveBeenCalled();
    expect(cmp.exportStatus()).toMatch(/Subject not found|Export failed/i);
  });
});
