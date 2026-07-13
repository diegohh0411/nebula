import { Component, ChangeDetectionStrategy, Input } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { BrnPopoverImports } from '@spartan-ng/brain/popover';
import { HlmPopoverImports } from '@spartan-ng/helm/popover';
import { HlmButton } from '@spartan-ng/helm/button';
import { ImageCollection } from '../../composables/image-collection.composable';
import { SortDirection, SortKeyId } from '../../utils/image-ordering';

@Component({
  selector: 'app-grid-controls',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [LucideAngularModule, BrnPopoverImports, HlmPopoverImports, HlmButton],
  templateUrl: './grid-controls.component.html',
})
export class GridControlsComponent {
  @Input({ required: true }) collection!: ImageCollection;

  protected selectSort(id: SortKeyId): void {
    this.collection.sort.update((s) => ({ ...s, key: id }));
  }

  protected setDirection(direction: SortDirection): void {
    this.collection.sort.update((s) => ({ ...s, direction }));
  }

  /**
   * yyyy-mm-dd (from an <input type="date">) → epoch seconds at a local-time
   * day boundary, or null. Local time matches how the gallery groups days.
   * `bound: 'start'` snaps to 00:00:00, `'end'` to 23:59:59 so the range is
   * inclusive of the whole selected day.
   */
  private toEpoch(value: string, bound: 'start' | 'end'): number | null {
    if (!value) return null;
    const [y, m, d] = value.split('-').map(Number);
    if (!y || !m || !d) return null;
    const date =
      bound === 'start'
        ? new Date(y, m - 1, d, 0, 0, 0, 0)
        : new Date(y, m - 1, d, 23, 59, 59, 999);
    return Math.floor(date.getTime() / 1000);
  }

  /** epoch seconds → yyyy-mm-dd (local) for binding back into <input type="date">. */
  private toInputValue(epoch: number | null): string {
    if (epoch == null) return '';
    const d = new Date(epoch * 1000);
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${y}-${m}-${day}`;
  }

  protected fromInput(): string {
    return this.toInputValue(this.collection.dateRange().from);
  }

  protected toInput(): string {
    return this.toInputValue(this.collection.dateRange().to);
  }

  protected setFrom(value: string): void {
    this.collection.dateRange.update((r) => ({ ...r, from: this.toEpoch(value, 'start') }));
  }

  protected setTo(value: string): void {
    this.collection.dateRange.update((r) => ({ ...r, to: this.toEpoch(value, 'end') }));
  }

  protected clearRange(): void {
    this.collection.dateRange.set({ from: null, to: null });
  }
}
