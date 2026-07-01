import { defineConfig } from 'vite';
import angular from '@analogjs/vite-plugin-angular';

// The `@spartan-ng/helm/*` specifiers below are mapped to local files via
// tsconfig.json's `compilerOptions.paths` for type-checking, but Vite's dev/test
// module graph does not read tsconfig path mappings on its own. Angular's router
// plugin happens to pre-resolve these for components reachable from a lazy route
// (e.g. subject-detail, tags-view), which is why those specs pass without this
// alias — but any other component importing `@spartan-ng/helm/*` (e.g. a card or
// popover that isn't itself route-loaded) fails to resolve at test time without
// it. Mirror the tsconfig paths here so all components resolve consistently.
export default defineConfig({
  plugins: [angular()],
  resolve: {
    alias: {
      '@spartan-ng/helm/popover': new URL('./src/app/libs/ui/popover/src/index.ts', import.meta.url).pathname,
      '@spartan-ng/helm/utils': new URL('./src/app/libs/ui/utils/src/index.ts', import.meta.url).pathname,
      '@spartan-ng/helm/command': new URL('./src/app/libs/ui/command/src/index.ts', import.meta.url).pathname,
      '@spartan-ng/helm/button': new URL('./src/app/libs/ui/button/src/index.ts', import.meta.url).pathname,
      '@spartan-ng/helm/icon': new URL('./src/app/libs/ui/icon/src/index.ts', import.meta.url).pathname,
      '@spartan-ng/helm/card': new URL('./src/app/libs/ui/card/src/index.ts', import.meta.url).pathname,
      '@spartan-ng/helm/input': new URL('./src/app/libs/ui/input/src/index.ts', import.meta.url).pathname,
    },
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['src/test-setup.ts'],
    include: ['src/**/*.spec.ts'],
  },
});
