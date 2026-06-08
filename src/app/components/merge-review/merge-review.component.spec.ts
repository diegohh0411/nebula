import { ComponentFixture, TestBed } from '@angular/core/testing';
import { MergeReviewComponent } from './merge-review.component';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { MergeSuggestion, SearchResult, Subject } from '../../models/models';
import { Subject as RxSubject } from 'rxjs';

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

const makePhoto = (id: number): SearchResult => ({
  image_id: id, path: `/img/${id}.jpg`, thumbnail_path: `/thumb/${id}.jpg`,
  preview_path: null, score: 0,
});

describe('MergeReviewComponent', () => {
  let component: MergeReviewComponent;
  let fixture: ComponentFixture<MergeReviewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MergeReviewComponent],
      providers: [
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
    vi.spyOn(photoService, 'getSubjectPhotos')
      .mockImplementation(async (id: number) =>
        id === 1 ? [makePhoto(10)] : [makePhoto(20)]
      );

    component.suggestion = suggestion; // triggers setter → loadPhotos()
    // Flush microtasks: loadPhotos() uses Promise.all with mocked async fns
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(photoService.getSubjectPhotos).toHaveBeenCalledWith(1);
    expect(photoService.getSubjectPhotos).toHaveBeenCalledWith(2);
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

  it('confirm calls mergeSubjects with correct target/source then emits confirmed', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotos').mockResolvedValue([]);
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
    vi.spyOn(photoService, 'getSubjectPhotos').mockResolvedValue([]);
    vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    const dismissedSpy = vi.fn();
    component.dismissed.subscribe(dismissedSpy);

    component.suggestion = makeSuggestion(subA, subB);
    await component.dismiss();

    expect(photoService.dismissMergeSuggestion).toHaveBeenCalledWith(1);
    expect(dismissedSpy).toHaveBeenCalled();
  });
});
