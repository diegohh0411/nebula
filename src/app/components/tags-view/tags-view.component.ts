import { Component, OnInit, inject, signal, ChangeDetectionStrategy, DestroyRef } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { CommonModule } from '@angular/common';
import { ActivatedRoute, Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { TagWithCount, SubjectMatch } from '../../models/models';
import { EditableTextComponent } from '../editable-text/editable-text.component';
import { SubjectPersonCardComponent } from '../subject-person-card/subject-person-card.component';
import { HlmInput } from '@spartan-ng/helm/input';
import { PageHeaderComponent } from '../page-header/page-header.component';

@Component({
  selector: 'app-tags-view',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, EditableTextComponent, SubjectPersonCardComponent, HlmInput, PageHeaderComponent],
  templateUrl: './tags-view.component.html',
  styleUrl: './tags-view.component.css',
})
export class TagsViewComponent implements OnInit {
  protected photos = inject(PhotoService);
  private route = inject(ActivatedRoute);
  private router = inject(Router);
  private destroyRef = inject(DestroyRef);

  protected tags = signal<TagWithCount[]>([]);
  protected selectedTag = signal<TagWithCount | null>(null);
  protected tagSubjects = signal<SubjectMatch[]>([]);
  protected newTagName = signal('');
  protected createError = signal<string | null>(null);
  protected renameError = signal<string | null>(null);

  ngOnInit() {
    void this.loadTags().then(() => {
      this.route.queryParamMap
        .pipe(takeUntilDestroyed(this.destroyRef))
        .subscribe((params) => {
          void this.applyTagFromUrl(params.get('tag'));
        });
    });
  }

  private async loadTags(): Promise<void> {
    try {
      this.tags.set(await this.photos.listTags());
    } catch { /* ignore */ }
  }

  private async applyTagFromUrl(tagIdParam: string | null): Promise<void> {
    const tagId = tagIdParam ? Number(tagIdParam) : null;
    const tag = tagId !== null ? this.tags().find((t) => t.id === tagId) ?? null : null;
    this.selectedTag.set(tag);
    this.renameError.set(null);
    if (!tag) {
      this.tagSubjects.set([]);
      return;
    }
    try {
      this.tagSubjects.set(await this.photos.getTagSubjects(tag.id));
    } catch { /* ignore */ }
  }

  protected selectTag(tag: TagWithCount): void {
    void this.router.navigate([], {
      relativeTo: this.route,
      queryParams: { tag: tag.id },
      queryParamsHandling: 'merge',
      replaceUrl: true,
    });
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
      await this.loadTags();
      if (this.selectedTag()?.id === tag.id) {
        void this.router.navigate([], {
          relativeTo: this.route,
          queryParams: { tag: null },
          queryParamsHandling: 'merge',
          replaceUrl: true,
        });
      }
    } catch { /* ignore */ }
  }

  protected async onTagAdded(): Promise<void> {
    await this.loadTags();
  }

  protected async onTagRemoved(subjectId: number, tagId: number): Promise<void> {
    if (this.selectedTag()?.id === tagId) {
      this.tagSubjects.update((ss) => ss.filter((s) => s.subject.id !== subjectId));
    }
    await this.loadTags();
  }

  protected async onMerged(): Promise<void> {
    await this.loadTags();
    const tag = this.selectedTag();
    if (!tag) return;
    try {
      this.tagSubjects.set(await this.photos.getTagSubjects(tag.id));
    } catch { /* ignore */ }
  }
}
