import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { SubjectPersonCardComponent } from './subject-person-card.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch } from '../../models/models';

class PhotoServiceStub {
  getFaceCrop = vi.fn().mockResolvedValue('/cache/face-7.png');
  thumbnailUrl = vi.fn((p: string | null) => (p ? `asset://${p}` : null));
}

function match(over: Partial<SubjectMatch['subject']> = {}): SubjectMatch {
  return {
    subject: { id: 1, name: 'Sofía', thumbnail_face_id: 7, type: 'person', added_at: 0, ...over },
    tags: [],
  };
}

describe('SubjectPersonCardComponent', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [SubjectPersonCardComponent],
      providers: [provideRouter([]), { provide: PhotoService, useClass: PhotoServiceStub }],
    });
  });

  it('loads and renders the face crop image when thumbnail_face_id is present', async () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match());
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const img = fixture.nativeElement.querySelector('img') as HTMLImageElement | null;
    expect(img).toBeTruthy();
    expect(img!.getAttribute('src')).toBe('asset:///cache/face-7.png');
  });

  it('renders the placeholder (no img) when there is no thumbnail_face_id', async () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match({ thumbnail_face_id: null }));
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('img')).toBeNull();
    expect(fixture.nativeElement.textContent).toContain('👤');
  });

  it('shows "Unnamed" when the subject has no name', () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match({ name: null }));
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('Unnamed');
  });
});
