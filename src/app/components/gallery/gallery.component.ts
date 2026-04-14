import {
  Component,
  ChangeDetectionStrategy,
  inject,
} from '@angular/core';
import { PhotoService } from '../../services/photo.service';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { VirtualRow } from '../../models/models';

@Component({
  selector: 'app-gallery',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PhotoGridComponent],
  templateUrl: './gallery.component.html',
  styleUrl: './gallery.component.css',
})
export class GalleryComponent {
  protected photos = inject(PhotoService);

  protected trackRow(_idx: number, row: VirtualRow): string {
    if (row.type === 'header') return `header-${row.date}`;
    const first = row.images[0];
    const id = first ? ('id' in first ? first.id : first.image_id) : _idx;
    return `row-${id}`;
  }
}
