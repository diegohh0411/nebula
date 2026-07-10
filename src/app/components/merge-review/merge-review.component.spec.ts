import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { importProvidersFrom } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { MergeReviewComponent } from './merge-review.component';
import { MergePhotoGridComponent } from '../merge-photo-grid/merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { MergeSuggestion, SubjectPhotoFace, Subject } from '../../models/models';
import { Subject as RxSubject } from 'rxjs';
import { APP_ICONS } from '../../app-icons';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const makeSubject = (id: number, name: string | null): Subject => ({
  id, name, thumbnail_face_id: null, type: 'person', added_at: 0,
});

const makeSuggestion = (a: Subject, b: Subject): MergeSuggestion => ({
  id: 1, subject_a: a, subject_b: b, score: 0.92,
});

const makePhoto = (id: number, x = 0.5, y = 0.5, w = 0.3, h = 0.3): SubjectPhotoFace => ({
  face_id: id,
  image_id: id,
  path: `/img/${id}.jpg`,
  thumbnail_path: `/thumb/${id}.jpg`,
  preview_path: null,
  date_taken: null,
  mtime: 0,
  face_bbox: { x, y, w, h },
});

describe('MergeReviewComponent', () => {
  let component: MergeReviewComponent;
  let fixture: ComponentFixture<MergeReviewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MergeReviewComponent],
      providers: [
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$: new RxSubject(),
            imageAdded$: new RxSubject(),
            imageUpdated$: new RxSubject(),
            imageRemoved$: new RxSubject(),
            modelDownloadProgress$: new RxSubject(),
          },
        },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(MergeReviewComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);
  });

  it('creates', () => {
    expect(component).toBeTruthy();
  });

  it('loads photos for both subjects when suggestion is set', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    const suggestion = makeSuggestion(subA, subB);

    // Set spy BEFORE assigning suggestion so the setter's loadPhotos() call uses the mock
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces')
      .mockImplementation(async (id: number) =>
        id === 1 ? [makePhoto(10)] : [makePhoto(20)]
      );

    component.suggestion = suggestion; // triggers setter → loadPhotos()
    // Flush microtasks: loadPhotos() uses Promise.all with mocked async fns
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(photoService.getSubjectPhotosWithFaces).toHaveBeenCalledWith(1);
    expect(photoService.getSubjectPhotosWithFaces).toHaveBeenCalledWith(2);
    expect(component.photosA().length).toBe(1);
    expect(component.photosB().length).toBe(1);
  });

  it('mergeTarget returns named subject as target when one is named', () => {
    const named = makeSubject(2, 'Alice');
    const unnamed = makeSubject(1, null);
    component.suggestion = makeSuggestion(unnamed, named);
    expect(component.mergeTarget).toEqual({ target: named, source: unnamed });
  });

  it('mergeTarget returns lower id as target when both named', () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, 'Bob');
    component.suggestion = makeSuggestion(a, b);
    expect(component.mergeTarget).toEqual({ target: a, source: b });
  });

  it('marks only the source subject\'s grid as removable', async () => {
    const subA = makeSubject(1, 'Alice'); // named -> target
    const subB = makeSubject(2, null);    // unnamed -> source
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);

    component.suggestion = makeSuggestion(subA, subB);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    const grids = fixture.debugElement.queryAll(By.directive(MergePhotoGridComponent));
    expect(grids.length).toBe(2);
    expect((grids[0].componentInstance as MergePhotoGridComponent).removable).toBe(false); // col A = subject_a = target
    expect((grids[1].componentInstance as MergePhotoGridComponent).removable).toBe(true);  // col B = subject_b = source
  });

  it('removing a face from grid A filters it out of photosA', async () => {
    const subA = makeSubject(1, null);    // unnamed -> source
    const subB = makeSubject(2, 'Bob');   // named -> target
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) =>
      id === 1 ? [makePhoto(10), makePhoto(20)] : []
    );

    component.suggestion = makeSuggestion(subA, subB);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    const grids = fixture.debugElement.queryAll(By.directive(MergePhotoGridComponent));
    (grids[0].componentInstance as MergePhotoGridComponent).removed.emit(10);

    expect(component.photosA().map(f => f.face_id)).toEqual([20]);
  });

  it('removing a face from grid B filters it out of photosB', async () => {
    const subA = makeSubject(1, 'Alice'); // named -> target
    const subB = makeSubject(2, null);    // unnamed -> source
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) =>
      id === 2 ? [makePhoto(30), makePhoto(40)] : []
    );

    component.suggestion = makeSuggestion(subA, subB);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    const grids = fixture.debugElement.queryAll(By.directive(MergePhotoGridComponent));
    (grids[1].componentInstance as MergePhotoGridComponent).removed.emit(30);

    expect(component.photosB().map(f => f.face_id)).toEqual([40]);
  });

  it('confirm calls mergeSubjects with correct target/source then emits confirmed', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    const confirmedSpy = vi.fn();
    component.confirmed.subscribe(confirmedSpy);

    component.suggestion = makeSuggestion(subA, subB); // subA is named, subA is target
    await component.confirm();

    expect(photoService.mergeSubjects).toHaveBeenCalledWith(1, 2);
    expect(confirmedSpy).toHaveBeenCalled();
  });

  it('dismiss calls dismissMergeSuggestion then emits dismissed', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    const dismissedSpy = vi.fn();
    component.dismissed.subscribe(dismissedSpy);

    component.suggestion = makeSuggestion(subA, subB);
    await component.dismiss();

    expect(photoService.dismissMergeSuggestion).toHaveBeenCalledWith(1);
    expect(dismissedSpy).toHaveBeenCalled();
  });

  it('labels the left button "Not the same person" in the default (canDismiss=true) mode', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(subA, subB);
    fixture.detectChanges();

    const dismissBtn = fixture.debugElement.query(By.css('button[cdkFocusInitial]'));
    expect(dismissBtn.nativeElement.textContent.trim()).toBe('Not the same person');
  });

  it('with canDismiss=false, dismiss() emits dismissed without calling dismissMergeSuggestion', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    const dismissedSpy = vi.fn();
    component.dismissed.subscribe(dismissedSpy);

    component.canDismiss = false;
    component.suggestion = makeSuggestion(subA, subB);
    fixture.detectChanges();
    const dismissBtn = fixture.debugElement.query(By.css('button[cdkFocusInitial]'));
    expect(dismissBtn.nativeElement.textContent.trim()).toBe('Not the same person');

    await component.dismiss();

    expect(photoService.dismissMergeSuggestion).not.toHaveBeenCalled();
    expect(dismissedSpy).toHaveBeenCalled();
  });

  it('Case 1: committing a new unique name calls nameSubject and updates the signal', async () => {
    const subA = makeSubject(1, null);
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    photoService.subjects.set([subA, subB]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', 'Charlie');

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, 'Charlie');
    expect(component.subjectA()?.name).toBe('Charlie');
    expect(component.nameErrorA()).toBeNull();
  });

  it('Case 2: naming a column the OTHER column\'s name is allowed (no error)', async () => {
    const subA = makeSubject(1, null);
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: 2 });
    photoService.subjects.set([subA, subB]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', 'bob'); // case-insensitive match of other column

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, 'bob');
    expect(component.nameErrorA()).toBeNull();
  });

  it('Case 3: naming a column after a THIRD subject is blocked (no backend call, error shown)', async () => {
    const subA = makeSubject(1, null);
    const subB = makeSubject(2, 'Bob');
    const third = makeSubject(3, 'Jane');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const nameSpy = vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: 3 });
    photoService.subjects.set([subA, subB, third]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', 'jane'); // case-insensitive match of third subject

    expect(nameSpy).not.toHaveBeenCalled();
    expect(component.subjectA()?.name).toBeNull(); // reverted / unchanged
    expect(component.nameErrorA()).toContain('already exists');
  });

  it('committing an empty value clears the name', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    photoService.subjects.set([subA, subB]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', '   ');

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, null);
    expect(component.subjectA()?.name).toBeNull();
  });
});
