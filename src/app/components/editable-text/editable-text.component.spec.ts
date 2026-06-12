import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { EditableTextComponent } from './editable-text.component';

describe('EditableTextComponent', () => {
  let component: EditableTextComponent;
  let fixture: ComponentFixture<EditableTextComponent>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [EditableTextComponent],
    }).compileComponents();

    fixture = TestBed.createComponent(EditableTextComponent);
    component = fixture.componentInstance;
  });

  // ── Display state ────────────────────────────────────────────────────────────

  it('displays the value as a clickable span when value is set', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    const span = fixture.debugElement.query(By.css('span[role="button"]'));
    expect(span).not.toBeNull();
    expect(span.nativeElement.textContent.trim()).toBe('Alice');
    expect(fixture.debugElement.query(By.css('input'))).toBeNull();
  });

  it('displays the placeholder when value is null', () => {
    component.value = null;
    component.placeholder = '+ Add a name';
    fixture.detectChanges();

    const span = fixture.debugElement.query(By.css('span[role="button"]'));
    expect(span).not.toBeNull();
    expect(span.nativeElement.textContent.trim()).toBe('+ Add a name');
    expect(fixture.debugElement.query(By.css('input'))).toBeNull();
  });

  it('displays the placeholder when value is empty string', () => {
    component.value = '';
    component.placeholder = '+ Add a name';
    fixture.detectChanges();

    const span = fixture.debugElement.query(By.css('span[role="button"]'));
    expect(span.nativeElement.textContent.trim()).toBe('+ Add a name');
  });

  // ── Entering edit mode ───────────────────────────────────────────────────────

  it('clicking the value span enters edit mode and shows the input', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    const span = fixture.debugElement.query(By.css('span[role="button"]'));
    span.nativeElement.click();
    fixture.detectChanges();

    expect(fixture.debugElement.query(By.css('input'))).not.toBeNull();
    expect(fixture.debugElement.query(By.css('span[role="button"]'))).toBeNull();
  });

  it('clicking the placeholder span enters edit mode', () => {
    component.value = null;
    component.placeholder = '+ Add a name';
    fixture.detectChanges();

    const span = fixture.debugElement.query(By.css('span[role="button"]'));
    span.nativeElement.click();
    fixture.detectChanges();

    expect(fixture.debugElement.query(By.css('input'))).not.toBeNull();
  });

  it('input is pre-filled with the current value when editing starts', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const input = fixture.debugElement.query(By.css('input'));
    expect(input.nativeElement.value).toBe('Alice');
  });

  it('input is pre-filled with empty string when value is null and editing starts', () => {
    component.value = null;
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const input = fixture.debugElement.query(By.css('input'));
    expect(input.nativeElement.value).toBe('');
  });

  // ── startEditing input setter ────────────────────────────────────────────────

  it('startEditing=true programmatically enters edit mode', () => {
    component.value = 'Bob';
    fixture.detectChanges();

    component.startEditing = true;
    fixture.detectChanges();

    expect(fixture.debugElement.query(By.css('input'))).not.toBeNull();
  });

  it('startEditing=true is a no-op when already editing', () => {
    component.value = 'Bob';
    fixture.detectChanges();

    // Enter edit mode via click
    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();
    expect(component['isEditing']()).toBe(true);

    // Re-setting startEditing should not cause issues (no-op guard)
    component.startEditing = true;
    fixture.detectChanges();

    expect(component['isEditing']()).toBe(true);
    expect(fixture.debugElement.query(By.css('input'))).not.toBeNull();
  });

  it('startEditing=false does not enter edit mode', () => {
    component.value = 'Bob';
    fixture.detectChanges();

    component.startEditing = false;
    fixture.detectChanges();

    expect(fixture.debugElement.query(By.css('input'))).toBeNull();
  });

  // ── Commit on blur ───────────────────────────────────────────────────────────

  it('blur commits the current draft and emits via (commit)', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    // Simulate user typing
    input.value = 'Alice Smith';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new Event('blur'));
    fixture.detectChanges();

    expect(commitSpy).toHaveBeenCalledWith('Alice Smith');
    expect(component['isEditing']()).toBe(false);
  });

  it('blur trims whitespace before emitting', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = '  Bob  ';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new Event('blur'));
    fixture.detectChanges();

    expect(commitSpy).toHaveBeenCalledWith('Bob');
  });

  // ── Commit on Enter ──────────────────────────────────────────────────────────

  it('Enter key commits the draft and emits via (commit)', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = 'Alice Updated';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    fixture.detectChanges();

    expect(commitSpy).toHaveBeenCalledWith('Alice Updated');
    expect(component['isEditing']()).toBe(false);
  });

  // ── Escape cancels ───────────────────────────────────────────────────────────

  it('Escape exits edit mode without emitting (commit)', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = 'Something Else';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    fixture.detectChanges();

    expect(commitSpy).not.toHaveBeenCalled();
    expect(component['isEditing']()).toBe(false);
  });

  it('Escape does not emit even when draft equals original value', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    fixture.debugElement.query(By.css('input')).nativeElement
      .dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    fixture.detectChanges();

    expect(commitSpy).not.toHaveBeenCalled();
  });

  // ── Empty string commit (name removal) ──────────────────────────────────────

  it('committing an empty string emits "" (not null) — caller converts as needed', () => {
    // NOTE: The component emits draft().trim() which is "" for empty input.
    // The component does NOT convert "" to null internally — the parent is responsible
    // for that conversion (e.g., PeopleViewComponent maps "" → null before calling the API).
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = '';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    fixture.detectChanges();

    expect(commitSpy).toHaveBeenCalledWith('');
  });

  it('committing a whitespace-only string emits "" after trim', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = '   ';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new Event('blur'));
    fixture.detectChanges();

    expect(commitSpy).toHaveBeenCalledWith('');
  });

  // ── Tab key handling ─────────────────────────────────────────────────────────

  it('Tab key calls preventDefault, commits the draft, and emits (tabbed)', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    const tabbedSpy = vi.fn();
    component.commit.subscribe(commitSpy);
    component.tabbed.subscribe(tabbedSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = 'Alice Smith';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    const tabEvent = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    input.dispatchEvent(tabEvent);
    fixture.detectChanges();

    expect(tabEvent.defaultPrevented).toBe(true);
    expect(commitSpy).toHaveBeenCalledWith('Alice Smith');
    expect(tabbedSpy).toHaveBeenCalled();
    expect(component['isEditing']()).toBe(false);
  });

  it('Tab emits (tabbed) after exiting edit mode', () => {
    component.value = 'Bob';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const tabbedSpy = vi.fn();
    component.tabbed.subscribe(tabbedSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    fixture.detectChanges();

    expect(tabbedSpy).toHaveBeenCalledTimes(1);
    expect(component['isEditing']()).toBe(false);
  });

  it('Tab on empty draft emits "" via (commit) and then emits (tabbed)', () => {
    component.value = 'Bob';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    const tabbedSpy = vi.fn();
    component.commit.subscribe(commitSpy);
    component.tabbed.subscribe(tabbedSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = '';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    fixture.detectChanges();

    expect(commitSpy).toHaveBeenCalledWith('');
    expect(tabbedSpy).toHaveBeenCalled();
  });

  // ── Focus on edit entry ──────────────────────────────────────────────────────

  it('focuses the input on a single click into edit mode', async () => {
    const fixture = TestBed.createComponent(EditableTextComponent);
    fixture.componentInstance.placeholder = '+ Add a name';
    fixture.detectChanges();

    // First entry into edit mode (single click on the placeholder span).
    const trigger = fixture.nativeElement.querySelector('[role="button"]') as HTMLElement;
    trigger.click();
    fixture.detectChanges();
    await fixture.whenStable();

    const input = fixture.nativeElement.querySelector('input') as HTMLInputElement;
    expect(input).toBeTruthy();
    expect(document.activeElement).toBe(input);
  });

  // ── doCommit is idempotent ───────────────────────────────────────────────────

  it('blur after Enter does not emit a second (commit)', () => {
    component.value = 'Alice';
    fixture.detectChanges();

    fixture.debugElement.query(By.css('span[role="button"]')).nativeElement.click();
    fixture.detectChanges();

    const commitSpy = vi.fn();
    component.commit.subscribe(commitSpy);

    const input = fixture.debugElement.query(By.css('input')).nativeElement;
    input.value = 'Alice Smith';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    fixture.detectChanges();
    // After Enter, isEditing is false. A subsequent blur should be a no-op.
    input.dispatchEvent(new Event('blur'));
    fixture.detectChanges();

    expect(commitSpy).toHaveBeenCalledTimes(1);
  });
});
