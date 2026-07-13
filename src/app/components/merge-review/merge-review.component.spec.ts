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

  it('mergeTarget uses targetOverride/redirectSource when a redirect is active', () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    component.suggestion = makeSuggestion(a, b); // normally: target=a, source=b

    const roberto = makeSubject(99, 'Roberto');
    (component as any).targetOverride.set(roberto);
    (component as any).redirectSource.set(b);

    expect(component.mergeTarget).toEqual({ target: roberto, source: b });
  });

  it('mergeTarget falls back to normal tiebreak when targetOverride is null', () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    component.suggestion = makeSuggestion(a, b);

    expect((component as any).targetOverride()).toBeNull();
    expect(component.mergeTarget).toEqual({ target: a, source: b });
  });

  it('assigning a new suggestion resets an active redirect from the previous suggestion', () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    component.suggestion = makeSuggestion(a, b);

    const roberto = makeSubject(99, 'Roberto');
    (component as any).targetOverride.set(roberto);
    (component as any).redirectSource.set(b);

    const c = makeSubject(3, 'Cara');
    const d = makeSubject(4, null);
    component.suggestion = makeSuggestion(c, d); // simulates advancing to the next review

    expect((component as any).targetOverride()).toBeNull();
    expect((component as any).redirectSource()).toBeNull();
    expect(component.mergeTarget).toEqual({ target: c, source: d }); // not the stale Roberto
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
    expect(confirmedSpy).toHaveBeenCalledWith(1); // subA (id 1) is the named target
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
    expect((component.nameErrorA() as any)?.message).toContain('already exists');
  });

  it('dismiss() shows the exit confirm instead of dismissing when names are identical', async () => {
    const subA = makeSubject(1, 'Noah');
    const subB = makeSubject(2, 'noah'); // case-insensitive identical
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(subA, subB);

    component.dismiss();

    expect(component.namesIdentical()).toBe(true);
    expect(component.showExitConfirm()).toBe(true);
    expect(dismissSpy).not.toHaveBeenCalled();
  });

  it('hides the primary actions while the exit confirm is shown', async () => {
    const subA = makeSubject(1, 'Noah');
    const subB = makeSubject(2, 'noah');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(subA, subB);
    fixture.detectChanges();

    component.dismiss(); // opens the guard
    fixture.detectChanges();

    const buttonLabels = fixture.debugElement
      .queryAll(By.css('.modal-actions button'))
      .map((b) => b.nativeElement.textContent.trim());
    expect(buttonLabels).toEqual(['Keep separate', 'Merge']);
  });

  it('with canDismiss=false, dismiss() emits dismissed directly even when names are identical (no exit confirm)', async () => {
    const subA = makeSubject(1, 'Noah');
    const subB = makeSubject(2, 'Noah');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    const dismissedSpy = vi.fn();
    component.dismissed.subscribe(dismissedSpy);

    component.canDismiss = false;
    component.suggestion = makeSuggestion(subA, subB);

    await component.dismiss();

    expect(component.namesIdentical()).toBe(true);
    expect(component.showExitConfirm()).toBe(false);
    expect(dismissSpy).not.toHaveBeenCalled();
    expect(dismissedSpy).toHaveBeenCalled();
  });

  it('keepSeparate() closes without calling dismissMergeSuggestion (no cannot_link)', async () => {
    const subA = makeSubject(1, 'Noah');
    const subB = makeSubject(2, 'Noah');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    const closedSpy = vi.fn();
    component.closed.subscribe(closedSpy);
    component.suggestion = makeSuggestion(subA, subB);

    component.dismiss();          // opens the guard
    component.keepSeparate();     // choose "Keep separate"

    expect(dismissSpy).not.toHaveBeenCalled();
    expect(closedSpy).toHaveBeenCalled();
    expect(component.showExitConfirm()).toBe(false);
  });

  it('dismiss() still dismisses directly when names differ', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(subA, subB);

    await component.dismiss();

    expect(component.showExitConfirm()).toBe(false);
    expect(dismissSpy).toHaveBeenCalledWith(1);
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

  it('applyRedirect sets override/source and reloads faces into the original keep slot', async () => {
    const a = makeSubject(1, 'Alice');   // named -> original target/keep, column A
    const b = makeSubject(2, null);      // unnamed -> original source, column B
    const roberto = makeSubject(99, 'Roberto');

    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) => {
      if (id === 99) return [makePhoto(500)];
      return [];
    });

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await (component as any).applyRedirect(roberto);

    expect((component as any).targetOverride()).toEqual(roberto);
    expect((component as any).redirectSource()).toEqual(b); // original non-target participant
    expect(photoService.getSubjectPhotosWithFaces).toHaveBeenCalledWith(99);
    // Column A held the original keep subject's (Alice's) faces; it now shows Roberto's.
    expect(component.photosA().map(f => f.face_id)).toEqual([500]);
    expect((component as any).showRedirectPicker()).toBe(false);
  });

  it('applyRedirect uses the explicit source when provided, not mergeTarget.source', async () => {
    const a = makeSubject(1, 'Alice');   // named -> original target/keep, column A
    const b = makeSubject(2, null);      // unnamed -> original source, column B
    const roberto = makeSubject(99, 'Roberto');

    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    // Simulate a Part-2-style collision caught while renaming the *target* column (Alice),
    // not the source column — mergeTarget.source is still `b`, but the actual redirect must
    // treat `a` as the source, since Alice is the one being redirected into Roberto.
    await (component as any).applyRedirect(roberto, a);

    expect((component as any).redirectSource()).toEqual(a);
  });

  it('a second applyRedirect call reloads into the same slot as the first, not a re-derived one', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(99, 'Roberto');
    const carla = makeSubject(100, 'Carla');

    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) => {
      if (id === 99) return [makePhoto(500)];
      if (id === 100) return [makePhoto(600)];
      return [];
    });

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await (component as any).applyRedirect(roberto);   // first pick -> column A (Alice's original slot)
    await (component as any).applyRedirect(carla);     // re-pick -> must still land in column A, not B

    expect(component.photosA().map(f => f.face_id)).toEqual([600]); // Carla's faces
    expect(component.photosB().map(f => f.face_id)).toEqual([]);    // B (the real candidate) untouched
  });

  it('shows the "Merge into someone else…" link in the normal footer, hidden while submitting', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    let link = fixture.debugElement.query(By.css('[data-test="redirect-link"]'));
    expect(link).toBeTruthy();

    component.submitting.set(true);
    fixture.detectChanges();
    link = fixture.debugElement.query(By.css('[data-test="redirect-link"]'));
    expect(link).toBeFalsy();
  });

  it('redirectCandidates excludes the current source and unnamed subjects, filters by query', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null); // source
    photoService.subjects.set([
      a, b,
      makeSubject(3, null),        // unnamed -> excluded
      makeSubject(4, 'Roberto'),
      makeSubject(5, 'Robert Sr.'),
    ]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    (component as any).openRedirectPicker();
    (component as any).redirectQuery.set('rob');

    const names = (component as any).redirectCandidates().map((s: any) => s.name);
    expect(names).toEqual(['Roberto', 'Robert Sr.']);
  });

  it('redirectCandidates is empty (not thrown) when query matches nothing', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    photoService.subjects.set([a, b, makeSubject(4, 'Roberto')]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    (component as any).openRedirectPicker();
    (component as any).redirectQuery.set('zzz-no-match');

    expect((component as any).redirectCandidates()).toEqual([]);
  });

  it('Enter on the highlighted candidate calls applyRedirect', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(4, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    const applySpy = vi.spyOn(component as any, 'applyRedirect').mockResolvedValue(undefined);
    (component as any).openRedirectPicker();
    (component as any).redirectQuery.set('Roberto');
    (component as any).redirectHighlight.set(0);

    (component as any).onRedirectKeydown({ key: 'Enter', preventDefault: () => {}, stopPropagation: () => {} } as unknown as KeyboardEvent);

    expect(applySpy).toHaveBeenCalledWith(roberto);
  });

  it('Escape closes the picker without applying a redirect and does not propagate', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    (component as any).openRedirectPicker();
    let propagated = false;
    (component as any).onRedirectKeydown({
      key: 'Escape',
      preventDefault: () => {},
      stopPropagation: () => { propagated = true; },
    } as unknown as KeyboardEvent);

    expect((component as any).showRedirectPicker()).toBe(false);
    expect((component as any).targetOverride()).toBeNull();
    expect(propagated).toBe(true); // confirms stopPropagation was called, not that it reached document
  });

  it('a real Escape keydown on the rendered redirect input does not close the whole modal', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const closedSpy = vi.fn();
    component.closed.subscribe(closedSpy);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    (component as any).openRedirectPicker();
    fixture.detectChanges();

    const input = fixture.debugElement.query(By.css('.redirect-input')).nativeElement as HTMLInputElement;
    const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    input.dispatchEvent(event);
    fixture.detectChanges();

    expect((component as any).showRedirectPicker()).toBe(false);
    expect(closedSpy).not.toHaveBeenCalled(); // the modal itself must still be open
  });

  it('loads an avatar crop for each redirect candidate', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = { ...makeSubject(4, 'Roberto'), thumbnail_face_id: 777 };
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('/crops/777.jpg');
    vi.spyOn(photoService, 'thumbnailUrl').mockImplementation((p) => p);

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    (component as any).openRedirectPicker();
    fixture.detectChanges();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(photoService.getFaceCrop).toHaveBeenCalledWith(777);
    expect((component as any).redirectAvatarUrls().get(4)).toBe('/crops/777.jpg');
  });

  it('leaves the avatar entry null when a candidate has no thumbnail_face_id', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const noThumb = makeSubject(4, 'Roberto'); // thumbnail_face_id: null via makeSubject
    photoService.subjects.set([a, b, noThumb]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    (component as any).openRedirectPicker();
    fixture.detectChanges();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect((component as any).redirectAvatarUrls().get(4) ?? null).toBeNull();
  });

  it('the redirected column shows the picked subject\'s name and keep badge, not the original subject\'s', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await (component as any).applyRedirect(roberto); // redirects column A (Alice's original slot)
    fixture.detectChanges();

    const colA = fixture.debugElement.query(By.css('.subject-col'));
    expect(colA.nativeElement.textContent).toContain('Roberto');
    expect(colA.nativeElement.textContent).not.toContain('Alice');
    expect(colA.query(By.css('.keep-badge'))).toBeTruthy();
  });

  it('the header match-% chip is hidden or relabeled once a redirect is active', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await (component as any).applyRedirect(roberto);
    fixture.detectChanges();

    const chip = fixture.debugElement.query(By.css('[data-test="match-score-chip"]'));
    expect(chip.nativeElement.textContent).not.toContain('%');
  });

  it('onNameCommit collision populates a structured error with the conflicting subject', async () => {
    const a = makeSubject(1, null);
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await component.onNameCommit('a', 'Roberto');

    expect(component.nameErrorA()).toEqual({
      message: 'A subject named "Roberto" already exists.',
      conflict: roberto,
    });
  });

  it('clicking "Merge into {name}" on the rendered collision error applies the redirect and never calls nameSubject', async () => {
    const a = makeSubject(1, null);
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const nameSubjectSpy = vi.spyOn(photoService, 'nameSubject');
    const applySpy = vi.spyOn(component as any, 'applyRedirect').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    await component.onNameCommit('a', 'Roberto');
    fixture.detectChanges();

    const button = fixture.debugElement.query(By.css('[data-test="collision-redirect-a"]'));
    expect(button).toBeTruthy();
    button.triggerEventHandler('click', null);

    expect(applySpy).toHaveBeenCalledWith(roberto, a); // explicit source: `a` is the renamed subject
    expect(nameSubjectSpy).not.toHaveBeenCalled();
  });

  it('a collision while renaming the currently-kept (target) column redirects that column, not mergeTarget.source', async () => {
    const alice = makeSubject(1, 'Alice');  // named -> mergeTarget.target (the "keep" subject)
    const b = makeSubject(2, null);         // unnamed -> mergeTarget.source
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([alice, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const mergeSpy = vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(alice, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    // User tries to rename Alice (the target/keep column) to "Roberto" -> collides.
    await component.onNameCommit('a', 'Roberto');
    const conflict = component.nameErrorA()!.conflict;
    await (component as any).applyRedirect(conflict, alice); // Part 2 must pass `alice`, not mergeTarget.source (`b`)
    await component.confirm();

    // Alice (the one actually renamed) is merged into Roberto; `b` is left completely alone.
    expect(mergeSpy).toHaveBeenCalledWith(9, 1);
  });

  it('confirm shows an error and reopens the picker if the redirected target no longer exists', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const mergeSpy = vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    (component as any).targetOverride.set(roberto);
    (component as any).redirectSource.set(b);
    // Roberto is no longer in the live subjects list (deleted elsewhere mid-flow).
    photoService.subjects.set([a, b]);

    await component.confirm();

    expect(mergeSpy).not.toHaveBeenCalled();
    expect((component as any).redirectGoneError()).toBe('Roberto is no longer available — pick another subject.');
    expect((component as any).showRedirectPicker()).toBe(true);
  });

  it('confirm proceeds normally for a redirected target that still exists', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const mergeSpy = vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    photoService.subjects.set([a, b, roberto]);
    (component as any).targetOverride.set(roberto);
    (component as any).redirectSource.set(b);

    await component.confirm();

    expect(mergeSpy).toHaveBeenCalledWith(9, 2);
  });

  it('the "no longer available" error is actually visible in the DOM, in the reopened picker', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    (component as any).targetOverride.set(roberto);
    (component as any).redirectSource.set(b);
    photoService.subjects.set([a, b]); // Roberto no longer present

    await component.confirm();
    fixture.detectChanges();

    const errorEl = fixture.debugElement.query(By.css('[data-test="redirect-gone-error"]'));
    expect(errorEl).toBeTruthy();
    expect(errorEl.nativeElement.textContent).toContain('no longer available');
  });
});
