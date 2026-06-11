import {
  Component,
  Input,
  Output,
  EventEmitter,
  signal,
  ViewChild,
  ElementRef,
  afterNextRender,
} from '@angular/core';

@Component({
  selector: 'app-editable-text',
  standalone: true,
  templateUrl: './editable-text.component.html',
})
export class EditableTextComponent {
  @Input() value: string | null = null;
  @Input() placeholder = '';
  @Input() placeholderClass = '';
  @Input() displayClass = '';

  /** Setting to true triggers edit mode externally (e.g. Tab chaining). Re-setting to true
   *  while already editing is a no-op. */
  @Input() set startEditing(trigger: boolean) {
    if (trigger && !this.isEditing()) {
      this.draft.set(this.value ?? '');
      this._focusPending.set(true);
      this.isEditing.set(true);
    }
  }

  /** Emits the trimmed value on blur or Enter, including "" for explicit removals. */
  @Output() commit = new EventEmitter<string>();

  /** Emits when Tab is pressed inside the input (after committing), so the parent can
   *  chain focus to the next editable field. */
  @Output() tabbed = new EventEmitter<void>();

  protected isEditing = signal(false);
  protected draft = signal('');
  private _focusPending = signal(false);

  @ViewChild('inputEl') private inputRef?: ElementRef<HTMLInputElement>;

  constructor() {
    afterNextRender({
      read: () => {
        if (this._focusPending()) {
          this._focusPending.set(false);
          this.inputRef?.nativeElement.focus();
        }
      },
    });
  }

  protected startEdit(): void {
    this.draft.set(this.value ?? '');
    this._focusPending.set(true);
    this.isEditing.set(true);
  }

  protected onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      this.doCommit();
    } else if (event.key === 'Escape') {
      this.isEditing.set(false);
    } else if (event.key === 'Tab') {
      event.preventDefault();
      this.doCommit();
      this.tabbed.emit();
    }
  }

  protected onBlur(): void {
    this.doCommit();
  }

  private doCommit(): void {
    if (!this.isEditing()) return;
    this.isEditing.set(false);
    this.commit.emit(this.draft().trim());
  }
}
