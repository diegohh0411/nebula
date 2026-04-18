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
  protected isDragOver = signal(false);

  protected onSearch(): void {
    void this.photos.searchByText(this.query());
  }

  protected onClear(): void {
    this.query.set('');
    this.photos.clearSearch();
  }

  protected onDragOver(event: DragEvent): void {
    event.preventDefault();
    this.isDragOver.set(true);
  }

  protected onDragLeave(event: DragEvent): void {
    this.isDragOver.set(false);
  }

  protected onDrop(event: DragEvent): void {
    event.preventDefault();
    this.isDragOver.set(false);
    const file = event.dataTransfer?.files[0];
    if (!file || !file.type.startsWith('image/')) return;
    const reader = new FileReader();
    reader.onload = () => {
      const base64 = (reader.result as string).split(',')[1];
      const objectUrl = URL.createObjectURL(file);
      void this.photos.searchByExternalImage(base64, file.type, objectUrl);
    };
    reader.readAsDataURL(file);
  }

  protected onPaste(event: ClipboardEvent): void {
    const items = event.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        const file = item.getAsFile();
        if (!file) continue;
        const reader = new FileReader();
        reader.onload = () => {
          const base64 = (reader.result as string).split(',')[1];
          const objectUrl = URL.createObjectURL(file);
          void this.photos.searchByExternalImage(base64, file.type, objectUrl);
        };
        reader.readAsDataURL(file);
        break;
      }
    }
  }

  protected clearImageSearch(): void {
    this.query.set('');
    this.photos.clearSearch();
  }
}
