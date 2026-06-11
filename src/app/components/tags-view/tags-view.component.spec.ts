import { TestBed } from '@angular/core/testing';
import { Subject as RxSubject } from 'rxjs';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { TagWithCount, SubjectMatch } from '../../models/models';

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
