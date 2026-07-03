import { ConfirmRemoveFolderDialogComponent } from './confirm-remove-folder-dialog.component';
import { TestBed } from '@angular/core/testing';

describe('ConfirmRemoveFolderDialogComponent', () => {
  it('should create', () => {
    TestBed.configureTestingModule({ imports: [ConfirmRemoveFolderDialogComponent] });
    const fixture = TestBed.createComponent(ConfirmRemoveFolderDialogComponent);
    expect(fixture.componentInstance).toBeTruthy();
  });
});
