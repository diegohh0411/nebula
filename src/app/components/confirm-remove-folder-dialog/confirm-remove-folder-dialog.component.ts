import { Component, ChangeDetectionStrategy, input, output } from '@angular/core';

@Component({
  selector: 'app-confirm-remove-folder-dialog',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './confirm-remove-folder-dialog.component.html',
})
export class ConfirmRemoveFolderDialogComponent {
  readonly open = input(false);
  readonly folderName = input<string>('');
  readonly confirm = output<void>();
  readonly cancel = output<void>();
}
