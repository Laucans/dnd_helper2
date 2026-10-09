// The real shell, with fakes at its own injection points (fetch, EventSource, clock): the
// coalescing of announcements and the read-after-write floor are proved end to end. Nothing here
// mocks the Capability at the call site.

import { describe, expect, it } from 'vitest';
import { PartyLevelController, type PartyLevelState } from '../src/controller';
import { answer, json, wire } from './fake-transport';

const ready = (level: number | null, pcCount: number, asOf: number, possiblyStale = false): PartyLevelState => ({ kind: 'ready', level, pcCount, asOf, possiblyStale });

function start(campagneId = 'A') {
  const w = wire();
  const states: PartyLevelState[] = [];
  const controller = new PartyLevelController(w.shell, (s) => states.push(s));
  controller.setCampagneId(campagneId);
  return { ...w, controller, states };
}

const reads = (fetch: { calls: { url: string }[] }) => fetch.calls.filter((c) => c.url.includes('/capabilities/'));

describe('through the real shell', () => {
  it('reads campagne.niveauDuGroupe with exactly {"variables":{"campagneId"}}', async () => {
    const t = start('A');
    t.fetch.enqueue(answer(4, 2, 3));
    await t.clock.tick();
    expect(reads(t.fetch)).toEqual([
      { method: 'POST', url: 'http://127.0.0.1:7878/capabilities/campagne.niveauDuGroupe', body: '{"variables":{"campagneId":"A"}}' },
    ]);
    expect(t.controller.state()).toEqual(ready(4, 2, 3));
  });

  it('announcements 5, 6 and 7 in one tick make one read, and the view ends on the asOf 7 answer', async () => {
    const t = start();
    t.fetch.enqueue(answer(3, 2, 4));
    await t.clock.tick();
    t.fetch.enqueue(answer(5, 3, 7));
    for (const v of [5, 6, 7]) t.sources[0]!.push(v);
    await t.clock.tick();
    expect(reads(t.fetch)).toHaveLength(2);
    expect(t.controller.state()).toEqual(ready(5, 3, 7));
  });

  it('an answer below the announced version never replaces the value: the previous one stays on screen until the asOf 9 answer lands', async () => {
    const t = start();
    t.fetch.enqueue(answer(3, 2, 4));
    await t.clock.tick();
    t.fetch.enqueue(answer(3, 2, 8), answer(6, 3, 9));
    t.sources[0]!.push(9);
    await t.clock.tick();
    expect(t.states.map((s) => s.kind)).toEqual(['loading', 'ready']); // no loading, no empty state in between
    await t.clock.advance(1_000);
    expect(t.controller.state()).toEqual(ready(6, 3, 9));
    expect(t.states.filter((s) => s.kind === 'ready').map((s) => s.kind === 'ready' && s.level)).toEqual([3, 6]);
  });

  it('archiving the last PC reads level null and shows an empty party', async () => {
    const t = start();
    t.fetch.enqueue(answer(4, 1, 4));
    await t.clock.tick();
    t.fetch.enqueue(answer(null, 0, 5));
    t.sources[0]!.push(5);
    await t.clock.tick();
    expect(t.controller.state()).toEqual(ready(null, 0, 5));
  });

  it('a 404 {"error":"not-found"} is the not-found state', async () => {
    const t = start();
    t.fetch.enqueue(json({ error: 'not-found' }, 404));
    await t.clock.tick();
    expect(t.controller.state()).toEqual({ kind: 'not-found' });
  });

  it('a 500 is unavailable; the next announcement and a 200 recover it', async () => {
    const t = start();
    t.fetch.enqueue(json({ error: 'boom' }, 500));
    await t.clock.tick();
    expect(t.controller.state()).toEqual({ kind: 'unavailable' });
    t.fetch.enqueue(answer(4, 2, 6));
    t.sources[0]!.push(6);
    await t.clock.tick();
    expect(t.controller.state()).toEqual(ready(4, 2, 6));
  });

  it('a failed re-read keeps the last good value, marked possibly stale', async () => {
    const t = start();
    t.fetch.enqueue(answer(4, 2, 4));
    await t.clock.tick();
    t.fetch.enqueue(json({ error: 'boom' }, 500));
    t.sources[0]!.push(5);
    await t.clock.tick();
    expect(t.controller.state()).toEqual(ready(4, 2, 4, true));
  });

  it('two campaigns side by side each read their own, and an announcement refreshes both', async () => {
    const w = wire();
    const a = new PartyLevelController(w.shell, () => undefined);
    const b = new PartyLevelController(w.shell, () => undefined);
    w.fetch.enqueue(
      () => answer(4, 2, 3),
      () => answer(9, 1, 3),
    );
    a.setCampagneId('A');
    b.setCampagneId('B');
    await w.clock.tick();
    expect(w.fetch.calls.map((c) => c.body)).toEqual(['{"variables":{"campagneId":"A"}}', '{"variables":{"campagneId":"B"}}']);
    expect(a.state()).toEqual(ready(4, 2, 3));
    expect(b.state()).toEqual(ready(9, 1, 3));
    w.fetch.enqueue(answer(5, 3, 4), answer(9, 1, 4));
    w.sources[0]!.push(4);
    await w.clock.tick();
    expect(a.state()).toEqual(ready(5, 3, 4));
    expect(b.state()).toEqual(ready(9, 1, 4));
  });

  it('after dispose a pending read has no effect and the stream is no longer held', async () => {
    const t = start();
    let release: (() => void) | undefined;
    t.fetch.enqueue(
      () =>
        new Promise<Response>((resolve) => {
          release = () => {
            resolve(answer(4, 2, 3));
          };
        }),
    );
    await t.clock.tick();
    t.controller.dispose();
    release?.();
    await t.clock.tick();
    expect(t.controller.state()).toEqual({ kind: 'loading' });
    expect(t.sources[0]!.readyState).toBe(2);
  });
});
