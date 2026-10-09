import { describe, expect, it } from 'vitest';
import { TransportError } from '@dnd-helper/micro-ui-shell';
import { PartyLevelController, type PartyLevelState } from '../src/controller';
import { FakeHost, data, result } from './fake-host';

// No argument: campaign 'A'. An explicit `undefined` stays undefined.
function setup(...args: [] | [string | undefined]) {
  const host = new FakeHost();
  const changes: PartyLevelState[] = [];
  const controller = new PartyLevelController(host, (s) => changes.push(s));
  controller.setCampagneId(args.length === 0 ? 'A' : args[0]);
  return { host, controller, changes };
}

const ready = (level: number | null, pcCount: number, asOf: number, possiblyStale = false): PartyLevelState => ({ kind: 'ready', level, pcCount, asOf, possiblyStale });
const transportError = (): TransportError => new TransportError('http', 500);

describe('reading (rules 7, 9, 10, 11, 14)', () => {
  it('watches the Capability by identifier with the campaign id, and nothing else', () => {
    const { host } = setup('A');
    expect(host.calls).toHaveLength(1);
    expect(host.calls[0]).toMatchObject({ capability: 'campagne.niveauDuGroupe', variables: { campagneId: 'A' } });
  });

  it('is loading until the first answer, never 0 or "no party level"', () => {
    const { controller } = setup();
    expect(controller.state()).toEqual({ kind: 'loading' });
  });

  it('mixed levels: PCs at 3 and 4 show level 4 and 2 PCs, as received', () => {
    const { host, controller } = setup();
    host.emit(0, data(4, 2, 5));
    expect(controller.state()).toEqual(ready(4, 2, 5));
  });

  it('one PC: the level is that PC\'s level and the count is 1', () => {
    const { host, controller } = setup();
    host.emit(0, data(9, 1, 2));
    expect(controller.state()).toEqual(ready(9, 1, 2));
  });

  it.each([1, 20])('keeps the bound %i', (level) => {
    const { host, controller } = setup();
    host.emit(0, data(level, 3, 2));
    expect(controller.state()).toEqual(ready(level, 3, 2));
  });

  it('level null is a valid state: an empty party with a count of 0', () => {
    const { host, controller } = setup();
    host.emit(0, data(null, 0, 2));
    expect(controller.state()).toEqual(ready(null, 0, 2));
  });

  it('no cap on the count', () => {
    const { host, controller } = setup();
    host.emit(0, data(5, 40, 2));
    expect(controller.state()).toMatchObject({ pcCount: 40 });
  });

  it('orders by the envelope asOf, and by the result\'s own when the answer has none', () => {
    const { host, controller } = setup();
    host.emit(0, { kind: 'data', data: result(4, 2, 8), asOf: null });
    expect(controller.state()).toMatchObject({ asOf: 8 });
    host.emit(0, { kind: 'data', data: result(3, 2, 5), asOf: null });
    expect(controller.state()).toMatchObject({ level: 4, asOf: 8 });
  });
});

describe('a dataVersion change (rules 16–20)', () => {
  it('a newer result replaces the value', () => {
    const { host, controller } = setup();
    host.emit(0, data(3, 2, 5));
    host.emit(0, data(4, 3, 6));
    expect(controller.state()).toEqual(ready(4, 3, 6));
  });

  it('archiving the last active PC switches to "no party level"', () => {
    const { host, controller } = setup();
    host.emit(0, data(4, 1, 5));
    host.emit(0, data(null, 0, 6));
    expect(controller.state()).toEqual(ready(null, 0, 6));
  });

  it('responses out of order: a lower asOf than the one shown is ignored', () => {
    const { host, controller, changes } = setup();
    host.emit(0, data(5, 2, 7));
    const before = changes.length;
    host.emit(0, data(2, 1, 5));
    expect(controller.state()).toEqual(ready(5, 2, 7));
    expect(changes).toHaveLength(before);
  });

  it('an announcement that changes nothing notifies nobody, and never passes through another state', () => {
    const { host, controller, changes } = setup();
    host.emit(0, data(4, 2, 5));
    host.emit(0, data(4, 2, 5));
    host.emit(0, data(4, 2, 6));
    expect(changes.map((s) => s.kind)).toEqual(['loading', 'ready']);
    expect(controller.state()).toEqual(ready(4, 2, 6)); // the newer asOf still counts for ordering
    host.emit(0, data(1, 1, 5));
    expect(controller.state()).toMatchObject({ level: 4, asOf: 6 });
  });

  it('does not poll: no watch beyond the one at mount', () => {
    const { host } = setup();
    expect(host.calls).toHaveLength(1);
  });
});

