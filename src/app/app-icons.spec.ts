import { APP_ICONS } from './app-icons';

/** Raw contents of every template in the app, loaded at build time by Vite. */
const templates = import.meta.glob('./**/*.html', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

/** Convert a Lucide kebab-case icon name (e.g. "chevron-down") to its PascalCase export key ("ChevronDown"). */
function toPascal(kebab: string): string {
  return kebab
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('');
}

/** Collect every `<lucide-icon name="…">` literal used across the app's templates. */
function usedIconNames(): string[] {
  const names = new Set<string>();
  const re = /<lucide-icon\b[^>]*\bname="([a-z][a-z0-9-]*)"/g;
  for (const html of Object.values(templates)) {
    for (const m of html.matchAll(re)) names.add(m[1]);
  }
  return [...names];
}

describe('APP_ICONS registration', () => {
  it('registers every Lucide icon referenced in templates', () => {
    const used = usedIconNames();
    // Guard against the regex silently matching nothing (e.g. template syntax changes).
    expect(used.length).toBeGreaterThan(0);

    const missing = used.filter((name) => !(toPascal(name) in APP_ICONS));
    expect(missing, `Unregistered lucide icons (add to APP_ICONS): ${missing.join(', ')}`).toEqual([]);
  });
});
