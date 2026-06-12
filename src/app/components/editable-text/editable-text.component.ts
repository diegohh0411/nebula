import {
  Component,
  Input,
  Output,
  EventEmitter,
  signal,
  ViewChild,
  ElementRef,
  afterNextRender,
  inject,
  Injector,
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
      this.isEditing.set(true);
      this.focusInput();
    }
  }

  /** Emits the trimmed value on blur or Enter, including "" for explicit removals. */
  @Output() commit = new EventEmitter<string>();

  /** Emits when Tab is pressed inside the input (after committing), so the parent can
   *  chain focus to the next editable field. */
  @Output() tabbed = new EventEmitter<void>();

  protected isEditing = signal(false);
  protected draft = signal('');
  private injector = inject(Injector);

  @ViewChild('inputEl') private inputRef?: ElementRef<HTMLInputElement>;

  protected startEdit(): void {
    this.draft.set(this.value ?? '');
    this.isEditing.set(true);
    this.focusInput();
  }

  private focusInput(): void {
    afterNextRender(
      () => {
        const el = this.inputRef?.nativeElement;
        el?.focus();
        el?.select();
      },
      { injector: this.injector },
    );
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
