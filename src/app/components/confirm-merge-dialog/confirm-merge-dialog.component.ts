import { Component, ChangeDetectionStrategy, input, output } from '@angular/core';

@Component({
  selector: 'app-confirm-merge-dialog',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './confirm-merge-dialog.component.html',
})
export class ConfirmMergeDialogComponent {
  readonly open = input(false);
  readonly merge = output<void>();
  readonly cancel = output<void>();
}
