import {
  Component,
  ChangeDetectionStrategy,
  inject,
  ElementRef,
  AfterViewInit,
  OnDestroy,
  HostListener,
  viewChild,
  signal,
} from '@angular/core';
import { PhotoService } from '../../services/photo.service';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { LightboxComponent } from '../lightbox/lightbox.component';
import { TimelineScrubberComponent } from '../timeline-scrubber/timeline-scrubber.component';
import { VirtualRow } from '../../models/models';
import { ScrollingModule, CdkVirtualScrollViewport } from '@angular/cdk/scrolling';

@Component({
  selector: 'app-gallery',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PhotoGridComponent, LightboxComponent, TimelineScrubberComponent, ScrollingModule],
  templateUrl: './gallery.component.html',
  styleUrl: './gallery.component.css',
})
export class GalleryComponent implements AfterViewInit, OnDestroy {
  protected photos = inject(PhotoService);
  private elementRef = inject(ElementRef);
  private resizeObserver?: ResizeObserver;

  protected viewport = viewChild(CdkVirtualScrollViewport);

  protected isLassoing = signal(false);
  protected lassoRect = signal<{top: number, left: number, width: number, height: number} | null>(null);
  private lassoStart = {x: 0, y: 0};

  onPointerDown(event: PointerEvent) {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return;
    
    this.isLassoing.set(true);
    this.lassoStart = {x: event.clientX, y: event.clientY};
    this.lassoRect.set({top: event.clientY, left: event.clientX, width: 0, height: 0});
    this.photos.clearSelection();
  }

  @HostListener('window:pointermove', ['$event'])
  onPointerMove(event: PointerEvent) {
    if (!this.isLassoing()) return;

    const left = Math.min(this.lassoStart.x, event.clientX);
    const top = Math.min(this.lassoStart.y, event.clientY);
    const width = Math.abs(this.lassoStart.x - event.clientX);
    const height = Math.abs(this.lassoStart.y - event.clientY);

    this.lassoRect.set({top, left, width, height});
    this.updateSelection();
  }

  @HostListener('window:pointerup')
  onPointerUp() {
    this.isLassoing.set(false);
    this.lassoRect.set(null);
  }

  private updateSelection() {
    const rect = this.lassoRect();
    if (!rect) return;

    const elements = document.querySelectorAll('.photo-cell');
    const selectedIds: number[] = [];
    
    elements.forEach(el => {
      const elRect = el.getBoundingClientRect();
      const isIntersecting = !(
        elRect.right < rect.left ||
        elRect.left > rect.left + rect.width ||
        elRect.bottom < rect.top ||
        elRect.top > rect.top + rect.height
      );

      if (isIntersecting) {
        const id = Number(el.getAttribute('data-id'));
        if (!isNaN(id)) selectedIds.push(id);
      }
    });

    this.photos.setSelection(selectedIds);
  }

  ngAfterViewInit() {
    this.resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const width = entry.contentRect.width;
        if (width > 0) {
          this.photos.viewportWidth.set(width);
        }
      }
    });
    this.resizeObserver.observe(this.elementRef.nativeElement);
  }

  ngOnDestroy() {
    this.resizeObserver?.disconnect();
  }

  protected trackRow(_idx: number, row: VirtualRow): string {
    if (row.type === 'header') return `header-${row.date}`;
    const first = row.images[0];
    const id = first ? ('id' in first ? first.id : first.image_id) : _idx;
    return `row-${id}`;
  }

  protected getRowHeight(row: VirtualRow): number {
    if (row.type === 'header') return 48; // Standard header height
    return row.rowHeight;
  }

  scrollToDate(date: string) {
    const rows = this.photos.virtualRows();
    const idx = rows.findIndex(r => r.type === 'header' && r.date === date);
    if (idx !== -1) {
      this.viewport()?.scrollToIndex(idx, 'smooth');
    }
  }
}
