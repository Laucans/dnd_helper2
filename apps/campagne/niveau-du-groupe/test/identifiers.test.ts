import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { NIVEAU_DU_GROUPE } from '../src/identifiers';

// Manifests are data: they are read, never imported.
const read = (path: string): Record<string, unknown> => JSON.parse(readFileSync(new URL(path, import.meta.url), 'utf8')) as Record<string, unknown>;

const manifest = read('../micro-ui.json');
const capability = read('../../../../crates/campagne/capabilities/niveau-du-groupe/capability.json');

describe('micro-ui.json (rules 1–4)', () => {
  it('needs exactly the Capability the code reads, as the Capability declares it', () => {
    expect(manifest['needs']).toEqual([NIVEAU_DU_GROUPE]);
    expect(capability['capability']).toBe(NIVEAU_DU_GROUPE);
  });

  it('is of the same system as that Capability', () => {
    expect(manifest['system']).toBe('campagne');
    expect(capability['system']).toBe('campagne');
  });

  it('is NiveauDuGroupe, takes the one prop campagneId, and has a description', () => {
    expect(manifest['microUi']).toBe('NiveauDuGroupe');
    expect(manifest['props']).toEqual({ campagneId: 'ID' });
    expect(capability['input']).toEqual({ campagneId: 'ID' });
    expect(typeof manifest['description']).toBe('string');
    expect(manifest['description']).not.toBe('');
  });

  it('declares no action', () => {
    const actions = manifest['actions'];
    expect(actions === undefined || (Array.isArray(actions) && actions.length === 0)).toBe(true);
  });
});
