import { Component, OnInit, OnDestroy } from '@angular/core';
import { Subject, takeUntil } from 'rxjs';
import { open } from '@tauri-apps/plugin-dialog';
import { TauriService, Folder, IndexingStatus } from '../../services/tauri.service';

@Component({
  selector: 'app-folder-manager',
  standalone: true,
  imports: [],
  templateUrl: './folder-manager.component.html',
  styleUrl: './folder-manager.component.css',
})
export class FolderManagerComponent implements OnInit, OnDestroy {
  folders: Folder[] = [];
  status: IndexingStatus | null = null;
  loading = false;
  private destroy$ = new Subject<void>();

  constructor(private tauri: TauriService) {}

  ngOnInit(): void {
    this.loadFolders();
    this.loadStatus();
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  async addFolder(): Promise<void> {
    const selected = await open({
      directory: true,
      multiple: false,
      title: 'Select a folder to index',
    });
    if (typeof selected === 'string' && selected) {
      this.loading = true;
      try {
        await this.tauri.addFolder(selected);
        await this.loadFolders();
        await this.loadStatus();
      } catch (e) {
        console.error('Failed to add folder:', e);
      } finally {
        this.loading = false;
      }
    }
  }

  async removeFolder(id: number): Promise<void> {
    try {
      await this.tauri.removeFolder(id);
      await this.loadFolders();
      await this.loadStatus();
    } catch (e) {
      console.error('Failed to remove folder:', e);
    }
  }

  async loadFolders(): Promise<void> {
    try {
      this.folders = await this.tauri.listFolders();
    } catch (e) {
      console.error('Failed to load folders:', e);
    }
  }

  async loadStatus(): Promise<void> {
    try {
      this.status = await this.tauri.getIndexingStatus();
    } catch (e) {
      console.error('Failed to load status:', e);
    }
  }
}
