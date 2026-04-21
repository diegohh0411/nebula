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

  isConfirming = signal(false);
  pendingModelId = signal<string | null>(null);
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
  }

  async loadSettings() {
    try {
      const model = await invoke<string | null>('get_setting', { key: 'embedding_model' });
      this.currentModel.set(model || 'diegohh/siglip2-base-patch16-224');
    } catch (e) {
      console.error('Failed to load settings:', e);
    }
  }

  async selectModel(modelId: string) {
    if (modelId === this.currentModel() || this.isProcessing()) return;
    
    this.pendingModelId.set(modelId);
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
    if (modelId && this.confirmInputValue() === 'REINDEX' && !this.isProcessing()) {
      this.isProcessing.set(true);
      this.processingPhase.set('downloading');
      this.downloadProgress.set(0);
      try {
        await invoke('update_setting', { key: 'embedding_model', value: modelId });
        this.currentModel.set(modelId);
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
