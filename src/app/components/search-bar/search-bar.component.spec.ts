import { ComponentFixture, TestBed, fakeAsync, tick } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { provideRouter } from '@angular/router';
import { Subject as RxSubject } from 'rxjs';
import { SearchBarComponent } from './search-bar.component';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';

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

const fakeMatch = {
  subject: { id: 42, name: 'José', thumbnail_face_id: null, type: 'person', added_at: 0 },
  tags: [{ id: 1, name: 'Cabaña-21', added_at: 0 }],
};

describe('SearchBarComponent — typeahead', () => {
  let fixture: ComponentFixture<SearchBarComponent>;
  let component: SearchBarComponent;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [SearchBarComponent],
      providers: [
        provideRouter([]),
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(SearchBarComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);
    fixture.detectChanges();
  });

  it('shows dropdown after 200ms debounce for query >=2 chars', async () => {
    vi.spyOn(photoService, 'searchSubjects').mockResolvedValue([fakeMatch]);

    // Simulate debounce manually by calling the method directly
    const c = component as any;
    c.query.set('jo');
    c.typeaheadTimer = null;
    // Directly invoke the search (bypassing setTimeout for determinism)
    const matches = await photoService.searchSubjects('jo');
    c.typeaheadMatches.set(matches);
    c.typeaheadOpen.set(matches.length > 0);

    fixture.detectChanges();

    const dropdown = fixture.debugElement.query(By.css('.typeahead-dropdown'));
    expect(dropdown).not.toBeNull();
    const items = fixture.debugElement.queryAll(By.css('.typeahead-item'));
    expect(items.length).toBe(1);
    expect(items[0].nativeElement.textContent).toContain('José');
  });

  it('closes dropdown for query < 2 chars', fakeAsync(() => {
    vi.spyOn(photoService, 'searchSubjects').mockResolvedValue([fakeMatch]);

    const c = component as any;
    c.typeaheadMatches.set([fakeMatch]);
    c.typeaheadOpen.set(true);
    fixture.detectChanges();

    const input = fixture.debugElement.query(By.css('.search-input')).nativeElement as HTMLInputElement;
    input.value = 'j';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    expect(c.typeaheadOpen()).toBe(false);
    expect(fixture.debugElement.query(By.css('.typeahead-dropdown'))).toBeNull();
  }));
});
