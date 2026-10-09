import { mapViolations } from '@dnd-helper/micro-ui-shell';
import { describe, expect, it } from 'vitest';
import { fieldOfMessage, levelPayload, pcPayload, PJ_VIOLATIONS } from '../src/violations';

const rejected = (...violations: string[]) =>
  mapViolations({ commandId: 'c', status: 'rejected', dataVersion: null, violations, reviewId: null }, PJ_VIOLATIONS);

describe('PJ_VIOLATIONS', () => {
  it.each([
    ['pc-name-required', 'nom'],
    ['pc-name-unique-in-campaign', 'nom'],
    ['pc-class-required', 'classe'],
    ['pc-level-range', 'niveau'],
  ])('%s marks %s', (id, field) => {
    expect(rejected(id)).toEqual([expect.objectContaining({ id, field, mapped: true })]);
  });

  it.each(['campaign-active', 'pc-active'])('%s marks no input', (id) => {
    expect(rejected(id)).toEqual([expect.objectContaining({ id, field: null, mapped: false })]);
  });

  it('matches by exact id only, never by prefix, and keeps an unknown id verbatim', () => {
    const out = rejected('pc-name-required-x', 'pc-level', 'toString', 'brand-new-rule');
    expect(out.map((v) => v.field)).toEqual([null, null, null, null]);
    expect(out.map((v) => v.id)).toEqual(['pc-name-required-x', 'pc-level', 'toString', 'brand-new-rule']);
  });

  it('has exactly the four ids of the table', () => {
    expect(Object.keys(PJ_VIOLATIONS).sort()).toEqual(['pc-class-required', 'pc-level-range', 'pc-name-required', 'pc-name-unique-in-campaign']);
  });
});

describe('fieldOfMessage', () => {
  it('honours the engine’s <Aggregate>.<field> form and nothing else', () => {
    expect(fieldOfMessage('PJ.name')).toBe('nom');
    expect(fieldOfMessage('PJ.class')).toBe('classe');
    expect(fieldOfMessage('PJ.level')).toBe('niveau');
    expect(fieldOfMessage('PJ.campagneId')).toBeNull();
    expect(fieldOfMessage('name')).toBeNull();
    expect(fieldOfMessage('PJ.names')).toBeNull();
    expect(fieldOfMessage(undefined)).toBeNull();
  });
});

describe('levelPayload', () => {
  it.each([
    ['', undefined],
    ['5', 5],
    ['-3', -3],
    ['0', 0],
    ['21', 21],
    ['007', 7],
    ['5.0', '5.0'],
    ['5.5', '5.5'],
    [' 5', ' 5'],
    ['5 ', '5 '],
    ['abc', 'abc'],
    ['1e2', '1e2'],
    ['+5', '+5'],
    ['99999999999999999999', '99999999999999999999'],
  ])('%j → %j', (raw, expected) => {
    expect(levelPayload(raw)).toBe(expected);
  });
});

describe('pcPayload', () => {
  it('sends text as typed: no trim, no normalisation, no default', () => {
    const decomposed = 'Rémi  ';
    expect(pcPayload({ nom: decomposed, classe: '  ', niveau: '5' })).toEqual({ nom: decomposed, classe: '  ', niveau: 5 });
    expect(pcPayload({ nom: decomposed, classe: '', niveau: '5' })['nom']).toBe(decomposed);
  });

  it('omits the level when empty and sends empty text as empty', () => {
    expect(pcPayload({ nom: '', classe: '', niveau: '' })).toEqual({ nom: '', classe: '' });
    expect(pcPayload({ nom: ' a ', classe: ' b ', niveau: '' })).toEqual({ nom: ' a ', classe: ' b ' });
  });
});
