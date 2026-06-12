import { Component, OnInit, inject, signal, ChangeDetectionStrategy } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterLink } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { TagWithCount, SubjectMatch } from '../../models/models';
import { EditableTextComponent } from '../editable-text/editable-text.component';
import { SubjectPersonCardComponent } from '../subject-person-card/subject-person-card.component';
import { HlmInput } from '@spartan-ng/helm/input';

@Component({
  selector: 'app-tags-view',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, RouterLink, EditableTextComponent, SubjectPersonCardComponent, HlmInput],
  templateUrl: './tags-view.component.html',
  styleUrl: './tags-view.component.css',
})
export class TagsViewComponent implements OnInit {
  protected photos = inject(PhotoService);

  protected tags = signal<TagWithCount[]>([]);
  protected selectedTag = signal<TagWithCount | null>(null);
  protected tagSubjects = signal<SubjectMatch[]>([]);
  protected newTagName = signal('');
  protected createError = signal<string | null>(null);
  protected renameError = signal<string | null>(null);

  ngOnInit() {
    void this.loadTags();
  }

  private async loadTags(): Promise<void> {
    try {
      this.tags.set(await this.photos.listTags());
    } catch { /* ignore */ }
  }

  protected async selectTag(tag: TagWithCount): Promise<void> {
    this.selectedTag.set(tag);
    this.renameError.set(null);
    try {
      this.tagSubjects.set(await this.photos.getTagSubjects(tag.id));
    } catch { /* ignore */ }
  }

  protected async createTag(): Promise<void> {
    const name = this.newTagName().trim();
    if (!name) return;
    try {
      this.createError.set(null);
      await this.photos.createTag(name);
      this.newTagName.set('');
      await this.loadTags();
    } catch (e: unknown) {
      this.createError.set(typeof e === 'string' ? e : 'Failed to create tag');
    }
  }

  protected async renameTag(tag: TagWithCount, newName: string): Promise<void> {
    if (!newName.trim()) return;
    try {
      this.renameError.set(null);
      await this.photos.renameTag(tag.id, newName);
      await this.loadTags();
      const updated = this.tags().find((t) => t.id === tag.id);
      if (updated && this.selectedTag()?.id === tag.id) {
        this.selectedTag.set(updated);
      }
    } catch (e: unknown) {
      this.renameError.set(typeof e === 'string' ? e : 'Name already exists');
    }
  }

  protected async deleteTag(tag: TagWithCount): Promise<void> {
    if (!confirm(`Delete tag "${tag.name}"? Subjects are not affected.`)) return;
    try {
      await this.photos.deleteTag(tag.id);
      if (this.selectedTag()?.id === tag.id) {
        this.selectedTag.set(null);
        this.tagSubjects.set([]);
      }
      await this.loadTags();
    } catch { /* ignore */ }
  }

  protected async removeSubjectFromTag(subjectId: number): Promise<void> {
    const tag = this.selectedTag();
    if (!tag) return;
    try {
      await this.photos.removeSubjectTag(subjectId, tag.id);
      this.tagSubjects.update((ss) => ss.filter((s) => s.subject.id !== subjectId));
      await this.loadTags();
    } catch { /* ignore */ }
  }
}
