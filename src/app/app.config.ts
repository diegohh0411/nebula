import { ApplicationConfig, provideZoneChangeDetection, importProvidersFrom } from '@angular/core';
import { provideRouter } from '@angular/router';
import { routes } from './app.routes';
import { LucideAngularModule, Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus, Settings, Cpu, AlertTriangle } from 'lucide-angular';

export const appConfig: ApplicationConfig = {
  providers: [
    provideZoneChangeDetection({ eventCoalescing: true }),
    provideRouter(routes),
    importProvidersFrom(LucideAngularModule.pick({ Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus, Settings, Cpu, AlertTriangle })),
  ],
};
