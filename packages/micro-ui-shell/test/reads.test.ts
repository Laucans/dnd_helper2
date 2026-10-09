import { describe, expect, it } from 'vitest';
import { stableStringify } from '../src/reads';
import type { WatchEvent } from '../src/index';
import { CMD, accepted, dataVersionEvent, harness, json, notFound, settled } from './fakes';

const answer = (data: unknown, asOf: number | null = 1): Response => json(asOf === null ? { data } : { asOf, data });
const submitBase = { dataCapability: 'campagne.modifierPJ', version: 1, mode: 'relative' as const, payload: {} };

describe('read (rules 13–19)', () => {
  it('posts exactly {"variables": …} to the identifier route', async () => {
    const h = harness();
    h.fetch.enqueue(answer([{ nom: 'A' }], 4));
    const out = await h.client.read('campagne.listerPjs', { campagneId: 'A' });
    expect(out).toEqual({ kind: 'found', data: [{ nom: 'A' }], asOf: 4 });
    const call = h.fetch.calls[0]!;
    expect(call).toMatchObject({ method: 'POST', url: 'http://127.0.0.1:7878/capabilities/campagne.listerPjs' });
    expect(call.body).toBe('{"variables":{"campagneId":"A"}}');
  });

  it('adds no variable of its own', async () => {
    const h = harness();
    h.fetch.enqueue(answer([]));
    await h.client.read('campagne.listerCampagnes');
    expect(h.fetch.calls[0]!.body).toBe('{"variables":{}}');
  });

  it('not-found is one outcome, with no retry and no transport error', async () => {
    const h = harness();
    h.fetch.enqueue(notFound());
    expect(await h.client.read('campagne.listerPjs', { campagneId: 'gone' })).toEqual({ kind: 'not-found' });
    expect(h.fetch.calls).toHaveLength(1);
  });

  it('a 404 with another body (route not served yet) is a transport error', async () => {
    const h = harness();
    h.fetch.enqueue(new Response('nope', { status: 404 }), json({ error: 'other' }, 404));
    await expect(h.client.read('campagne.listerPjs')).rejects.toMatchObject({ name: 'TransportError', status: 404 });
    await expect(h.client.read('campagne.listerPjs')).rejects.toMatchObject({ name: 'TransportError', errorId: 'other' });
  });

  it('a 2xx without data is a protocol error', async () => {
    const h = harness();
    h.fetch.enqueue(json({ asOf: 1 }), new Response('<html>', { status: 200 }));
    await expect(h.client.read('campagne.listerPjs')).rejects.toMatchObject({ name: 'ProtocolError' });
    await expect(h.client.read('campagne.listerPjs')).rejects.toMatchObject({ name: 'ProtocolError' });
  });

  it('stableStringify ignores key order, not content', () => {
    expect(stableStringify({ a: 1, b: { d: 1, c: [1, { z: 1, y: 2 }] } })).toBe(stableStringify({ b: { c: [1, { y: 2, z: 1 }], d: 1 }, a: 1 }));
    expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: '1' }));
  });
});

