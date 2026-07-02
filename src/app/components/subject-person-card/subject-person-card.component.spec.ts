import { TestBed } from '@angular/core/testing';
import { provideRouter, Router } from '@angular/router';
import { SubjectPersonCardComponent } from './subject-person-card.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch } from '../../models/models';

class PhotoServiceStub {
  getFaceCrop = vi.fn().mockResolvedValue('/cache/face-7.png');
  thumbnailUrl = vi.fn((p: string | null) => (p ? `asset://${p}` : null));
  nameSubject = vi.fn().mockResolvedValue({ duplicate_subject_id: null });
  mergeSubjects = vi.fn().mockResolvedValue(undefined);
  addSubjectTag = vi.fn().mockResolvedValue({ id: 9, name: 'New Tag', added_at: 0 });
  removeSubjectTag = vi.fn().mockResolvedValue(undefined);
  getSubjectTags = vi.fn().mockResolvedValue([]);
  listTags = vi.fn().mockResolvedValue([]);
}

function match(over: Partial<SubjectMatch['subject']> = {}, tags: SubjectMatch['tags'] = []): SubjectMatch {
  return {
    subject: { id: 1, name: 'Sofía', thumbnail_face_id: 7, type: 'person', added_at: 0, ...over },
    tags,
  };
}

describe('SubjectPersonCardComponent', () => {
  let stub: PhotoServiceStub;

  beforeEach(() => {
    stub = new PhotoServiceStub();
    TestBed.configureTestingModule({
      imports: [SubjectPersonCardComponent],
      providers: [provideRouter([]), { provide: PhotoService, useValue: stub }],
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

  it('does not navigate when clicking the name to edit it', () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match());
    fixture.detectChanges();

    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate');

    const nameEl = fixture.nativeElement.querySelector('.person-card-name') as HTMLElement;
    nameEl.click();
    fixture.detectChanges();

    expect(navigateSpy).not.toHaveBeenCalled();
  });

  it('commits a new name via nameSubject and updates the display', async () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match());
    fixture.detectChanges();

    const nameEl = fixture.nativeElement.querySelector('.person-card-name') as HTMLElement;
    nameEl.click();
    fixture.detectChanges();

    const input = fixture.nativeElement.querySelector('.person-card-meta input') as HTMLInputElement;
    input.value = 'Renamed';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await fixture.whenStable();
    fixture.detectChanges();

    expect(stub.nameSubject).toHaveBeenCalledWith(1, 'Renamed');
    expect(fixture.nativeElement.querySelector('.person-card-name').textContent).toContain('Renamed');
  });

  it('shows the merge dialog and emits merged() on a duplicate-name conflict', async () => {
    stub.nameSubject = vi.fn().mockResolvedValue({ duplicate_subject_id: 42 });
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match());
    const mergedSpy = vi.fn();
    fixture.componentInstance.merged.subscribe(mergedSpy);
    fixture.detectChanges();

    const nameEl = fixture.nativeElement.querySelector('.person-card-name') as HTMLElement;
    nameEl.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('.person-card-meta input') as HTMLInputElement;
    input.value = 'Sofía Duplicate';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.textContent).toContain('Duplicate Name');

    const buttons = Array.from(fixture.nativeElement.querySelectorAll('button')) as HTMLButtonElement[];
    const mergeButton = buttons.find((b) => b.textContent?.trim() === 'Merge')!;
    mergeButton.click();
    await fixture.whenStable();

    expect(stub.mergeSubjects).toHaveBeenCalledWith(1, 42);
    expect(mergedSpy).toHaveBeenCalled();
  });

  it('adds a tag via addSubjectTag and emits tagAdded', async () => {
    stub.getSubjectTags = vi.fn().mockResolvedValue([{ id: 9, name: 'New Tag', added_at: 0 }]);
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match());
    const tagAddedSpy = vi.fn();
    fixture.componentInstance.tagAdded.subscribe(tagAddedSpy);
    fixture.detectChanges();

    const input = fixture.nativeElement.querySelector('.person-card-add-tag input') as HTMLInputElement;
    input.value = 'New Tag';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await fixture.whenStable();
    fixture.detectChanges();

    expect(stub.addSubjectTag).toHaveBeenCalledWith(1, 'New Tag');
    expect(tagAddedSpy).toHaveBeenCalledWith({ id: 9, name: 'New Tag', added_at: 0 });
    expect(fixture.nativeElement.textContent).toContain('New Tag');
  });

  it('removes a tag via removeSubjectTag, emits tagRemoved, and does not navigate', async () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match({}, [{ id: 5, name: 'Old Tag', added_at: 0 }]));
    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate');
    const tagRemovedSpy = vi.fn();
    fixture.componentInstance.tagRemoved.subscribe(tagRemovedSpy);
    fixture.detectChanges();

    const removeBtn = fixture.nativeElement.querySelector('.person-card-tag-remove') as HTMLButtonElement;
    removeBtn.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(stub.removeSubjectTag).toHaveBeenCalledWith(1, 5);
    expect(tagRemovedSpy).toHaveBeenCalledWith(5);
    expect(navigateSpy).not.toHaveBeenCalled();
    expect(fixture.nativeElement.textContent).not.toContain('Old Tag');
  });

  it('surfaces an error and keeps the tag when removeSubjectTag fails', async () => {
    stub.removeSubjectTag = vi.fn().mockRejectedValue('boom');
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match({}, [{ id: 5, name: 'Old Tag', added_at: 0 }]));
    const tagRemovedSpy = vi.fn();
    fixture.componentInstance.tagRemoved.subscribe(tagRemovedSpy);
    fixture.detectChanges();

    const removeBtn = fixture.nativeElement.querySelector('.person-card-tag-remove') as HTMLButtonElement;
    removeBtn.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(tagRemovedSpy).not.toHaveBeenCalled();
    expect(fixture.nativeElement.textContent).toContain('Old Tag');
    expect(fixture.nativeElement.querySelector('.person-card-tag-error')?.textContent).toContain('boom');
  });

  it('shows an error and closes the dialog without emitting merged when mergeSubjects fails', async () => {
    stub.nameSubject = vi.fn().mockResolvedValue({ duplicate_subject_id: 42 });
    stub.mergeSubjects = vi.fn().mockRejectedValue('merge failed');
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match());
    const mergedSpy = vi.fn();
    fixture.componentInstance.merged.subscribe(mergedSpy);
    fixture.detectChanges();

    const nameEl = fixture.nativeElement.querySelector('.person-card-name') as HTMLElement;
    nameEl.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('.person-card-meta input') as HTMLInputElement;
    input.value = 'Sofía Duplicate';
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await fixture.whenStable();
    fixture.detectChanges();

    const buttons = Array.from(fixture.nativeElement.querySelectorAll('button')) as HTMLButtonElement[];
    const mergeButton = buttons.find((b) => b.textContent?.trim() === 'Merge')!;
    mergeButton.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(stub.mergeSubjects).toHaveBeenCalledWith(1, 42);
    expect(mergedSpy).not.toHaveBeenCalled();
    expect(fixture.nativeElement.textContent).toContain('Duplicate Name');
    expect(fixture.nativeElement.textContent).toContain('merge failed');
  });
});