describe('the campaign id (rules 7, 8, 26)', () => {
  it.each([undefined, ''])('%j: no read, nothing shown', (id) => {
    const { host, controller } = setup(id);
    expect(host.calls).toHaveLength(0);
    expect(controller.state()).toEqual({ kind: 'idle' });
  });

  it('a campaign appearing later starts the read', () => {
    const { host, controller } = setup(undefined);
    controller.setCampagneId('A');
    expect(host.calls).toHaveLength(1);
    expect(controller.state()).toEqual({ kind: 'loading' });
  });

  it('a change drops the previous value at once, releases the old watch and reads the new campaign', () => {
    const { host, controller } = setup('A');
    host.emit(0, data(4, 2, 5));
    controller.setCampagneId('B');
    expect(controller.state()).toEqual({ kind: 'loading' });
    expect(host.calls[0]!.released).toBe(true);
    expect(host.calls[1]).toMatchObject({ variables: { campagneId: 'B' }, released: false });
  });

  it('a late answer for the old campaign is discarded', () => {
    const { host, controller } = setup('A');
    controller.setCampagneId('B');
    host.emit(0, data(4, 2, 99));
    host.emit(0, { kind: 'not-found' });
    host.emit(0, { kind: 'error', error: transportError() });
    expect(controller.state()).toEqual({ kind: 'loading' });
    host.emit(1, data(7, 1, 3));
    expect(controller.state()).toEqual(ready(7, 1, 3));
  });

  it('the new campaign is not held to the old campaign\'s asOf', () => {
    const { host, controller } = setup('A');
    host.emit(0, data(4, 2, 50));
    controller.setCampagneId('B');
    host.emit(1, data(2, 1, 3));
    expect(controller.state()).toEqual(ready(2, 1, 3));
  });

  it('the same id again changes nothing', () => {
    const { host, controller } = setup('A');
    host.emit(0, data(4, 2, 5));
    controller.setCampagneId('A');
    expect(host.calls).toHaveLength(1);
    expect(controller.state()).toEqual(ready(4, 2, 5));
  });

  it('losing the campaign id releases the watch and drops the value', () => {
    const { host, controller } = setup('A');
    host.emit(0, data(4, 2, 5));
    controller.setCampagneId(undefined);
    expect(host.calls[0]!.released).toBe(true);
    expect(controller.state()).toEqual({ kind: 'idle' });
  });

  it('two campaigns side by side: an event for A leaves B untouched', () => {
    const host = new FakeHost();
    const a = new PartyLevelController(host, () => undefined);
    const b = new PartyLevelController(host, () => undefined);
    a.setCampagneId('A');
    b.setCampagneId('B');
    host.emit(0, data(4, 2, 5));
    host.emit(1, data(9, 1, 5));
    host.emit(0, data(5, 3, 6));
    expect(a.state()).toEqual(ready(5, 3, 6));
    expect(b.state()).toEqual(ready(9, 1, 5));
  });

  it('an answer replayed inside watch() (a loaded key) is handled, before the unsubscribe exists', () => {
    const host = new FakeHost();
    host.onWatch = (call) => {
      call.listener(data(6, 2, 4));
    };
    const controller = new PartyLevelController(host, () => undefined);
    controller.setCampagneId('A');
    expect(controller.state()).toEqual(ready(6, 2, 4));
    controller.setCampagneId('B');
    expect(host.calls[0]!.released).toBe(true);
  });

  it('a switch made from inside the replayed answer releases the watch it just obtained', () => {
    const host = new FakeHost();
    const controller = new PartyLevelController(host, () => undefined);
    host.onWatch = (call) => {
      if (call.variables['campagneId'] === 'A') controller.setCampagneId('B');
    };
    controller.setCampagneId('A');
    expect(host.calls.map((c) => [c.variables['campagneId'], c.released])).toEqual([
      ['A', true],
      ['B', false],
    ]);
  });
});

