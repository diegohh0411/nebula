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
  OnInit,
} from '@angular/core';
import { Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { LightboxComponent } from '../lightbox/lightbox.component';
import { SearchBarComponent } from '../search-bar/search-bar.component';
import { TimelineScrubberComponent } from '../timeline-scrubber/timeline-scrubber.component';
import { SubjectPersonCardComponent } from '../subject-person-card/subject-person-card.component';
import { VirtualRow } from '../../models/models';
import { ScrollingModule, CdkVirtualScrollViewport } from '@angular/cdk/scrolling';
import { CdkAutoSizeVirtualScroll } from '@angular/cdk-experimental/scrolling';

@Component({
  selector: 'app-gallery',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    PhotoGridComponent,
    LightboxComponent,
    TimelineScrubberComponent,
    SearchBarComponent,
    SubjectPersonCardComponent,
    ScrollingModule,
    CdkAutoSizeVirtualScroll,
  ],
  templateUrl: './gallery.component.html',
  styleUrl: './gallery.component.css',
})
export class GalleryComponent implements OnInit, AfterViewInit, OnDestroy {
  protected photos = inject(PhotoService);
  private router = inject(Router);
  private elementRef = inject(ElementRef);
  private resizeObserver?: ResizeObserver;

  protected viewport = viewChild(CdkVirtualScrollViewport);

  protected isLassoing = signal(false);
  protected lassoRect = signal<{top: number, left: number, width: number, height: number} | null>(null);
  private lassoStart = {x: 0, y: 0};
  private cachedCells: NodeListOf<Element> | null = null;
  private selectionRafId: number | null = null;

  ngOnInit() {
    // Ensure images are loaded when we navigate to gallery
    void this.photos.refreshImages();
  }

  onPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    if (target.closest('button') || target.closest('.photo-cell')) return;
    
    target.setPointerCapture(event.pointerId);
    this.isLassoing.set(true);
    this.lassoStart = {x: event.clientX, y: event.clientY};
    this.lassoRect.set({top: event.clientY, left: event.clientX, width: 0, height: 0});
    this.photos.clearSelection();

    // Cache the cell list at the start of lasso
    const viewportEl = this.elementRef.nativeElement.querySelector('.gallery-viewport');
    this.cachedCells = (viewportEl || document).querySelectorAll('.photo-cell');
  }

  @HostListener('window:pointermove', ['$event'])
  onPointerMove(event: PointerEvent) {
    if (!this.isLassoing()) return;

    const left = Math.min(this.lassoStart.x, event.clientX);
    const top = Math.min(this.lassoStart.y, event.clientY);
    const width = Math.abs(this.lassoStart.x - event.clientX);
    const height = Math.abs(this.lassoStart.y - event.clientY);

    this.lassoRect.set({top, left, width, height});

    if (this.selectionRafId === null) {
      this.selectionRafId = requestAnimationFrame(() => {
        this.updateSelection();
        this.selectionRafId = null;
      });
    }
  }

  @HostListener('window:pointerup', ['$event'])
  @HostListener('window:pointercancel', ['$event'])
  onPointerUp(event: PointerEvent) {
    if (!this.isLassoing()) return;
    const target = event.target as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) {
      target.releasePointerCapture(event.pointerId);
    }
    
    this.isLassoing.set(false);
    this.lassoRect.set(null);
    this.cachedCells = null;
    if (this.selectionRafId !== null) {
      cancelAnimationFrame(this.selectionRafId);
      this.selectionRafId = null;
    }
  }

  private updateSelection() {
    const rect = this.lassoRect();
    if (!rect || !this.cachedCells) return;

    const selectedIds: number[] = [];
    
    this.cachedCells.forEach(el => {
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
    if (row.type === 'people') return 'people-row';
    if (row.type === 'header') return `header-${row.date}`;
    const first = row.images[0];
    const id = first ? ('id' in first ? first.id : first.image_id) : _idx;
    return `row-${id}`;
  }

  scrollToDate(date: string) {
    const rows = this.photos.virtualRows();
    const idx = rows.findIndex(r => r.type === 'header' && r.date === date);
    if (idx !== -1) {
      this.viewport()?.scrollToIndex(idx, 'smooth');
    }
  }
}
