import { describe, expect, it } from 'vitest';
import { mapViolations } from '../src/violations';
import type { CommandResult } from '../src/contracts';
import { result } from './fakes';

const rejected = (violations: string[], reviewId: string | null = null): CommandResult =>
  result('rejected', { violations, reviewId }) as unknown as CommandResult;

const MAP = {
  'pc-name-required': { field: 'name', message: 'Le nom est requis.' },
  'pc-name-unique-in-campaign': { field: 'name', message: 'Ce nom existe déjà.' },
  'pc-level-range': { field: 'level', message: 'Le niveau va de 1 à 20.' },
};

describe('mapViolations (rules 37–41)', () => {
  it('maps every id, keeps server order, and keeps two ids on one field', () => {
    const out = mapViolations(rejected(['pc-level-range', 'pc-name-required', 'pc-name-unique-in-campaign']), MAP);
    expect(out.map((v) => v.id)).toEqual(['pc-level-range', 'pc-name-required', 'pc-name-unique-in-campaign']);
    expect(out.map((v) => v.field)).toEqual(['level', 'name', 'name']);
    expect(out.every((v) => v.mapped)).toBe(true);
  });

  it('gives an unknown id a generic message that carries the id, without throwing', () => {
    const [v] = mapViolations(rejected(['brand-new-rule']), MAP);
    expect(v).toEqual({ id: 'brand-new-rule', field: null, message: 'rejected: brand-new-rule', mapped: false });
  });

  it('does not read inherited properties as ids', () => {
    const [v] = mapViolations(rejected(['constructor']), MAP);
    expect(v?.mapped).toBe(false);
  });

  it('works with an empty map', () => {
    expect(mapViolations(rejected(['pc-level-range']), {})[0]?.mapped).toBe(false);
  });

  it('a rejection without ids (human review) yields one generic entry', () => {
    expect(mapViolations(rejected([], 'review-1'), MAP)).toEqual([{ id: null, field: null, message: 'rejected', mapped: false }]);
  });

  it('the caller owns the generic copy', () => {
    const [v] = mapViolations(rejected(['x']), MAP, (id) => `Refusé (${String(id)})`);
    expect(v?.message).toBe('Refusé (x)');
  });

  it('yields nothing for a result that is not rejected, and never edits the result', () => {
    const applied = result('applied') as unknown as CommandResult;
    expect(mapViolations(applied, MAP)).toEqual([]);
    const r = rejected(['pc-level-range']);
    const before = structuredClone(r);
    mapViolations(r, MAP);
    expect(r).toEqual(before);
    expect(Object.keys(r)).toHaveLength(5);
  });
});
