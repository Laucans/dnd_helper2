// The boundary of the unit (rules 4, 5, 15, 27): it reads through the host shell, by identifier,
// and nothing else. Read from the source text, like the shell's own isolation test.

import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const dir = new URL('../src/', import.meta.url);
// The checks are on code: a comment that says "never innerHTML" is not a use of it.
const withoutComments = (text: string): string => text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
const sources = readdirSync(dir)
  .filter((name) => name.endsWith('.ts'))
  .map((name) => ({ name, text: withoutComments(readFileSync(new URL(name, dir), 'utf8')) }));

// `from '…'`, a side-effect `import '…'` and a dynamic `import('…')`.
const importsOf = (text: string): string[] => [...text.matchAll(/\b(?:from|import)\s*\(?\s*'([^']+)'/g)].map((m) => m[1]!);

describe('the unit stays inside its boundary', () => {
  it('finds its sources', () => {
    expect(sources.length).toBeGreaterThan(0);
  });

  it('imports only its own files and the shell', () => {
    for (const { name, text } of sources) {
      for (const specifier of importsOf(text)) {
        expect(specifier.startsWith('./') || specifier === '@dnd-helper/micro-ui-shell', `${name} imports ${specifier}`).toBe(true);
      }
    }
  });

  it.each([
    ['a database variable or the environment', /DATABASE_URL|process\.env/],
    ['a transport of its own (the host provides one)', /\bfetch\s*\(|new\s+EventSource\b|XMLHttpRequest|WebSocket/],
    ['a storage', /localStorage|sessionStorage|indexedDB|document\.cookie/],
    ['a write: no command, no mutation', /\.submit\s*\(|\.act\s*\(|\.awaitResult\s*\(/],
    ['a query or a database client', /\b(?:select|insert|update|delete)\s+\w+\s+(?:from|into|set)\b|graphql|postgres|sqlx/i],
    ['a dynamic import or a require', /\bimport\s*\(|\brequire\s*\(/],
    ['the console', /\bconsole\./],
    ['innerHTML', /innerHTML|outerHTML|insertAdjacentHTML/],
  ])('contains no %s', (_what, pattern) => {
    for (const { name, text } of sources) {
      expect(pattern.test(text), `${name} matches ${String(pattern)}`).toBe(false);
    }
  });

  it('calls the host through watch only', () => {
    const calls = sources.flatMap(({ text }) => [...text.matchAll(/\b(?:this\.host|deps\.shell|shell)\.(\w+)/g)].map((m) => m[1]));
    expect(new Set(calls)).toEqual(new Set(['watch']));
  });
});
