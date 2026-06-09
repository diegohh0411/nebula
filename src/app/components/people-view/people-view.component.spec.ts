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

  it('calls nameSubject with id and trimmed name when Enter is pressed', async () => {
    photoService.subjects.set([makeSubject(1, null)]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    fixture.detectChanges();

    const hint = fixture.debugElement.query(By.css('[data-testid="add-name-hint"]'));
    hint.triggerEventHandler('click', new MouseEvent('click'));
    fixture.detectChanges();

    component.editingName.set('  Alice  ');
    fixture.detectChanges();

    const input = fixture.debugElement.query(By.css('[data-testid="name-input"]'));
    input.triggerEventHandler('keydown', new KeyboardEvent('keydown', { key: 'Enter' }));
    await fixture.whenStable();

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, 'Alice');
  });

  it('shows name on card immediately before nameSubject resolves (optimistic update)', async () => {
    photoService.subjects.set([makeSubject(1, null)]);
    let resolve!: (v: { duplicate_subject_id: null }) => void;
    vi.spyOn(photoService, 'nameSubject').mockReturnValue(
      new Promise(r => { resolve = r; })
    );
    fixture.detectChanges();

    const hint = fixture.debugElement.query(By.css('[data-testid="add-name-hint"]'));
    hint.triggerEventHandler('click', new MouseEvent('click'));
    fixture.detectChanges();

    component.editingName.set('Alice');
    const input = fixture.debugElement.query(By.css('[data-testid="name-input"]'));
    input.triggerEventHandler('keydown', new KeyboardEvent('keydown', { key: 'Enter' }));
    fixture.detectChanges();

    // Name is visible before service resolves
    expect(photoService.subjects()[0].name).toBe('Alice');
    resolve({ duplicate_subject_id: null });
  });

  it('clicking the card link does not enter editing mode', () => {
    photoService.subjects.set([makeSubject(1, null)]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    fixture.detectChanges();

    const cardLink = fixture.debugElement.query(By.css('a[routerLink]'));
    cardLink.triggerEventHandler('click', new MouseEvent('click', { bubbles: true }));
    fixture.detectChanges();

    expect(component.editingSubjectId()).toBeNull();
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
    component.editingSubjectId.set(1);
    component.editingName.set('Emma');
    await component['commitName'](current);

    const conflict = component['namingConflict']();
    expect(conflict).not.toBeNull();
    expect(conflict!.id).toBe(-1);
    expect(conflict!.subject_a.id).toBe(2);
    expect(conflict!.subject_b.id).toBe(1);
    expect(conflict!.score).toBe(1.0);
  });
});
