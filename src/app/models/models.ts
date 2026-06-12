export interface Folder {
  id: number;
  path: string;
  added_at: number;
  photo_count: number;
}

export interface Image {
  id: number;
  folder_id: number;
  path: string;
  file_hash: string;
  date_taken: number | null;
  mtime: number;
  thumbnail_path: string | null;
  preview_path: string | null;
  semantic_analysis_done: boolean;
  subject_analysis_done: boolean;
  added_at: number;
  updated_at: number;
  deleted_at: number | null;
}

export interface SearchResult {
  image_id: number;
  path: string;
  thumbnail_path: string | null;
  preview_path: string | null;
  score: number;
  date_taken: number | null;
  mtime: number;
  semantic_analysis_done: boolean;
  subject_analysis_done: boolean;
}

export type ProcessingStage = 'pending' | 'ready';

export function getProcessingStage(
  img: Pick<Image | SearchResult, 'semantic_analysis_done' | 'subject_analysis_done'>
): ProcessingStage {
  if (!img.semantic_analysis_done || !img.subject_analysis_done) return 'pending';
  return 'ready';
}

export interface PipelineStats {
  total_pending: number;
  images_per_sec: number;
}

export function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '';
  if (seconds < 60) return `~${Math.round(seconds)}s left`;
  
  const totalMinutes = Math.round(seconds / 60);
  if (totalMinutes < 60) return `~${totalMinutes} min left`;
  
  const h = Math.floor(totalMinutes / 60);
  const m = totalMinutes % 60;
  return m > 0 ? `~${h}h ${m}m left` : `~${h}h left`;
}

export interface ImageAddedEvent {
  image_id: number;
  path: string;
}

export interface ImageUpdatedEvent {
  image_id: number;
}

export interface ImageRemovedEvent {
  path: string;
}

/** A day group for display in the gallery */
export interface DayGroup {
  label: string;   // "Today", "Yesterday", or "April 12, 2026"
  date: string;    // ISO date "2026-04-12" for sorting
  images: (Image | SearchResult)[];
}

export interface Subject {
  id: number;
  name: string | null;
  thumbnail_face_id: number | null;
  type: string;
  added_at: number;
}

export interface Tag {
  id: number;
  name: string;
  added_at: number;
}

export interface TagWithCount extends Tag {
  subject_count: number;
}

export interface SubjectMatch {
  subject: Subject;
  tags: Tag[];
}

export interface Face {
  id: number;
  image_id: number;
  subject_id: number | null;
  bbox_x: number;
  bbox_y: number;
  bbox_w: number;
  bbox_h: number;
  added_at: number;
  is_manual: boolean;
}

export interface SubjectDetail {
  subject: Subject;
  photo_count: number;
  face_count: number;
}

export interface MergeSuggestion {
  id: number;
  subject_a: Subject;
  subject_b: Subject;
  score: number;
}

export interface NameSubjectResult {
  duplicate_subject_id: number | null;
}

/** A virtual scroll row: either a people strip, a day header, or a row of images */
export type VirtualRow =
  | { type: 'people'; matches: SubjectMatch[] }
  | { type: 'header'; label: string; date: string; collapsed: boolean; count: number }
  | { type: 'row'; images: (Image | SearchResult)[]; rowHeight: number };

export interface ModelDownloadEvent {
  file: string;
  bytes_done: number;
  bytes_total: number | null;
  done: boolean;
  error: string | null;
}
