import {
  Search, Info, X, ChevronLeft, ChevronRight, ChevronDown, ArrowLeft, Pencil, Star,
  EllipsisVertical, Plus, Settings, Cpu, AlertTriangle, Sparkles, ScanFace, HardDrive,
  Download, Folder, Image, Images, Tag, Users,
} from 'lucide-angular';

/**
 * Every Lucide icon referenced by a `<lucide-icon name="…">` in any template must be
 * registered here — lucide-angular throws at render time for unregistered names, which
 * blanks out the surrounding view (this caused the sidebar to render invisible items).
 * `app-icons.spec.ts` scans the templates and fails if any used icon is missing here.
 *
 * Kept in its own leaf module (no app imports) so the spec can load it without dragging
 * in the component graph.
 */
export const APP_ICONS = {
  Search, Info, X, ChevronLeft, ChevronRight, ChevronDown, ArrowLeft, Pencil, Star,
  EllipsisVertical, Plus, Settings, Cpu, AlertTriangle, Sparkles, ScanFace, HardDrive,
  Download, Folder, Image, Images, Tag, Users,
};
