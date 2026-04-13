import { Component, OnInit, OnDestroy } from '@angular/core';
import { UnlistenFn } from '@tauri-apps/api/event';
import {
  TauriService,
  IndexingStatus,
  EmbeddingProgress,
} from '../../services/tauri.service';

@Component({
  selector: 'app-embedding-status',
  standalone: true,
  imports: [],
  templateUrl: './embedding-status.component.html',
  styleUrl: './embedding-status.component.css',
})
export class EmbeddingStatusComponent implements OnInit, OnDestroy {
  status: IndexingStatus | null = null;
  progress: EmbeddingProgress | null = null;
  isRunning = false;
  sidecarReady = false;
  private unlistenProgress: UnlistenFn | null = null;
  private unlistenComplete: UnlistenFn | null = null;

  constructor(private tauri: TauriService) {}

  async ngOnInit(): Promise<void> {
    this.unlistenProgress = await this.tauri.onEmbeddingProgress((p) => {
      this.progress = p;
      this.isRunning = true;
    });
    this.unlistenComplete = await this.tauri.onEmbeddingComplete(() => {
      this.isRunning = false;
      this.progress = null;
      this.loadStatus();
    });
    await this.checkSidecar();
    await this.loadStatus();
  }

  ngOnDestroy(): void {
    this.unlistenProgress?.();
    this.unlistenComplete?.();
  }

  async checkSidecar(): Promise<void> {
    try {
      this.sidecarReady = await this.tauri.sidecarHealth();
    } catch {
      this.sidecarReady = false;
    }
  }

  async startSidecar(): Promise<void> {
    try {
      await this.tauri.startSidecar();
      this.sidecarReady = true;
    } catch (e) {
      console.error('Failed to start sidecar:', e);
    }
  }

  async startEmbedding(): Promise<void> {
    try {
      this.isRunning = true;
      await this.tauri.startEmbeddingJob();
    } catch (e) {
      console.error('Failed to start embedding:', e);
      this.isRunning = false;
    }
  }

  async loadStatus(): Promise<void> {
    try {
      this.status = await this.tauri.getIndexingStatus();
    } catch (e) {
      console.error('Failed to load status:', e);
    }
  }

  get progressPercent(): number {
    if (!this.progress) return 0;
    return Math.round((this.progress.current / this.progress.total) * 100);
  }
}
