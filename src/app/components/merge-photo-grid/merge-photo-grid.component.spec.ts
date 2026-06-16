import { ComponentFixture, TestBed } from '@angular/core/testing';
import { MergePhotoGridComponent } from './merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectPhotoFace } from '../../models/models';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const makePhoto = (faceId: number, imageId: number): SubjectPhotoFace => ({
  face_id: faceId,
  image_id: imageId,
  path: `/img/${imageId}.jpg`,
  thumbnail_path: `/thumb/${imageId}.jpg`,
  preview_path: null,
  date_taken: null,
  mtime: 0,
  face_bbox: { x: 0, y: 0, w: 0.5, h: 0.5 },
});

describe('MergePhotoGridComponent', () => {
  let component: MergePhotoGridComponent;
  let fixture: ComponentFixture<MergePhotoGridComponent>;
  let getFaceCrop: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    getFaceCrop = vi.fn((faceId: number) => Promise.resolve(`/crops/${faceId}.webp`));

    await TestBed.configureTestingModule({
      imports: [MergePhotoGridComponent],
      providers: [
        {
          provide: PhotoService,
          useValue: {
            getFaceCrop,
            thumbnailUrl: (p: string | null) => p,
            openLightbox: vi.fn(),
          },
        },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(MergePhotoGridComponent);
    component = fixture.componentInstance;
  });

  it('creates', () => {
    expect(component).toBeTruthy();
  });

  it('loads the real face crop for a face and exposes its URL', async () => {
    const img = makePhoto(7, 1);
    component.images = [img];

    // No crop resolved yet.
    expect(component['cropUrl'](img)).toBeNull();

    await component['loadCrop'](7);

    expect(getFaceCrop).toHaveBeenCalledWith(7);
    expect(component['cropUrl'](img)).toBe('/crops/7.webp');
  });

  it('does not refetch a crop that is already cached', async () => {
    await component['loadCrop'](7);
    await component['loadCrop'](7);
    expect(getFaceCrop).toHaveBeenCalledTimes(1);
  });

  it('opens the lightbox with the full mapped list when a cell is clicked', () => {
    const openLightbox = TestBed.inject(PhotoService).openLightbox as ReturnType<typeof vi.fn>;
    component.images = [makePhoto(1, 100), makePhoto(2, 200), makePhoto(3, 300)];

    // Click the middle cell.
    (component as unknown as { onClick: (p: SubjectPhotoFace) => void }).onClick(component.images[1]);

    expect(openLightbox).toHaveBeenCalledTimes(1);
    const [clicked, list] = openLightbox.mock.calls[0];
    expect(clicked.image_id).toBe(200);
    expect(list.map((i: { image_id: number }) => i.image_id)).toEqual([100, 200, 300]);
  });
});
