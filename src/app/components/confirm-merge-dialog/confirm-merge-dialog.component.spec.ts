import { TestBed } from '@angular/core/testing';
import { ConfirmMergeDialogComponent } from './confirm-merge-dialog.component';

describe('ConfirmMergeDialogComponent', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({ imports: [ConfirmMergeDialogComponent] });
  });

  it('renders nothing when open is false', () => {
    const fixture = TestBed.createComponent(ConfirmMergeDialogComponent);
    fixture.componentRef.setInput('open', false);
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent.trim()).toBe('');
  });

  it('renders the dialog copy when open is true', () => {
    const fixture = TestBed.createComponent(ConfirmMergeDialogComponent);
    fixture.componentRef.setInput('open', true);
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('Duplicate Name');
    expect(fixture.nativeElement.textContent).toContain('A subject with this name already exists');
  });

  it('emits merge when the Merge button is clicked', () => {
    const fixture = TestBed.createComponent(ConfirmMergeDialogComponent);
    fixture.componentRef.setInput('open', true);
    const mergeSpy = vi.fn();
    fixture.componentInstance.merge.subscribe(mergeSpy);
    fixture.detectChanges();

    const buttons = Array.from(fixture.nativeElement.querySelectorAll('button')) as HTMLButtonElement[];
    const mergeButton = buttons.find((b) => b.textContent?.trim() === 'Merge')!;
    mergeButton.click();

    expect(mergeSpy).toHaveBeenCalled();
  });

  it('emits cancel when Keep Separate is clicked', () => {
    const fixture = TestBed.createComponent(ConfirmMergeDialogComponent);
    fixture.componentRef.setInput('open', true);
    const cancelSpy = vi.fn();
    fixture.componentInstance.cancel.subscribe(cancelSpy);
    fixture.detectChanges();

    const buttons = Array.from(fixture.nativeElement.querySelectorAll('button')) as HTMLButtonElement[];
    const cancelButton = buttons.find((b) => b.textContent?.trim() === 'Keep Separate')!;
    cancelButton.click();

    expect(cancelSpy).toHaveBeenCalled();
  });
});
