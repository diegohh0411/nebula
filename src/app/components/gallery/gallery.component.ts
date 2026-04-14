import {
  Component,
  ChangeDetectionStrategy,
  inject,
  ElementRef,
  AfterViewInit,
  OnDestroy,
  viewChild,
} from '@angular/core';
import { PhotoService } from '../../services/photo.service';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { VirtualRow } from '../../models/models';
import { ScrollingModule, CdkVirtualScrollViewport } from '@angular/cdk/scrolling';

@Component({
  selector: 'app-gallery',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PhotoGridComponent, ScrollingModule],
  templateUrl: './gallery.component.html',
  styleUrl: './gallery.component.css',
})
export class GalleryComponent implements AfterViewInit, OnDestroy {
  protected photos = inject(PhotoService);
  private elementRef = inject(ElementRef);
  private resizeObserver?: ResizeObserver;

  protected viewport = viewChild(CdkVirtualScrollViewport);

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
}
