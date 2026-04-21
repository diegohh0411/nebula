import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { invoke } from '@tauri-apps/api/core';
import { LucideAngularModule } from 'lucide-angular';
import {
  HlmCard,
  HlmCardHeader,
  HlmCardTitle,
  HlmCardDescription,
  HlmCardContent,
  HlmCardFooter,
} from '../../libs/ui/card/src';
import { HlmButtonDirective } from '../../libs/ui/button/src';

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
    HlmButtonDirective,
  ],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.css'
})
export class SettingsComponent implements OnInit {
  models = signal<ModelInfo[]>([]);
  currentModel = signal<string | null>(null);

  isConfirming = signal(false);
  pendingModelId = signal<string | null>(null);
  confirmInputValue = signal('');
  isProcessing = signal(false);

  async ngOnInit() {
    await this.loadModels();
    await this.loadSettings();
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
      this.currentModel.set(model || 'diegohh/siglip2-base-patch16-224'); // Default fallback
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
      try {
        await invoke('update_setting', { key: 'embedding_model', value: modelId });
        this.currentModel.set(modelId);
        this.isConfirming.set(false);
        this.pendingModelId.set(null);
      } catch (e) {
        console.error('Failed to update model:', e);
        // Error handling could be improved with a toast
      } finally {
        this.isProcessing.set(false);
      }
    }
  }
}
