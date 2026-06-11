import { TestBed } from '@angular/core/testing';
import { Subject as RxSubject } from 'rxjs';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { Tag } from '../../models/models';

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
