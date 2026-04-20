import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { invoke } from '@tauri-apps/api/core';
import { LucideAngularModule } from 'lucide-angular';

interface ModelInfo {
  id: string;
  name: string;
  description: string;
}

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [CommonModule, LucideAngularModule],
  templateUrl: './settings.component.html',
  styleUrl: './settings.component.css'
})
export class SettingsComponent implements OnInit {
  models = signal<ModelInfo[]>([]);
  currentModel = signal<string | null>(null);

  async ngOnInit() {
    await this.loadModels();
    await this.loadSettings();
  }

  async loadModels() {
    const availableModels = await invoke<ModelInfo[]>('get_available_models');
    this.models.set(availableModels);
  }

  async loadSettings() {
    const model = await invoke<string | null>('get_setting', { key: 'embedding_model' });
    this.currentModel.set(model || 'diegohh/siglip2-base-patch16-224'); // Default fallback
  }

  async selectModel(modelId: string) {
    // For now we just update the DB. Task 5 will handle the re-index dialog.
    await invoke('update_setting', { key: 'embedding_model', value: modelId });
    this.currentModel.set(modelId);
  }
}
