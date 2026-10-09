// The manifest against contract D and against the counterpart manifests it
// names. The JSON files are read as data, never imported (SPEC rule 5).

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ARCHIVER_CAMPAGNE, CREER_CAMPAGNE, LISTER_CAMPAGNES } from '../src/identifiers';
import { VIOLATIONS } from '../src/violations';

const repo = new URL('../../../../', import.meta.url);
const json = (path: string): Record<string, unknown> => JSON.parse(readFileSync(new URL(path, repo), 'utf8')) as Record<string, unknown>;

const manifest = json('apps/campagne/campagnes/micro-ui.json');
const schema = json('contracts/d-micro-ui-manifest.schema.json');
const lister = json('crates/campagne/capabilities/lister-campagnes/capability.json');
const creer = json('crates/dataguard/data-capabilities/creer-campagne/data-capability.json');
const archiver = json('crates/dataguard/data-capabilities/archiver-campagne/data-capability.json');

describe('micro-ui.json (contract D)', () => {
  it('has exactly the keys contract D allows, and every required one', () => {
    const allowed = Object.keys(schema['properties'] as object);
    expect(Object.keys(manifest).sort()).toEqual([...allowed].sort());
    for (const key of schema['required'] as string[]) expect(manifest).toHaveProperty(key);
  });

  it('matches the patterns of contract D', () => {
    const props = schema['properties'] as Record<string, { pattern?: string; items?: { pattern: string } }>;
    expect(manifest['microUi']).toMatch(new RegExp(props['microUi']!.pattern!));
    expect(manifest['system']).toMatch(new RegExp(props['system']!.pattern!));
    const identifier = new RegExp(props['needs']!.items!.pattern);
    for (const id of [...(manifest['needs'] as string[]), ...(manifest['actions'] as string[])]) expect(id).toMatch(identifier);
  });

  it('names this Micro-UI and its system', () => {
    expect(manifest['microUi']).toBe('Campagnes');
    expect(manifest['system']).toBe('campagne');
    expect(typeof manifest['description']).toBe('string');
    expect((manifest['description'] as string).length).toBeGreaterThan(20);
  });

  it('needs and acts through the three identifiers the code uses, and no other', () => {
    expect(manifest['needs']).toEqual([LISTER_CAMPAGNES]);
    expect(manifest['actions']).toEqual([CREER_CAMPAGNE.id, ARCHIVER_CAMPAGNE.id]);
  });

  it('declares the emitted campaign id as an ID prop and nothing else', () => {
    expect(manifest['props']).toEqual({ campagneId: 'ID' });
  });
});

describe('the counterparts it relies on (SPEC rule 2)', () => {
  it('has a Capability of its own system under the exact identifier', () => {
    expect(lister['capability']).toBe(LISTER_CAMPAGNES);
    expect(lister['system']).toBe(manifest['system']);
  });

  it.each([
    ['creer-campagne', creer, CREER_CAMPAGNE, 'insert'],
    ['archiver-campagne', archiver, ARCHIVER_CAMPAGNE, 'update'],
  ] as const)('%s: identifier, version, mode and aggregate match the constants', (_name, dc, constants, effect) => {
    expect(dc['dataCapability']).toBe(constants.id);
    expect(dc['version']).toBe(constants.version);
    expect(dc['mode']).toBe(constants.mode);
    expect((dc['target'] as { aggregate: string }).aggregate).toBe(constants.aggregate);
    expect(dc['effect']).toBe(effect);
    expect(dc['callableBy']).toContain('campagne');
  });

  it('archive takes a target id and an empty payload, so the UI sends nothing else', () => {
    expect((archiver['target'] as { id?: string }).id).toBeDefined();
    expect(archiver['payload']).toEqual({});
  });

  it('create takes the name alone', () => {
    expect(creer['payload']).toEqual({ name: 'string' });
    expect(creer['idempotencyKey']).toBe('required');
  });

  it('every violation id the UI maps is an invariant of creer-campagne', () => {
    for (const id of Object.keys(VIOLATIONS)) expect(creer['invariants']).toContain(id);
  });
});
