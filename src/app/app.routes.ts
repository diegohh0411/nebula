import { Routes } from "@angular/router";
import { GalleryComponent } from "./components/gallery/gallery.component";
import { PeopleViewComponent } from "./components/people-view/people-view.component";
import { TagsViewComponent } from "./components/tags-view/tags-view.component";
import { SubjectDetailComponent } from "./components/subject-detail/subject-detail.component";
import { FacePickerComponent } from "./components/face-picker/face-picker.component";
import { SettingsComponent } from "./components/settings/settings.component";

export const routes: Routes = [
  { path: "", component: GalleryComponent },
  { path: "people", component: PeopleViewComponent },
  { path: "tags", component: TagsViewComponent },
  { path: "subject/:id", component: SubjectDetailComponent },
  { path: "subject/:id/face-picker", component: FacePickerComponent },
  { path: "settings", component: SettingsComponent },
];
