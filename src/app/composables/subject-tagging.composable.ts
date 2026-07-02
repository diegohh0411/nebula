import { inject, signal, Signal, WritableSignal } from '@angular/core';
import { PhotoService } from '../services/photo.service';
import { Tag, TagWithCount } from '../models/models';

export interface SubjectTaggingApi {
  readonly name: WritableSignal<string | null>;
  readonly tags: WritableSignal<Tag[]>;
  readonly allTags: WritableSignal<TagWithCount[]>;
  readonly newTagName: WritableSignal<string>;
  readonly tagError: WritableSignal<string | null>;
  readonly mergeError: WritableSignal<string | null>;
  readonly nameConflict: WritableSignal<{ subjectId: number } | null>;

  saveName(value: string): Promise<void>;
  confirmMerge(): Promise<void>;
  cancelMerge(): void;
  onTagFocus(): Promise<void>;
  addTag(): Promise<void>;
  removeTag(tagId: number): Promise<void>;
}

export interface SubjectTaggingCallbacks {
  onNameSaved?: (name: string | null) => void;
  onMerged?: (targetId: number) => void;
  onTagAdded?: (tag: Tag) => void;
  onTagRemoved?: (tagId: number) => void;
}

export function injectSubjectTagging(
  subjectId: Signal<number | null>,
  callbacks?: SubjectTaggingCallbacks,
): SubjectTaggingApi {
  const photos = inject(PhotoService);

  const name = signal<string | null>(null);
  const tags = signal<Tag[]>([]);
  const allTags = signal<TagWithCount[]>([]);
  const newTagName = signal('');
  const tagError = signal<string | null>(null);
  const mergeError = signal<string | null>(null);
  const nameConflict = signal<{ subjectId: number } | null>(null);

  async function saveName(value: string): Promise<void> {
    const id = subjectId();
    if (id === null) return;
    const newName = value || null;
    try {
      const result = await photos.nameSubject(id, newName);
      name.set(newName);
      callbacks?.onNameSaved?.(newName);
      if (result.duplicate_subject_id) {
        nameConflict.set({ subjectId: result.duplicate_subject_id });
      }
    } catch (e) {
      console.error('Failed to save name', e);
    }
  }

  async function confirmMerge(): Promise<void> {
    const id = subjectId();
    const conflict = nameConflict();
    if (id === null || conflict === null) return;
    try {
      mergeError.set(null);
      await photos.mergeSubjects(id, conflict.subjectId);
      nameConflict.set(null);
      callbacks?.onMerged?.(id);
    } catch (e: unknown) {
      mergeError.set(typeof e === 'string' ? e : 'Failed to merge subjects');
    }
  }

  function cancelMerge(): void {
    nameConflict.set(null);
    mergeError.set(null);
  }

  async function onTagFocus(): Promise<void> {
    try {
      allTags.set(await photos.listTags());
    } catch { /* ignore — non-critical autocomplete data */ }
  }

  async function addTag(): Promise<void> {
    const id = subjectId();
    const newName = newTagName().trim();
    if (!newName || id === null) return;
    try {
      tagError.set(null);
      const tag = await photos.addSubjectTag(id, newName);
      newTagName.set('');
      tags.update((ts) => [...ts, tag]);
      callbacks?.onTagAdded?.(tag);
    } catch (e: unknown) {
      tagError.set(typeof e === 'string' ? e : 'Failed to add tag');
    }
  }

  async function removeTag(tagId: number): Promise<void> {
    const id = subjectId();
    if (id === null) return;
    try {
      await photos.removeSubjectTag(id, tagId);
      tags.update((ts) => ts.filter((t) => t.id !== tagId));
      callbacks?.onTagRemoved?.(tagId);
    } catch (e: unknown) {
      tagError.set(typeof e === 'string' ? e : 'Failed to remove tag');
    }
  }

  return {
    name,
    tags,
    allTags,
    newTagName,
    tagError,
    mergeError,
    nameConflict,
    saveName,
    confirmMerge,
    cancelMerge,
    onTagFocus,
    addTag,
    removeTag,
  };
}
