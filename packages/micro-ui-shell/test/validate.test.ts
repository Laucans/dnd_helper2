import { describe, expect, it } from 'vitest';
import { ProtocolError } from '../src/errors';
import {
  parseCommandResult,
  parseDataVersionEvent,
  parseLookupAnswer,
  parseMessage,
  parseReadAnswer,
  parseSubmitAnswer,
} from '../src/validate';
import { entry, result } from './fakes';

describe('command result (contract H)', () => {
  it('accepts exactly the five fields', () => {
    expect(parseCommandResult(result('applied'))).toEqual(result('applied'));
    expect(parseCommandResult(result('rejected', { violations: ['pc-level-range'] })).violations).toEqual(['pc-level-range']);
  });

  it('refuses an unknown status', () => {
    expect(() => parseCommandResult(result('queued'))).toThrow(ProtocolError);
    expect(() => parseCommandResult(result('done'))).toThrow(ProtocolError);
  });

  it('refuses a sixth field and a missing one, and never reads them as applied', () => {
    expect(() => parseCommandResult({ ...result('applied'), extra: 1 })).toThrow(ProtocolError);
    expect(() => parseCommandResult({ status: 'applied' })).toThrow(ProtocolError);
    expect(() => parseCommandResult(null)).toThrow(ProtocolError);
  });

  it.each([-1, 1.5, '3'])('refuses dataVersion %j', (dataVersion) => {
    expect(() => parseCommandResult(result('applied', { dataVersion }))).toThrow(ProtocolError);
  });
});

describe('message (contract L)', () => {
  const ok = { message: 'ValueDeclaredAhead', to: ['gm'], actions: ['confirm_overwrite', 'cancel', 'edit'] };

  it('keeps a valid message and its extra fields', () => {
    expect(parseMessage({ ...ok, somethingNew: 7 })).toEqual({ ...ok, somethingNew: 7 });
  });

  it('drops (null, no throw) a message without `message`, with an unknown one, or with an empty `to`', () => {
    expect(parseMessage({ to: ['gm'] })).toBeNull();
    expect(parseMessage({ message: 'FromTheFuture', to: ['gm'] })).toBeNull();
    expect(parseMessage({ message: 'Parked', to: [] })).toBeNull();
    expect(parseMessage({ message: 'Parked' })).toBeNull();
    expect(parseMessage('nope')).toBeNull();
  });

  it('ignores an action it does not know', () => {
    expect(parseMessage({ ...ok, actions: ['cancel', 'teleport'] })?.actions).toEqual(['cancel']);
  });

  it('skips a bad message inside an answer instead of failing it', () => {
    const answer = parseLookupAnswer({ entry: entry('queued'), messages: [{ nope: 1 }, { message: 'CommandQueued', to: ['gm'] }] });
    expect('messages' in answer && answer.messages).toHaveLength(1);
  });
});

describe('answers', () => {
  it('a lookup answer is exactly one of entry / result', () => {
    expect(() => parseLookupAnswer({})).toThrow(ProtocolError);
    expect(() => parseLookupAnswer({ entry: entry('queued'), messages: [], result: result('applied') })).toThrow(ProtocolError);
    expect(() => parseLookupAnswer({ entry: { ...entry('queued'), state: 'weird' }, messages: [] })).toThrow(ProtocolError);
    expect(() => parseLookupAnswer({ entry: { ...entry('queued'), surprise: 1 }, messages: [] })).toThrow(ProtocolError);
  });

  it('a submit answer carries commandId, partition, replayed and warnings', () => {
    expect(() => parseSubmitAnswer({ result: result('applied') })).toThrow(ProtocolError);
    const a = parseSubmitAnswer({ commandId: 'c', partition: 'PJ/x', replayed: true, warnings: [], result: result('applied') });
    expect(a.replayed).toBe(true);
  });

  it('a read answer needs data and an integer asOf when present', () => {
    expect(parseReadAnswer({ data: [] , asOf: 3 })).toEqual({ data: [], asOf: 3 });
    expect(parseReadAnswer({ data: { a: 1 } }).asOf).toBeNull();
    expect(() => parseReadAnswer({ asOf: 3 })).toThrow(ProtocolError);
    expect(() => parseReadAnswer({ data: 1, asOf: 'x' })).toThrow(ProtocolError);
  });
});

describe('dataVersion event', () => {
  it('parses a version and drops anything else', () => {
    expect(parseDataVersionEvent('{"dataVersion":0}')).toBe(0);
    expect(parseDataVersionEvent('{"dataVersion":12}')).toBe(12);
    for (const bad of ['not json', '{"dataVersion":-1}', '{"dataVersion":1.5}', '{"dataVersion":"3"}', '{}', '[]', 'null', '']) {
      expect(parseDataVersionEvent(bad)).toBeNull();
    }
  });
});
