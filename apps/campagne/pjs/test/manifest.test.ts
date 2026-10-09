import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { AJOUTER_PJ, ARCHIVER_PJ, LISTER_PJS, MODIFIER_PJ } from '../src/identifiers';

// Data only: the manifests are read from disk, no sibling code is imported.
const repo = new URL('../../../../', import.meta.url);
const json = (path: string): Record<string, unknown> => JSON.parse(readFileSync(new URL(path, repo), 'utf8')) as Record<string, unknown>;

const manifest = json('apps/campagne/pjs/micro-ui.json');
const contract = json('contracts/d-micro-ui-manifest.schema.json') as { properties: Record<string, { pattern?: string; items?: { pattern: string } }>; required: string[] };
const dcs = {
  add: json('crates/dataguard/data-capabilities/ajouter-pj/data-capability.json'),
  edit: json('crates/dataguard/data-capabilities/modifier-pj/data-capability.json'),
  archive: json('crates/dataguard/data-capabilities/archiver-pj/data-capability.json'),
};

describe('micro-ui.json', () => {
  it('has the fields contract D requires and no other', () => {
    expect(Object.keys(manifest).sort()).toEqual(Object.keys(contract.properties).sort());
    for (const key of contract.required) expect(manifest).toHaveProperty(key);
  });

  it('follows contract D’s patterns for the id, the system and every identifier', () => {
    const re = (p: string | undefined): RegExp => new RegExp(p ?? '(?!)');
    expect(manifest['microUi']).toMatch(re(contract.properties['microUi']?.pattern));
    expect(manifest['system']).toMatch(re(contract.properties['system']?.pattern));
    for (const id of manifest['needs'] as string[]) expect(id).toMatch(re(contract.properties['needs']?.items?.pattern));
    for (const id of manifest['actions'] as string[]) expect(id).toMatch(re(contract.properties['actions']?.items?.pattern));
  });

  it('is the campagne Pjs Micro-UI with the single prop campagneId', () => {
    expect(manifest['microUi']).toBe('Pjs');
    expect(manifest['system']).toBe('campagne');
    expect(manifest['props']).toEqual({ campagneId: 'ID' });
  });

  it('needs lister-pjs only and acts through the add, edit and archive commands only', () => {
    expect(manifest['needs']).toEqual([LISTER_PJS]);
    expect(manifest['actions']).toEqual([AJOUTER_PJ.dataCapability, MODIFIER_PJ.dataCapability, ARCHIVER_PJ.dataCapability]);
  });
});

describe('the identifiers the code uses are the siblings’', () => {
  it('lister-pjs is a campagne Capability of that exact id', () => {
    const cap = json('crates/campagne/capabilities/lister-pjs/capability.json');
    expect(cap['capability']).toBe(LISTER_PJS);
    expect(cap['system']).toBe('campagne');
  });

  it.each([
    ['add', AJOUTER_PJ],
    ['edit', MODIFIER_PJ],
    ['archive', ARCHIVER_PJ],
  ] as const)('%s: id, version and mode equal the DataCapability’s, and campagne may call it', (key, used) => {
    const dc = dcs[key];
    expect(dc['dataCapability']).toBe(used.dataCapability);
    expect(dc['version']).toBe(used.version);
    expect(dc['mode']).toBe(used.mode);
    const callableBy = dc['callableBy'] as string[];
    expect(callableBy.some((c) => c === 'Pjs' || c === 'campagne' || c === '*')).toBe(true);
  });

  it('the edit and archive commands target the PJ aggregate', () => {
    for (const key of ['edit', 'archive'] as const) {
      expect((dcs[key]['target'] as { aggregate: string }).aggregate).toBe('PJ');
    }
    expect(MODIFIER_PJ.aggregate).toBe('PJ');
    expect(ARCHIVER_PJ.aggregate).toBe('PJ');
  });
});
