import {
  Component,
  OnInit,
  OnDestroy,
  inject,
  ChangeDetectionStrategy,
} from '@angular/core';
import { MediaMatcher } from '@angular/cdk/layout';
import { SidebarComponent } from './components/sidebar/sidebar.component';
import { RouterOutlet } from '@angular/router';
import { PhotoService } from './services/photo.service';
import { TauriEventsService } from './services/tauri-events.service';

@Component({
  selector: 'app-root',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SidebarComponent, RouterOutlet],
  template: `
    <div class="flex h-screen bg-background text-foreground overflow-hidden">
      <app-sidebar class="flex-shrink-0" />
      <div class="flex flex-col flex-1 min-w-0 h-full">
        <router-outlet />
      </div>
    </div>
  `,
  styles: [
    `
      :host {
        display: block;
        height: 100vh;
      }
    `,
  ],
})
export class AppComponent implements OnInit, OnDestroy {
  private media = inject(MediaMatcher);
  protected photos = inject(PhotoService);
  // Inject events service early so its constructor wires up listeners
  private _events = inject(TauriEventsService);

  private darkQuery = this.media.matchMedia('(prefers-color-scheme: dark)');
  private themeListener = (e: MediaQueryListEvent) => this.applyTheme(e.matches);

  ngOnInit(): void {
    this.applyTheme(this.darkQuery.matches);
    this.darkQuery.addEventListener('change', this.themeListener);

    // Bootstrap data
    void this.photos.loadFolders();
    void this.photos.refreshImages();
    void this.photos.refreshProcessingStatus();
  }

  ngOnDestroy(): void {
    this.darkQuery.removeEventListener('change', this.themeListener);
  }

  private applyTheme(isDark: boolean): void {
    document.documentElement.classList.toggle('dark', isDark);
  }
}
