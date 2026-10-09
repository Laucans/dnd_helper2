import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const pkg = new URL('../', import.meta.url);
const sources = readdirSync(new URL('src/', pkg)).filter((f) => f.endsWith('.ts'));
const read = (f: string): string => readFileSync(new URL(`src/${f}`, pkg), 'utf8');

describe('the shell stays a shell (rules 3–7, 53)', () => {
  it('has source files', () => {
    expect(sources.length).toBeGreaterThan(5);
  });

  it.each(sources)('%s imports nothing from crates, apps, wasm or another package', (file) => {
    const specifiers = [...read(file).matchAll(/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g)].map((m) => m[1] ?? '');
    for (const s of specifiers) {
      expect(s, `${file}: ${s}`).not.toMatch(/crates\/|apps\/|\.wasm|\/pkg\/|^@dnd-helper\//);
      expect(s, `${file}: ${s}`).toMatch(/^\.\/[a-z-]+$/);
    }
  });

  it.each(sources)('%s reads no environment, logs nothing, stores nothing', (file) => {
    const text = read(file);
    expect(text).not.toMatch(/process\.env|import\.meta\.env|DATABASE_URL/);
    expect(text).not.toMatch(/\bconsole\./);
    expect(text).not.toMatch(/localStorage|sessionStorage|indexedDB/);
  });

  it('sends no free-form query: no query-text parameter anywhere', () => {
    expect(sources.map(read).join('\n')).not.toMatch(/\bquery\s*[:?]|graphql|sql\b/i);
  });

  it('has no runtime dependency', () => {
    const manifest = JSON.parse(readFileSync(new URL('package.json', pkg), 'utf8')) as Record<string, unknown>;
    expect(manifest).not.toHaveProperty('dependencies');
    expect(manifest).not.toHaveProperty('peerDependencies');
  });

  it('there is no micro-ui.json in the package, and no conflict vocabulary', () => {
    expect(readdirSync(pkg)).not.toContain('micro-ui.json');
    expect(sources.map(read).join('\n')).not.toMatch(/conflict(?!-)|ConcurrencyError/i);
  });
});
