import { Component, OnInit, inject, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SavedReport, TagWithCount, Folder } from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';
import { FormsModule } from '@angular/forms';

@Component({
  selector: 'app-reports',
  standalone: true,
  imports: [RouterLink, LucideAngularModule, FormsModule],
  templateUrl: './reports.component.html',
  styleUrl: './reports.component.css',
})
export class ReportsComponent implements OnInit {
  protected photos = inject(PhotoService);
  private router = inject(Router);

  protected reports = signal<SavedReport[]>([]);
  protected tags = signal<TagWithCount[]>([]);
  
  protected isCreating = signal(false);
  protected newName = signal('');
  protected newFolderId = signal<number | null>(null);
  protected newTagId = signal<number | null>(null); // Simplified single tag for now

  async ngOnInit() {
    await this.loadData();
  }

  async loadData() {
    const [reps, tgs] = await Promise.all([
      this.photos.listSavedReports(),
      this.photos.listTags()
    ]);
    this.reports.set(reps);
    this.tags.set(tgs);
  }

  protected async deleteReport(id: number, e: Event) {
    e.stopPropagation();
    await this.photos.deleteSavedReport(id);
    await this.loadData();
  }

  protected getFolderName(id: number): string {
    const folder = this.photos.folders().find(f => f.id === id);
    if (!folder) return 'Unknown Folder';
    return folder.path.replace(/\\/g, '/').split('/').filter(Boolean).pop() ?? folder.path;
  }

  protected getTagsDesc(tagIds: number[]): string {
    const allTags = this.tags();
    return tagIds.map(id => allTags.find(t => t.id === id)?.name ?? 'Unknown').join(', ');
  }

  protected async createReport() {
    const fId = this.newFolderId();
    const tId = this.newTagId();
    const name = this.newName().trim();
    if (!fId || !tId || !name) return;

    const rep = await this.photos.createSavedReport(name, fId, [tId]);
    this.isCreating.set(false);
    this.newName.set('');
    void this.router.navigate(['/reports', rep.id]);
  }
}
