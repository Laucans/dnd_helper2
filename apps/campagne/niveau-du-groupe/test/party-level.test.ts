import { describe, expect, it } from 'vitest';
import { parsePartyLevel } from '../src/party-level';

const ok = (level: number | null, pcCount: number): Record<string, unknown> => ({ level, pcCount, model: 'v1', asOf: 3 });

describe('parsePartyLevel (rules 12, 13)', () => {
  it.each([
    ['two PCs', ok(4, 2)],
    ['no PC', ok(null, 0)],
    ['one PC at the lower bound', ok(1, 1)],
    ['twenty PCs at the upper bound', ok(20, 20)],
    ['no cap on the count', ok(7, 1_000)],
  ])('accepts %s and returns it as received', (_name, value) => {
    expect(parsePartyLevel(value)).toEqual(value);
  });

  it.each([
    ['level 0', ok(0, 1)],
    ['level 21', ok(21, 1)],
    ['a fractional level', ok(4.5, 2)],
    ['a numeric-string level', { ...ok(4, 2), level: '4' }],
    ['a null level with PCs', ok(null, 1)],
    ['a level with no PC', ok(3, 0)],
    ['a negative count', ok(null, -1)],
    ['a fractional count', ok(3, 1.5)],
    ['a numeric-string count', { ...ok(3, 1), pcCount: '1' }],
    ['another model', { ...ok(3, 1), model: 'v2' }],
    ['a missing asOf', { level: 3, pcCount: 1, model: 'v1' }],
    ['a negative asOf', { ...ok(3, 1), asOf: -1 }],
    ['a missing level key (missing is not null)', { pcCount: 0, model: 'v1', asOf: 1 }],
    ['an undefined level', { ...ok(3, 1), level: undefined }],
    ['null', null],
    ['an array', [ok(3, 1)]],
    ['a string', 'level'],
  ])('rejects %s', (_name, value) => {
    expect(parsePartyLevel(value)).toBeNull();
  });
});