describe('not found (rules 23, 25)', () => {
  it('drops the prior value and is not "no party level"', () => {
    const { host, controller } = setup();
    host.emit(0, data(4, 2, 5));
    host.emit(0, { kind: 'not-found' });
    expect(controller.state()).toEqual({ kind: 'not-found' });
  });

  it('the first answer may be not-found', () => {
    const { host, controller } = setup();
    host.emit(0, { kind: 'not-found' });
    expect(controller.state()).toEqual({ kind: 'not-found' });
  });

  it('recovers on the next answer', () => {
    const { host, controller } = setup();
    host.emit(0, { kind: 'not-found' });
    host.emit(0, data(3, 1, 2));
    expect(controller.state()).toEqual(ready(3, 1, 2));
  });
});

describe('a failed read (rule 24)', () => {
  it.each([
    ['an error', { kind: 'error', error: transportError() } as const],
    ['a stale answer', { kind: 'stale', asOf: 4, wanted: 9 } as const],
    ['a malformed result', { kind: 'data', data: result(0, 1, 5), asOf: 5 } as const],
    ['a level with no PC', { kind: 'data', data: result(3, 0, 5), asOf: 5 } as const],
    ['a null level with PCs', { kind: 'data', data: result(null, 2, 5), asOf: 5 } as const],
    ['a non-object result', { kind: 'data', data: 'oops', asOf: 5 } as const],
  ])('%s with no prior value is unavailable, never 0 or "no party level"', (_name, event) => {
    const { host, controller } = setup();
    host.emit(0, event);
    expect(controller.state()).toEqual({ kind: 'unavailable' });
  });

  it.each([
    ['an error', { kind: 'error', error: transportError() } as const],
    ['a stale answer', { kind: 'stale', asOf: 4, wanted: 9 } as const],
    ['a malformed result', { kind: 'data', data: result(0, 1, 9), asOf: 9 } as const],
  ])('%s keeps the last good value, marked possibly stale', (_name, event) => {
    const { host, controller } = setup();
    host.emit(0, data(4, 2, 5));
    host.emit(0, event);
    expect(controller.state()).toEqual(ready(4, 2, 5, true));
  });

  it('recovers by itself on the next answer, and the mark goes', () => {
    const { host, controller } = setup();
    host.emit(0, data(4, 2, 5));
    host.emit(0, { kind: 'error', error: transportError() });
    host.emit(0, data(5, 3, 6));
    expect(controller.state()).toEqual(ready(5, 3, 6));
    host.emit(0, { kind: 'error', error: transportError() });
    host.emit(0, { kind: 'not-found' });
    host.emit(0, { kind: 'error', error: transportError() });
    expect(controller.state()).toEqual({ kind: 'unavailable' });
    host.emit(0, data(5, 3, 7));
    expect(controller.state()).toEqual(ready(5, 3, 7));
  });
});

describe('unmount (rule 22)', () => {
  it('releases the watch and later events have no effect', () => {
    const { host, controller, changes } = setup();
    host.emit(0, data(4, 2, 5));
    controller.dispose();
    expect(host.active).toHaveLength(0);
    const before = changes.length;
    host.emit(0, data(9, 9, 9));
    host.emit(0, { kind: 'not-found' });
    expect(controller.state()).toEqual(ready(4, 2, 5));
    expect(changes).toHaveLength(before);
  });

  it('a campaign id given after dispose starts nothing; dispose twice is harmless', () => {
    const { host, controller } = setup();
    controller.dispose();
    controller.dispose();
    controller.setCampagneId('B');
    expect(host.calls).toHaveLength(1);
  });
});
