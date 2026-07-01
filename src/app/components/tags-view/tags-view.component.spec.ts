import { TestBed } from '@angular/core/testing';
import { Subject as RxSubject } from 'rxjs';
import { By } from '@angular/platform-browser';
import { provideRouter, Router } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { TagWithCount, SubjectMatch } from '../../models/models';
import { TagsViewComponent } from './tags-view.component';
import { SubjectPersonCardComponent } from '../subject-person-card/subject-person-card.component';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const mockTauriEvents = {
  pipelineStats$: new RxSubject(),
  imageAdded$: new RxSubject(),
  imageUpdated$: new RxSubject(),
  imageRemoved$: new RxSubject(),
  modelDownloadProgress$: new RxSubject(),
};

const fakeTag: TagWithCount = { id: 1, name: 'Cabaña-21', added_at: 0, subject_count: 2 };
const fakeMatch: SubjectMatch = {
  subject: { id: 10, name: 'Maria', thumbnail_face_id: null, type: 'person', added_at: 0 },
  tags: [{ id: 1, name: 'Cabaña-21', added_at: 0 }],
};

describe('TagsView — service-level', () => {
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

  it('listTags returns TagWithCount array', async () => {
    vi.spyOn(photoService, 'listTags').mockResolvedValue([fakeTag]);
    const tags = await photoService.listTags();
    expect(tags).toEqual([fakeTag]);
    expect(tags[0].subject_count).toBe(2);
  });

  it('getTagSubjects returns SubjectMatch array', async () => {
    vi.spyOn(photoService, 'getTagSubjects').mockResolvedValue([fakeMatch]);
    const subjects = await photoService.getTagSubjects(1);
    expect(subjects).toEqual([fakeMatch]);
    expect(photoService.getTagSubjects).toHaveBeenCalledWith(1);
  });

  it('deleteTag calls the correct command', async () => {
    vi.spyOn(photoService, 'deleteTag').mockResolvedValue(undefined);
    await photoService.deleteTag(1);
    expect(photoService.deleteTag).toHaveBeenCalledWith(1);
  });
});

describe('TagsViewComponent — card event wiring', () => {
  class TagsPhotoServiceStub {
    listTags = vi.fn();
    getTagSubjects = vi.fn();
    createTag = vi.fn();
    renameTag = vi.fn();
    deleteTag = vi.fn();
  }

  const fakeTag: TagWithCount = { id: 1, name: 'Cabaña-21', added_at: 0, subject_count: 2 };
  const fakeMatch: SubjectMatch = {
    subject: { id: 10, name: 'Maria', thumbnail_face_id: null, type: 'person', added_at: 0 },
    tags: [{ id: 1, name: 'Cabaña-21', added_at: 0 }],
  };

  let stub: TagsPhotoServiceStub;

  beforeEach(() => {
    stub = new TagsPhotoServiceStub();
    stub.listTags.mockResolvedValue([fakeTag]);
    stub.getTagSubjects.mockResolvedValue([fakeMatch]);
    TestBed.configureTestingModule({
      imports: [TagsViewComponent],
      providers: [provideRouter([]), { provide: PhotoService, useValue: stub }],
    });
  });

  it('evicts the subject from the grid when its currently-viewed tag is removed', async () => {
    const fixture = TestBed.createComponent(TagsViewComponent);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const tagEntry = fixture.nativeElement.querySelector('.tag-entry') as HTMLElement;
    tagEntry.click();
    await fixture.whenStable();
    fixture.detectChanges();

    const card = fixture.debugElement.query(By.directive(SubjectPersonCardComponent));
    card.componentInstance.tagRemoved.emit(1);
    await fixture.whenStable();
    fixture.detectChanges();

    expect(stub.listTags).toHaveBeenCalledTimes(2);
    expect(fixture.nativeElement.textContent).toContain('No subjects with this tag.');
  });

  it('keeps the subject visible when a different tag is removed from it', async () => {
    const fixture = TestBed.createComponent(TagsViewComponent);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const tagEntry = fixture.nativeElement.querySelector('.tag-entry') as HTMLElement;
    tagEntry.click();
    await fixture.whenStable();
    fixture.detectChanges();

    const card = fixture.debugElement.query(By.directive(SubjectPersonCardComponent));
    card.componentInstance.tagRemoved.emit(999);
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.textContent).toContain('Maria');
  });
});

describe('TagsViewComponent — persisted tag selection', () => {
  class TagsPhotoServiceStub {
    listTags = vi.fn();
    getTagSubjects = vi.fn();
    createTag = vi.fn();
    renameTag = vi.fn();
    deleteTag = vi.fn();
  }

  const fakeTags: TagWithCount[] = [
    { id: 1, name: 'Cabaña-21', added_at: 0, subject_count: 2 },
    { id: 2, name: 'Beach', added_at: 0, subject_count: 1 },
  ];

  let stub: TagsPhotoServiceStub;

  beforeEach(() => {
    stub = new TagsPhotoServiceStub();
    stub.listTags.mockResolvedValue(fakeTags);
    stub.getTagSubjects.mockResolvedValue([]);
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'tags', component: TagsViewComponent }]),
        { provide: PhotoService, useValue: stub },
      ],
    });
  });

  it('restores the selected tag from the tag query param on load', async () => {
    const harness = await RouterTestingHarness.create('/tags?tag=2');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();

    expect(stub.getTagSubjects).toHaveBeenCalledWith(2);
    expect(harness.routeNativeElement?.textContent).toContain('Beach');
  });

  it('updates the tag query param when a tag is selected', async () => {
    const harness = await RouterTestingHarness.create('/tags');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();

    const tagEntry = harness.routeNativeElement!.querySelectorAll('.tag-entry')[0] as HTMLElement;
    tagEntry.click();
    await harness.fixture.whenStable();
    harness.detectChanges();

    const router = TestBed.inject(Router);
    expect(router.url).toBe('/tags?tag=1');
  });
});
