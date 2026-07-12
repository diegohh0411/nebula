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

  /** yyyy-mm-dd (from an <input type="date">) → epoch seconds at UTC midnight, or null. */
  private toEpoch(value: string): number | null {
    if (!value) return null;
    const [y, m, d] = value.split('-').map(Number);
    if (!y || !m || !d) return null;
    return Math.floor(Date.UTC(y, m - 1, d) / 1000);
  }

  /** epoch seconds → yyyy-mm-dd for binding back into <input type="date">. */
  private toInputValue(epoch: number | null): string {
    if (epoch == null) return '';
    return new Date(epoch * 1000).toISOString().slice(0, 10);
  }

  protected fromInput(): string {
    return this.toInputValue(this.collection.dateRange().from);
  }

  protected toInput(): string {
    return this.toInputValue(this.collection.dateRange().to);
  }

  protected setFrom(value: string): void {
    this.collection.dateRange.update((r) => ({ ...r, from: this.toEpoch(value) }));
  }

  protected setTo(value: string): void {
    this.collection.dateRange.update((r) => ({ ...r, to: this.toEpoch(value) }));
  }

  protected clearRange(): void {
    this.collection.dateRange.set({ from: null, to: null });
  }
}
