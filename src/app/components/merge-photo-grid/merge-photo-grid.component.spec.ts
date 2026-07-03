import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { importProvidersFrom } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { MergePhotoGridComponent } from './merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectPhotoFace } from '../../models/models';
import { APP_ICONS } from '../../app-icons';

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
  let unassignFace: ReturnType<typeof vi.fn>;
  let openLightbox: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    getFaceCrop = vi.fn((faceId: number) => Promise.resolve(`/crops/${faceId}.webp`));
    unassignFace = vi.fn().mockResolvedValue(undefined);
    openLightbox = vi.fn();

    await TestBed.configureTestingModule({
      imports: [MergePhotoGridComponent],
      providers: [
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        {
          provide: PhotoService,
          useValue: {
            getFaceCrop,
            thumbnailUrl: (p: string | null) => p,
            openLightbox,
            unassignFace,
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
    component.images = [makePhoto(1, 100), makePhoto(2, 200), makePhoto(3, 300)];

    // Click the middle cell.
    (component as unknown as { onClick: (p: SubjectPhotoFace) => void }).onClick(component.images[1]);

    expect(openLightbox).toHaveBeenCalledTimes(1);
    const [clicked, list] = openLightbox.mock.calls[0];
    expect(clicked.image_id).toBe(200);
    expect(list.map((i: { image_id: number }) => i.image_id)).toEqual([100, 200, 300]);
  });

  it('does not render a remove badge when removable is false (default)', () => {
    component.images = [makePhoto(1, 100)];
    fixture.detectChanges();

    const badge = fixture.debugElement.query(By.css('button[aria-label="Remove face from subject"]'));
    expect(badge).toBeNull();
  });

  it('renders one remove badge per cell when removable is true', () => {
    component.images = [makePhoto(1, 100), makePhoto(2, 200)];
    component.removable = true;
    fixture.detectChanges();

    const badges = fixture.debugElement.queryAll(By.css('button[aria-label="Remove face from subject"]'));
    expect(badges.length).toBe(2);
  });

  it('clicking the remove badge unassigns the face, emits removed, and does not open the lightbox', async () => {
    component.images = [makePhoto(1, 100), makePhoto(2, 200)];
    component.removable = true;
    fixture.detectChanges();

    const removedSpy = vi.fn();
    component.removed.subscribe(removedSpy);

    const badge = fixture.debugElement.queryAll(By.css('button[aria-label="Remove face from subject"]'))[0];
    badge.nativeElement.click();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(unassignFace).toHaveBeenCalledWith(1);
    expect(removedSpy).toHaveBeenCalledWith(1);
    expect(openLightbox).not.toHaveBeenCalled();
  });

  it('disables the remove badge when only one face remains', () => {
    component.images = [makePhoto(1, 100)];
    component.removable = true;
    fixture.detectChanges();

    const badge = fixture.debugElement.query(By.css('button[aria-label="Remove face from subject"]'));
    expect(badge.nativeElement.disabled).toBe(true);
  });

  it('logs an error and does not emit removed when unassignFace fails', async () => {
    unassignFace.mockRejectedValueOnce(new Error('db error'));
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    component.images = [makePhoto(1, 100), makePhoto(2, 200)];
    component.removable = true;
    fixture.detectChanges();

    const removedSpy = vi.fn();
    component.removed.subscribe(removedSpy);

    const badge = fixture.debugElement.queryAll(By.css('button[aria-label="Remove face from subject"]'))[0];
    badge.nativeElement.click();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(errorSpy).toHaveBeenCalled();
    expect(removedSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });
});