describe('watch (rules 16, 17, 46–51)', () => {
  it('fetches once on subscribe and opens the stream', async () => {
    const h = harness();
    h.fetch.enqueue(answer(['x'], 2));
    const events: WatchEvent<string[]>[] = [];
    h.client.watch<string[]>('campagne.listerPjs', { campagneId: 'A' }, (e) => events.push(e));
    expect(h.sources.instances).toHaveLength(1);
    await h.clock.tick();
    expect(events).toEqual([{ kind: 'data', data: ['x'], asOf: 2 }]);
  });

  it('two watchers of one key share a fetch; another campaign never gets A\'s data', async () => {
    const h = harness();
    h.fetch.always((call) => (call.body?.includes('"A"') ? answer('data-of-A') : answer('data-of-B')));
    const a1: unknown[] = [];
    const a2: unknown[] = [];
    const b: unknown[] = [];
    h.client.watch('campagne.listerPjs', { campagneId: 'A', x: 1 }, (e) => a1.push(e));
    h.client.watch('campagne.listerPjs', { x: 1, campagneId: 'A' }, (e) => a2.push(e));
    h.client.watch('campagne.listerPjs', { campagneId: 'B' }, (e) => b.push(e));
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(2);
    expect(a1).toEqual(a2);
    expect(a1).toEqual([{ kind: 'data', data: 'data-of-A', asOf: 1 }]);
    expect(b).toEqual([{ kind: 'data', data: 'data-of-B', asOf: 1 }]);
    expect(h.sources.instances).toHaveLength(1);
  });

  it('a late watcher of a loaded key gets the last event at once, without a fetch', async () => {
    const h = harness();
    h.fetch.always(answer('v'));
    h.client.watch('campagne.listerPjs', { c: 1 }, () => undefined);
    await h.clock.tick();
    const late: unknown[] = [];
    h.client.watch('campagne.listerPjs', { c: 1 }, (e) => late.push(e));
    await h.clock.tick();
    expect(late).toEqual([{ kind: 'data', data: 'v', asOf: 1 }]);
    expect(h.fetch.calls).toHaveLength(1);
  });

  it('a burst of pushes is one refetch per entry, at the latest version', async () => {
    const h = harness();
    let asOf = 3;
    h.fetch.always(() => answer('v', asOf));
    const events: WatchEvent<string>[] = [];
    h.client.watch<string>('campagne.listerPjs', { c: 1 }, (e) => events.push(e));
    h.client.watch('campagne.listerCampagnes', {}, () => undefined);
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(2);
    asOf = 6;
    const es = h.sources.instances[0]!;
    for (const v of [4, 5, 6]) es.emit('dataVersion', dataVersionEvent(v));
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(4);
    expect(events.at(-1)).toEqual({ kind: 'data', data: 'v', asOf: 6 });
  });

  it('the same version as a push and as an own result refetches once', async () => {
    const h = harness();
    h.fetch.always(() => answer('v', 7));
    h.client.watch('campagne.listerPjs', { c: 1 }, () => undefined);
    await h.clock.tick();
    h.fetch.enqueue(accepted(), settled('applied', { dataVersion: 7 }));
    const s = await h.client.submit(submitBase);
    await h.client.awaitResult(s.commandId);
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(7));
    await h.clock.tick();
    const reads = h.fetch.calls.filter((c) => c.url.includes('/capabilities/'));
    expect(reads).toHaveLength(2); // the first load + one refetch
    expect(h.client.knownDataVersion()).toBe(7);
  });

  it('an asOf below the version just applied is retried, then reported stale — never as data', async () => {
    const h = harness({ staleRead: { retries: 2, delayMs: 50, pushWaitMs: 1000 } });
    h.fetch.always(() => answer('old', 1));
    const events: WatchEvent<string>[] = [];
    h.client.watch<string>('campagne.listerPjs', { c: 1 }, (e) => events.push(e));
    await h.clock.tick();
    expect(events).toEqual([{ kind: 'data', data: 'old', asOf: 1 }]);

    h.fetch.enqueue(accepted(), settled('applied', { dataVersion: 7 }));
    const s = await h.client.submit(submitBase);
    await h.client.awaitResult(s.commandId);
    const before = h.fetch.calls.length;
    await h.clock.tick();
    await h.clock.advance(50);
    await h.clock.advance(50);
    expect(h.fetch.calls.length - before).toBe(3); // first try + 2 retries
    expect(events.at(-1)).toEqual({ kind: 'stale', asOf: 1, wanted: 7 });
    expect(events.filter((e) => e.kind === 'data')).toHaveLength(1);
  });

  it('presents the data as soon as a retry catches up', async () => {
    const h = harness({ staleRead: { retries: 5, delayMs: 50, pushWaitMs: 1000 } });
    const versions = [1, 1, 8];
    h.fetch.enqueue(answer('old', 1));
    const events: WatchEvent<string>[] = [];
    h.client.watch<string>('campagne.listerPjs', { c: 1 }, (e) => events.push(e));
    await h.clock.tick();
    h.fetch.always(() => answer('new', versions.shift() ?? 8));
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(8));
    await h.clock.tick();
    await h.clock.advance(50);
    await h.clock.advance(50);
    expect(events.at(-1)).toEqual({ kind: 'data', data: 'new', asOf: 8 });
  });

  it('a read without asOf waits for the push to reach the version, or a bounded time, then refetches', async () => {
    const h = harness({ staleRead: { retries: 1, delayMs: 10, pushWaitMs: 1000 } });
    h.fetch.always(() => answer('v', null));
    const events: WatchEvent<string>[] = [];
    h.client.watch<string>('campagne.listerPjs', { c: 1 }, (e) => events.push(e));
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(1);

    h.fetch.enqueue(accepted(), settled('applied', { dataVersion: 7 }));
    const s = await h.client.submit(submitBase);
    await h.client.awaitResult(s.commandId);
    const before = h.fetch.calls.length;
    await h.clock.tick();
    expect(h.fetch.calls.length - before).toBe(1); // fetched, now waiting for the push
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(7));
    await h.clock.tick();
    expect(h.fetch.calls.length - before).toBe(2); // refetched once the push arrived

    // …or after the bounded wait.
    h.fetch.enqueue(accepted('queued', false, 'cmd-2'), settled('applied', { dataVersion: 9, commandId: 'cmd-2' }));
    const s2 = await h.client.submit({ ...submitBase, idempotencyKey: 'again' });
    await h.client.awaitResult(s2.commandId);
    const mid = h.fetch.calls.length;
    await h.clock.tick();
    await h.clock.advance(1000);
    expect(h.fetch.calls.length - mid).toBe(2);
    expect(events.at(-1)).toEqual({ kind: 'data', data: 'v', asOf: null });
  });

  it('a rejected result triggers no refetch; an applied null refetches once without moving the version', async () => {
    const h = harness();
    h.fetch.always(() => answer('v', 2));
    h.client.watch('campagne.listerPjs', { c: 1 }, () => undefined);
    await h.clock.tick();
    h.fetch.enqueue(settled('rejected', { violations: ['x'] }));
    await h.client.lookup('r1');
    await h.clock.tick();
    expect(h.fetch.calls.filter((c) => c.url.includes('/capabilities/'))).toHaveLength(1);

    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(2));
    await h.clock.tick();
    const base = h.fetch.calls.filter((c) => c.url.includes('/capabilities/')).length;
    h.fetch.enqueue(settled('applied', { dataVersion: null }));
    await h.client.lookup('n1');
    await h.clock.tick();
    expect(h.fetch.calls.filter((c) => c.url.includes('/capabilities/'))).toHaveLength(base + 1);
    expect(h.client.knownDataVersion()).toBe(2);
  });

  it('a message without dataVersion triggers no refetch (a pending answer)', async () => {
    const h = harness();
    h.fetch.always(() => answer('v', 2));
    h.client.watch('campagne.listerPjs', { c: 1 }, () => undefined);
    await h.clock.tick();
    h.fetch.enqueue(json({ entry: { command: CMD, dataCapability: 'campagne.modifierPJ@1', by: 'gm', partition: 'PJ/x', position: 0, basedOn: { version: 1 }, state: 'queued' }, messages: [{ message: 'CommandQueued', to: ['gm'] }] }));
    await h.client.lookup(CMD);
    await h.clock.tick();
    expect(h.fetch.calls.filter((c) => c.url.includes('/capabilities/'))).toHaveLength(1);
  });

  it('not-found is emitted, not retried and not an error', async () => {
    const h = harness();
    h.fetch.always(() => notFound());
    const events: WatchEvent<unknown>[] = [];
    h.client.watch('campagne.listerPjs', { campagneId: 'gone' }, (e) => events.push(e));
    await h.clock.tick();
    expect(events).toEqual([{ kind: 'not-found' }]);
    expect(h.fetch.calls).toHaveLength(1);
  });

  it('a transport failure reaches the listener as an error event', async () => {
    const h = harness();
    h.fetch.always(() => json({ error: 'internal' }, 500));
    const events: WatchEvent<unknown>[] = [];
    h.client.watch('campagne.listerPjs', {}, (e) => events.push(e));
    await h.clock.tick();
    expect(events[0]).toMatchObject({ kind: 'error', error: { name: 'TransportError', status: 500 } });
  });

  it('a bump during an in-flight fetch yields one more fetch, unless the answer already covers it', async () => {
    const h = harness();
    let release: (() => void) | undefined;
    h.fetch.always(
      () =>
        new Promise<Response>((resolve) => {
          release = () => {
            resolve(answer('v', 5));
          };
        }),
    );
    h.client.watch('campagne.listerPjs', {}, () => undefined);
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(1);
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(5)); // covered by the in-flight answer
    await h.clock.tick();
    release?.();
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(1);
  });

  it('unsubscribing the last watcher stops the refetches and closes the stream', async () => {
    const h = harness();
    h.fetch.always(() => answer('v', 1));
    const off = h.client.watch('campagne.listerPjs', {}, () => undefined);
    await h.clock.tick();
    off();
    off();
    expect(h.sources.instances[0]?.closed).toBe(true);
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(1);
  });

  it('a throwing watcher does not stop the others', async () => {
    const h = harness();
    h.fetch.always(() => answer('v', 1));
    const seen: unknown[] = [];
    h.client.watch('campagne.listerPjs', {}, () => {
      throw new Error('boom');
    });
    h.client.watch('campagne.listerPjs', {}, (e) => seen.push(e));
    await h.clock.tick();
    expect(seen).toHaveLength(1);
    expect(h.diagnostics).toEqual([{ kind: 'listener-threw' }]);
  });
});
