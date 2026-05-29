import { Component, OnInit, signal, inject, OnDestroy } from '@angular/core';
import { CommonModule } from '@angular/common';
import { invoke } from '@tauri-apps/api/core';
import { LucideAngularModule } from 'lucide-angular';
import { Subscription } from 'rxjs';
import {
  HlmCard,
  HlmCardHeader,
  HlmCardTitle,
  HlmCardDescription,
  HlmCardContent,
  HlmCardFooter,
} from '../../libs/ui/card/src';
import { HlmButton } from '../../libs/ui/button/src';
import { TauriEventsService } from '../../services/tauri-events.service';
import { ModelDownloadEvent } from '../../models/models';

interface ModelInfo {
  id: string;
  name: string;
  description: string;
  downloaded: boolean;
  size_bytes: number;
}

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [
    CommonModule,
    LucideAngularModule,
    HlmCard,
    HlmCardHeader,
    HlmCardTitle,
    HlmCardDescription,
    HlmCardContent,
    HlmCardFooter,
    HlmButton,
  ],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.css'
})
export class SettingsComponent implements OnInit, OnDestroy {
  private events = inject(TauriEventsService);
  private sub = new Subscription();

  models = signal<ModelInfo[]>([]);
  currentModel = signal<string | null>(null);

  subjectModels = signal<ModelInfo[]>([]);
  currentSubjectModel = signal<string | null>(null);

  isConfirming = signal(false);
  pendingModelId = signal<string | null>(null);
  pendingSection = signal<'vision' | 'subject'>('vision');
  confirmInputValue = signal('');
  isProcessing = signal(false);

  processingPhase = signal<'downloading' | 'reindexing'>('downloading');
  downloadProgress = signal<number | null>(null);
  currentDownloadFile = signal<string | null>(null);

  async ngOnInit() {
    await this.loadModels();
    await this.loadSettings();

    this.sub.add(
      this.events.modelDownloadProgress$.subscribe((ev: ModelDownloadEvent) => {
        if (ev.done) {
          this.processingPhase.set('reindexing');
          this.downloadProgress.set(100);
          return;
        }
        this.processingPhase.set('downloading');
        this.currentDownloadFile.set(ev.file);
        if (ev.bytes_total) {
          this.downloadProgress.set((ev.bytes_done / ev.bytes_total) * 100);
        } else {
          this.downloadProgress.set(null);
        }
      })
    );
  }

  ngOnDestroy() {
    this.sub.unsubscribe();
  }

  async loadModels() {
    try {
      const availableModels = await invoke<ModelInfo[]>('get_available_models');
      this.models.set(availableModels);
    } catch (e) {
      console.error('Failed to load models:', e);
    }
    try {
      const subjectModels = await invoke<ModelInfo[]>('get_available_subject_models');
      this.subjectModels.set(subjectModels);
    } catch (e) {
      console.error('Failed to load subject models:', e);
    }
  }

  async loadSettings() {
    try {
      const model = await invoke<string>('get_setting', { key: 'embedding_model' });
      this.currentModel.set(model);
    } catch (e) {
      console.error('Failed to load embedding_model setting:', e);
    }
    try {
      const subjectModel = await invoke<string>('get_setting', { key: 'subject_model' });
      this.currentSubjectModel.set(subjectModel);
    } catch (e) {
      console.error('Failed to load subject_model setting:', e);
    }
  }

  formatBytes(bytes: number): string {
    if (bytes === 0) return '';
    if (bytes < 1_073_741_824) {
      return `${(bytes / 1_048_576).toFixed(1)} MB`;
    }
    return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
  }

  selectVisionModel(modelId: string) {
    if (modelId === this.currentModel() || this.isProcessing()) return;
    this.pendingModelId.set(modelId);
    this.pendingSection.set('vision');
    this.confirmInputValue.set('');
    this.isConfirming.set(true);
  }

  selectSubjectModel(modelId: string) {
    if (modelId === this.currentSubjectModel() || this.isProcessing()) return;
    this.pendingModelId.set(modelId);
    this.pendingSection.set('subject');
    this.confirmInputValue.set('');
    this.isConfirming.set(true);
  }

  cancelSelection() {
    if (this.isProcessing()) return;
    this.isConfirming.set(false);
    this.pendingModelId.set(null);
  }

  async confirmSelection() {
    const modelId = this.pendingModelId();
    const section = this.pendingSection();
    if (modelId && this.confirmInputValue() === 'REINDEX' && !this.isProcessing()) {
      this.isProcessing.set(true);
      this.processingPhase.set(section === 'vision' ? 'downloading' : 'reindexing');
      this.downloadProgress.set(0);
      try {
        const key = section === 'vision' ? 'embedding_model' : 'subject_model';
        await invoke('update_setting', { key, value: modelId });
        if (section === 'vision') {
          this.currentModel.set(modelId);
        } else {
          this.currentSubjectModel.set(modelId);
        }
        this.isConfirming.set(false);
        this.pendingModelId.set(null);
      } catch (e) {
        console.error('Failed to update model:', e);
      } finally {
        this.isProcessing.set(false);
        this.downloadProgress.set(null);
        this.currentDownloadFile.set(null);
      }
    }
  }
}
