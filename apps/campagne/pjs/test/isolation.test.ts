import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const pkg = new URL('../', import.meta.url);
const sources = readdirSync(new URL('src/', pkg)).filter((f) => f.endsWith('.ts'));
const read = (f: string): string => readFileSync(new URL(`src/${f}`, pkg), 'utf8');

describe('the Micro-UI reaches the Data by identifier only', () => {
  it('has source files', () => {
    expect(sources.length).toBeGreaterThan(5);
  });

  it.each(sources)('%s imports the shell or a sibling file, nothing else', (file) => {
    const specifiers = [...read(file).matchAll(/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g)].map((m) => m[1] ?? '');
    for (const s of specifiers) {
      expect(s, `${file}: ${s}`).not.toMatch(/crates\/|\.wasm|\/pkg\/|packages\/|\.\.\//);
      expect(s, `${file}: ${s}`).toMatch(/^(@dnd-helper\/micro-ui-shell|\.\/[a-z-]+)$/);
    }
  });

  it.each(sources)('%s reads no environment, holds no secret, logs and stores nothing', (file) => {
    const text = read(file);
    expect(text).not.toMatch(/process\.env|import\.meta\.env|DATABASE_URL/);
    expect(text).not.toMatch(/\bconsole\./);
    expect(text).not.toMatch(/localStorage|sessionStorage|indexedDB/);
  });

  it.each(sources)('%s writes no HTML from data', (file) => {
    expect(read(file)).not.toMatch(/innerHTML|outerHTML|insertAdjacentHTML|document\.write/);
  });

  it('holds no SQL and no free-form query', () => {
    expect(sources.map(read).join('\n')).not.toMatch(/graphql|\bsql\b|\bquery\s*[:?]|select\s+.+\s+from/i);
  });

  it('names a Capability or DataCapability in identifiers.ts only', () => {
    for (const file of sources.filter((f) => f !== 'identifiers.ts')) {
      expect(read(file), file).not.toMatch(/['"`]campagne\./);
    }
  });

  it('has no runtime dependency but the shell', () => {
    const manifest = JSON.parse(readFileSync(new URL('package.json', pkg), 'utf8')) as { dependencies?: Record<string, string> };
    expect(Object.keys(manifest.dependencies ?? {})).toEqual(['@dnd-helper/micro-ui-shell']);
  });

  it('never speaks of a refusal for concurrency, in code or in copy', () => {
    const word = ['con', 'flict'].join('');
    const other = ['concur', 'rency'].join('');
    expect(sources.map(read).join('\n')).not.toMatch(new RegExp(`${word}|${other}`, 'i'));
  });

  it('keeps no business rule: no trim, no length or range check, no sort, no filter of the rows', () => {
    const code = sources.filter((f) => f !== 'copy.ts').map(read).join('\n');
    expect(code).not.toMatch(/\.trim\(|\.normalize\(|\.sort\(|\.toSorted\(|\.toLowerCase\(|Math\.(min|max|round|floor|ceil)\(/);
    expect(code).not.toMatch(/\.filter\(\s*\(?\s*r\b|rows\.filter/);
  });
});
