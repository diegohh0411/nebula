import { ComponentFixture, TestBed } from '@angular/core/testing';
import { MergePhotoGridComponent } from './merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectPhotoFace } from '../../models/models';
import { Subject as RxSubject } from 'rxjs';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const makePhoto = (
  imageId: number,
  x: number,
  y: number,
  w: number,
  h: number
): SubjectPhotoFace => ({
  image_id: imageId,
  path: `/img/${imageId}.jpg`,
  thumbnail_path: `/thumb/${imageId}.jpg`,
  preview_path: null,
  date_taken: null,
  mtime: 0,
  face_bbox: { x, y, w, h },
});

describe('MergePhotoGridComponent', () => {
  let component: MergePhotoGridComponent;
  let fixture: ComponentFixture<MergePhotoGridComponent>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MergePhotoGridComponent],
      providers: [
        PhotoService,
        {
          provide: PhotoService,
          useValue: {
            thumbnailUrl: (p: string | null) => p,
            prioritizePreviews: vi.fn().mockResolvedValue(undefined),
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

  it('computes object-position from face bbox center', () => {
    const img = makePhoto(1, 0.25, 0.25, 0.5, 0.5);
    expect(component.focus(img)).toEqual({ x: '50%', y: '50%' });
  });

  it('clamps object-position to 0..100', () => {
    const img = makePhoto(2, -0.1, 1.2, 0.5, 0.5);
    expect(component.focus(img)).toEqual({ x: '15%', y: '100%' });
  });
});
