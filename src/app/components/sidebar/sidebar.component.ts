import {
  Component,
  ChangeDetectionStrategy,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { open } from '@tauri-apps/plugin-dialog';
import { PhotoService } from '../../services/photo.service';

@Component({
  selector: 'app-sidebar',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule],
  templateUrl: './sidebar.component.html',
  styleUrl: './sidebar.component.css',
})
export class SidebarComponent {
  protected photos = inject(PhotoService);
  protected apiKeyDraft = signal('');

  protected folderBasename(path: string): string {
    return path.replace(/\\/g, '/').split('/').filter(Boolean).pop() ?? path;
  }

  protected async addFolder(): Promise<void> {
    const selected = await open({ directory: true, multiple: false });
    if (selected && typeof selected === 'string') {
      await this.photos.addFolder(selected);
    }
  }

  protected async removeFolder(id: number, event: MouseEvent): Promise<void> {
    event.stopPropagation();
    await this.photos.removeFolder(id);
  }

  protected selectFolder(id: number | null): void {
    this.photos.currentView.set('gallery');
    this.photos.selectFolder(id);
  }

  protected toggleApiKeyInput(): void {
    this.photos.showApiKeyInput.update((v) => !v);
    const key = this.photos.apiKey();
    this.apiKeyDraft.set(key ?? '');
  }

  protected async saveApiKey(): Promise<void> {
    const key = this.apiKeyDraft().trim();
    if (key) {
      await this.photos.saveApiKey(key);
    }
    this.photos.showApiKeyInput.set(false);
  }

  protected async regenerateThumbnails(): Promise<void> {
    if (confirm('Regenerate all thumbnails? This will clear existing ones and recreate them at higher resolution.')) {
      await this.photos.regenerateThumbnails();
    }
  }
}
