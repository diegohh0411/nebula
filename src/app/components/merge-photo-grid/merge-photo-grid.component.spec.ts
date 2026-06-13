import { ComponentFixture, TestBed } from '@angular/core/testing';
import { MergePhotoGridComponent } from './merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectPhotoFace } from '../../models/models';

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
    // x: bbox -0.1..0.4, padded 0..0.5, center 25%
    // y: bbox 1.2..1.7, padH=0.1; y0=max(0,1.1)=1.1, y1=min(1,1.8)=1; both clamped to 1, center 100%
    expect(component.focus(img)).toEqual({ x: '25%', y: '100%' });
  });

  it('adds 20% context padding around the face bbox', () => {
    const img = makePhoto(3, 0.3, 0.3, 0.2, 0.2);
    // padded box 0.26..0.54, center 0.40 -> 40%
    expect(component.focus(img)).toEqual({ x: '40%', y: '40%' });
  });

  it('clamps padded bbox to image bounds', () => {
    const img = makePhoto(4, 0.05, 0.95, 0.1, 0.05);
    // padded x: max(0,0.03)=0.03 -> min(1,0.17)=0.17, center 10%
    // padded y: max(0,0.94)=0.94 -> min(1,1.0)=1.0, center 97%
    expect(component.focus(img)).toEqual({ x: '10%', y: '97%' });
  });
});
