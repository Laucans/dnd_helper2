// A source scan: the Micro-UI stays a Micro-UI (SPEC rules 5, 10, 12, 42).

import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const pkg = new URL('../', import.meta.url);
const files = readdirSync(new URL('src/', pkg)).filter((f) => f.endsWith('.ts'));
const read = (f: string): string => readFileSync(new URL(`src/${f}`, pkg), 'utf8');
const all = files.map(read).join('\n');

describe('the Micro-UI stays a Micro-UI', () => {
  it('has source files', () => {
    expect(files).toEqual(expect.arrayContaining(['controller.ts', 'view.ts', 'identifiers.ts', 'violations.ts', 'index.ts']));
  });

  it.each(files)('%s imports only the shell and its own modules', (file) => {
    const specifiers = [...read(file).matchAll(/(?:from|import)\s*\(?\s*['"]([^'"]+)['"]/g)].map((m) => m[1] ?? '');
    for (const s of specifiers) {
      expect(s, `${file}: ${s}`).not.toMatch(/crates\/|packages\/|apps\/|pkg\/|\.wasm/);
      expect(s, `${file}: ${s}`).toMatch(/^(@dnd-helper\/micro-ui-shell|\.\/[a-z-]+)$/);
    }
  });

  it('writes no HTML: names are plain text (rule 10)', () => {
    expect(all).not.toMatch(/innerHTML|outerHTML|insertAdjacentHTML/);
  });

  it('reads no environment, logs nothing, stores nothing, opens no connection of its own', () => {
    expect(all).not.toMatch(/process\.env|DATABASE_URL/);
    expect(all).not.toMatch(/console\./);
    expect(all).not.toMatch(/localStorage|sessionStorage|indexedDB/);
    expect(all).not.toMatch(/\bfetch\s*\(|new EventSource|XMLHttpRequest|WebSocket/);
  });

  it('names exactly three identifiers, none of a PC or the party level (rule 42)', () => {
    const found = [...all.matchAll(/['"]([a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*)['"]/g)].map((m) => m[1]);
    expect([...new Set(found)].sort()).toEqual(['campagne.archiverCampagne', 'campagne.creerCampagne', 'campagne.listerCampagnes']);
    for (const f of files.filter((f) => f !== 'identifiers.ts')) {
      expect(read(f), f).not.toMatch(/['"]campagne\.[A-Za-z]+['"]/);
    }
    expect(all).not.toMatch(/niveauDuGroupe|listerPjs|modifierPJ|ajouterPJ|archiverPJ/i);
  });

  it('runs no name rule: no trim, normalisation, length or required check (rule 12)', () => {
    for (const file of ['controller.ts', 'view.ts']) {
      const text = read(file);
      expect(text, file).not.toMatch(/\.trim(?:Start|End)?\(|\.normalize\(|maxLength|minLength|(?<!fieldErrors\.)\bname\.length|\.required\b/);
      expect(text, file).not.toMatch(/['"](?:required|maxlength|minlength|pattern)['"]/i);
    }
  });

  it('never sorts, filters or reorders the rows it was given (rule 6)', () => {
    for (const file of ['controller.ts', 'view.ts']) {
      expect(read(file), file).not.toMatch(/\.sort\(|\.toSorted\(|\.reverse\(|\.toReversed\(|localeCompare/);
    }
    expect(read('controller.ts')).not.toMatch(/rows\.filter|data\.filter/);
  });

  it('has no concurrency wording: a queued command is a state, not an error (rule 25)', () => {
    expect(all).not.toMatch(/conflict(?!-)|concurren/i);
  });

  it('calls no delete and never acts on a command (rules 20, 28)', () => {
    expect(all).not.toMatch(/\.act\(|confirm_overwrite|effect:\s*['"]delete/);
  });

  it('depends on the shell alone', () => {
    const manifest = JSON.parse(readFileSync(new URL('package.json', pkg), 'utf8')) as { dependencies?: Record<string, string>; peerDependencies?: unknown };
    expect(Object.keys(manifest.dependencies ?? {})).toEqual(['@dnd-helper/micro-ui-shell']);
    expect(manifest).not.toHaveProperty('peerDependencies');
  });
});
