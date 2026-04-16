import {
  Component,
  ChangeDetectionStrategy,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { PhotoService } from '../../services/photo.service';

@Component({
  selector: 'app-search-bar',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule],
  templateUrl: './search-bar.component.html',
  styleUrl: './search-bar.component.css',
})
export class SearchBarComponent {
  protected photos = inject(PhotoService);
  protected query = signal('');

  protected onSearch(): void {
    void this.photos.searchByText(this.query());
  }

  protected onClear(): void {
    this.query.set('');
    this.photos.clearSearch();
  }
}
