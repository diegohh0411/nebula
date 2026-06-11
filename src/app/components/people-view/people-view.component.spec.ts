import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { provideRouter } from '@angular/router';
import { Subject as RxSubject } from 'rxjs';
import { PeopleViewComponent } from './people-view.component';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { Subject } from '../../models/models';

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

const makeSubject = (id: number, name: string | null): Subject => ({
  id, name, thumbnail_face_id: null, type: 'person', added_at: 0,
});

describe('PeopleViewComponent — inline naming', () => {
  let component: PeopleViewComponent;
  let fixture: ComponentFixture<PeopleViewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [PeopleViewComponent],
      providers: [
        PhotoService,
        provideRouter([]),
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(PeopleViewComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);

    vi.spyOn(photoService, 'loadSubjects').mockResolvedValue(undefined);
    vi.spyOn(photoService, 'getMergeSuggestions').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('');
  });

  it('calls nameSubject with id and name when onNameCommit is invoked', async () => {
    const subject = makeSubject(1, null);
    photoService.subjects.set([subject]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    fixture.detectChanges();

    await component['onNameCommit'](subject, 'Alice');

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, 'Alice');
  });

  it('calls nameSubject with null when empty string is committed (name removal)', async () => {
    const subject = makeSubject(1, 'Alice');
    photoService.subjects.set([subject]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    fixture.detectChanges();

    await component['onNameCommit'](subject, '');

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, null);
  });

  it('shows name on card immediately before nameSubject resolves (optimistic update)', async () => {
    const subject = makeSubject(1, null);
    photoService.subjects.set([subject]);
    let resolve!: (v: { duplicate_subject_id: null }) => void;
    vi.spyOn(photoService, 'nameSubject').mockReturnValue(
      new Promise(r => { resolve = r; })
    );
    fixture.detectChanges();

    void component['onNameCommit'](subject, 'Alice');

    expect(photoService.subjects()[0].name).toBe('Alice');
    resolve({ duplicate_subject_id: null });
  });

  it('avatar link exists as a standalone anchor separate from the name edit area', () => {
    photoService.subjects.set([makeSubject(1, null)]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    fixture.detectChanges();

    const link = fixture.debugElement.query(By.css('a[data-testid="subject-link"]'));
    expect(link).not.toBeNull();
    // EditableText is not a child of the anchor — name edits don't trigger navigation
    const editableInsideLink = link.query(By.css('app-editable-text'));
    expect(editableInsideLink).toBeNull();
  });
});

describe('PeopleViewComponent — name conflict', () => {
  let component: PeopleViewComponent;
  let fixture: ComponentFixture<PeopleViewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [PeopleViewComponent],
      providers: [
        PhotoService,
        provideRouter([]),
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(PeopleViewComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);

    vi.spyOn(photoService, 'loadSubjects').mockResolvedValue(undefined);
    vi.spyOn(photoService, 'getMergeSuggestions').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('');
  });

  it('sets namingConflict with synthetic suggestion when duplicate_subject_id returned', async () => {
    const current = makeSubject(1, null);
    const duplicate = makeSubject(2, 'Emma');
    photoService.subjects.set([current, duplicate]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: 2 });

    fixture.detectChanges();
    await component['onNameCommit'](current, 'Emma');

    const conflict = component['namingConflict']();
    expect(conflict).not.toBeNull();
    expect(conflict!.id).toBe(-1);
    expect(conflict!.subject_a.id).toBe(2);
    expect(conflict!.subject_b.id).toBe(1);
    expect(conflict!.score).toBe(1.0);
  });

  it('clears originalSubjects map when duplicate_subject_id is not found in subjects list', async () => {
    const current = makeSubject(1, null);
    photoService.subjects.set([current]); // no subject with id 99
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: 99 });

    fixture.detectChanges();
    await component['onNameCommit'](current, 'Emma');

    expect(component['namingConflict']()).toBeNull();
    expect(component['_originalSubjects'].has(1)).toBe(false);
  });
});

describe('PeopleViewComponent — Tab key navigation', () => {
  let component: PeopleViewComponent;
  let fixture: ComponentFixture<PeopleViewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [PeopleViewComponent],
      providers: [
        PhotoService,
        provideRouter([]),
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(PeopleViewComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);

    vi.spyOn(photoService, 'loadSubjects').mockResolvedValue(undefined);
    vi.spyOn(photoService, 'getMergeSuggestions').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('');
  });

  it('onNameTab sets editingSubjectId to next unnamed subject', () => {
    const subject1 = makeSubject(1, null);
    const subject2 = makeSubject(2, null);
    photoService.subjects.set([subject1, subject2]);
    fixture.detectChanges();

    component['onNameTab'](subject1);

    expect(component.editingSubjectId()).toBe(2);
  });

  it('onNameTab does nothing when there is no next unnamed subject', () => {
    const subject1 = makeSubject(1, null);
    const subject2 = makeSubject(2, 'Bob');
    photoService.subjects.set([subject1, subject2]);
    fixture.detectChanges();

    component['onNameTab'](subject1);

    expect(component.editingSubjectId()).toBeNull();
  });
});

describe('PeopleViewComponent — error handling', () => {
  let component: PeopleViewComponent;
  let fixture: ComponentFixture<PeopleViewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [PeopleViewComponent],
      providers: [
        PhotoService,
        provideRouter([]),
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(PeopleViewComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);

    vi.spyOn(photoService, 'loadSubjects').mockResolvedValue(undefined);
    vi.spyOn(photoService, 'getMergeSuggestions').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('');
  });

  it('reverts to original snapshot (not null) when nameSubject throws', async () => {
    const original: Subject = { ...makeSubject(1, null), name: 'OriginalName' };
    photoService.subjects.set([original]);
    vi.spyOn(photoService, 'nameSubject').mockRejectedValue(new Error('network error'));

    fixture.detectChanges();
    await component['onNameCommit'](original, 'NewName');

    expect(photoService.subjects()[0].name).toBe('OriginalName');
  });
});
