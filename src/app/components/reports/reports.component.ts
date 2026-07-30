import { Component, OnInit, inject, signal, computed, HostListener } from '@angular/core';
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
  protected isSubmitting = signal(false);
  protected newName = signal('');
  protected selectedFolderIds = signal<number[]>([]);
  protected isFolderDropdownOpen = signal(false);

  protected isDropdownOpen = signal(false);
  protected tagSearchQuery = signal('');
  protected selectedTagIds = signal<number[]>([]);

  protected filteredTags = computed(() => {
    const query = this.tagSearchQuery().toLowerCase().trim();
    const allTags = this.tags();
    if (!query) return allTags;
    return allTags.filter(t => t.name.toLowerCase().includes(query));
  });

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

  @HostListener('document:click')
  protected closeDropdown() {
    this.isDropdownOpen.set(false);
    this.isFolderDropdownOpen.set(false);
  }

  protected toggleDropdown(event: Event) {
    event.stopPropagation();
    this.isDropdownOpen.update(open => !open);
    this.isFolderDropdownOpen.set(false);
  }

  protected toggleFolderDropdown(event: Event) {
    event.stopPropagation();
    this.isFolderDropdownOpen.update(open => !open);
    this.isDropdownOpen.set(false);
  }

  protected isFolderSelected(folderId: number): boolean {
    return this.selectedFolderIds().includes(folderId);
  }

  protected toggleFolder(folderId: number) {
    this.selectedFolderIds.update(ids => {
      if (ids.includes(folderId)) {
        return ids.filter(id => id !== folderId);
      } else {
        return [...ids, folderId];
      }
    });
  }

  protected getSelectedFoldersText(): string {
    const ids = this.selectedFolderIds();
    if (ids.length === 0) return 'Select Folders...';
    if (ids.length === 1) return this.getFolderName(ids[0]);
    return `${ids.length} folders selected`;
  }

  protected isTagSelected(tagId: number): boolean {
    return this.selectedTagIds().includes(tagId);
  }

  protected toggleTag(tagId: number) {
    this.selectedTagIds.update(ids => {
      if (ids.includes(tagId)) {
        return ids.filter(id => id !== tagId);
      } else {
        return [...ids, tagId];
      }
    });
  }

  protected getSelectedTagsText(): string {
    const ids = this.selectedTagIds();
    const allTags = this.tags();
    if (ids.length === 0) return 'Select Tags...';
    if (ids.length === 1) {
      return allTags.find(t => t.id === ids[0])?.name ?? 'Unknown Tag';
    }
    return `${ids.length} tags selected`;
  }

  protected async deleteReport(id: number, e: Event) {
    e.stopPropagation();
    try {
      await this.photos.deleteSavedReport(id);
      await this.loadData();
    } catch (err) {
      console.error('Failed to delete report:', err);
      alert('Failed to delete report. See console for details.');
    }
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

  protected getFoldersDesc(folderIds: number[]): string {
    return folderIds.map(id => this.getFolderName(id)).join(', ');
  }

  protected async createReport() {
    const fIds = this.selectedFolderIds();
    const tIds = this.selectedTagIds();
    const name = this.newName().trim();
    if (fIds.length === 0 || tIds.length === 0 || !name || this.isSubmitting()) return;

    this.isSubmitting.set(true);
    try {
      const rep = await this.photos.createSavedReport(name, fIds, tIds);
      this.isCreating.set(false);
      this.newName.set('');
      this.selectedFolderIds.set([]);
      this.selectedTagIds.set([]);
      this.tagSearchQuery.set('');
      this.isDropdownOpen.set(false);
      this.isFolderDropdownOpen.set(false);
      void this.router.navigate(['/reports', rep.id]);
    } catch (err) {
      console.error('Failed to create report:', err);
      alert('Failed to create report. See console for details.');
    } finally {
      this.isSubmitting.set(false);
    }
  }

  protected cancelCreation() {
    this.isCreating.set(false);
    this.newName.set('');
    this.selectedFolderIds.set([]);
    this.selectedTagIds.set([]);
    this.tagSearchQuery.set('');
    this.isDropdownOpen.set(false);
    this.isFolderDropdownOpen.set(false);
  }
}
