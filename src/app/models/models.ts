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
  date_file: number;
  thumbnail_path: string | null;
  embed_status: 'pending' | 'done' | 'failed';
  added_at: number;
  updated_at: number;
  deleted_at: number | null;
}

export interface SearchResult {
  image_id: number;
  path: string;
  thumbnail_path: string | null;
  score: number;
  date_taken: number | null;
  date_file: number;
  embed_status: 'pending' | 'done' | 'failed';
}

export interface EmbedStatus {
  pending: number;
  done: number;
}

export interface EmbedProgressEvent {
  pending: number;
  done: number;
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

export interface Face {
  id: number;
  image_id: number;
  subject_id: number | null;
  bbox_x: number;
  bbox_y: number;
  bbox_w: number;
  bbox_h: number;
  added_at: number;
}

export interface SubjectDetail {
  subject: Subject;
  photo_count: number;
  face_count: number;
}

/** A virtual scroll row: either a day header or a row of images */
export type VirtualRow =
  | { type: 'header'; label: string; date: string }
  | { type: 'row'; images: (Image | SearchResult)[]; rowHeight: number };
