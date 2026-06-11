import { TestBed } from '@angular/core/testing';
import { Subject as RxSubject } from 'rxjs';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { SubjectMatch } from '../../models/models';

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

const fakeMatch: SubjectMatch = {
  subject: { id: 1, name: 'Maria', thumbnail_face_id: null, type: 'person', added_at: 0 },
  tags: [{ id: 1, name: 'Cabaña-21', added_at: 0 }],
};

describe('Gallery — subjectMatches signal (service-level)', () => {
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

  it('subjects row condition: shows when search active and matches non-empty', () => {
    photoService.searchResults.set([]);
    photoService.subjectMatches.set([fakeMatch]);

    // gallery.component.html renders row when searchResults() !== null && subjectMatches().length > 0
    const shouldShow = photoService.searchResults() !== null && photoService.subjectMatches().length > 0;
    expect(shouldShow).toBe(true);
    expect(photoService.subjectMatches()[0].subject.name).toBe('Maria');
  });

  it('subjects row condition: hidden when matches empty', () => {
    photoService.searchResults.set([]);
    photoService.subjectMatches.set([]);

    const shouldShow = photoService.searchResults() !== null && photoService.subjectMatches().length > 0;
    expect(shouldShow).toBe(false);
  });

  it('subjects row condition: hidden when not in search mode', () => {
    photoService.searchResults.set(null);
    photoService.subjectMatches.set([fakeMatch]);

    const shouldShow = photoService.searchResults() !== null && photoService.subjectMatches().length > 0;
    expect(shouldShow).toBe(false);
  });
});
