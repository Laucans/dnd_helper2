import { describe, expect, it } from 'vitest';
import { dataVersionEvent, harness } from './fakes';

describe('dataVersion subscription (rules 42–45, 50)', () => {
  it('opens one stream on the first listener, shares it, closes on the last', () => {
    const h = harness();
    const a = h.client.onDataVersion(() => undefined);
    const b = h.client.onDataVersion(() => undefined);
    expect(h.sources.instances).toHaveLength(1);
    expect(h.sources.instances[0]?.url).toBe('http://127.0.0.1:7878/data-version');
    a();
    expect(h.sources.instances[0]?.closed).toBe(false);
    b();
    expect(h.sources.instances[0]?.closed).toBe(true);
    b(); // idempotent
    a();
    expect(h.sources.instances).toHaveLength(1);
  });

  it('opens nothing before a listener exists', () => {
    expect(harness().sources.instances).toHaveLength(0);
  });

  it('yields only strictly greater versions; the first one counts, even 0', () => {
    const h = harness();
    const seen: number[] = [];
    h.client.onDataVersion((v) => seen.push(v));
    const es = h.sources.instances[0]!;
    es.emit('dataVersion', dataVersionEvent(0));
    for (const v of [3, 3, 2, 5]) es.emit('dataVersion', dataVersionEvent(v));
    expect(seen).toEqual([0, 3, 5]);
    expect(h.client.knownDataVersion()).toBe(5);
  });

  it('drops a malformed payload without a throw, a callback or a refetch', () => {
    const h = harness();
    const seen: number[] = [];
    h.client.onDataVersion((v) => seen.push(v));
    const es = h.sources.instances[0]!;
    es.emit('dataVersion', 'not json');
    es.emit('dataVersion', '{"dataVersion":-1}');
    es.emit('dataVersion', '{"dataVersion":null}');
    es.emit('somethingElse', dataVersionEvent(9));
    expect(seen).toEqual([]);
    expect(h.client.knownDataVersion()).toBeNull();
    expect(h.diagnostics).toHaveLength(3);
  });

  it('a throwing listener does not stop the others', () => {
    const h = harness();
    const seen: number[] = [];
    h.client.onDataVersion(() => {
      throw new Error('boom');
    });
    h.client.onDataVersion((v) => seen.push(v));
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(1));
    expect(seen).toEqual([1]);
    expect(h.diagnostics).toEqual([{ kind: 'listener-threw' }]);
  });

  it('unsubscribing stops all further callbacks to that listener', () => {
    const h = harness();
    const seen: number[] = [];
    const off = h.client.onDataVersion((v) => seen.push(v));
    h.client.onDataVersion(() => undefined);
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(1));
    off();
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(2));
    expect(seen).toEqual([1]);
  });

  it('reconnects by itself when the browser gave up (readyState 2), with bounded backoff', async () => {
    const h = harness({ reconnect: { initialMs: 100, maxMs: 300 } });
    h.client.onDataVersion(() => undefined);
    h.sources.instances[0]!.fail(2);
    expect(h.sources.instances[0]?.closed).toBe(true);
    await h.clock.advance(99);
    expect(h.sources.instances).toHaveLength(1);
    await h.clock.advance(1);
    expect(h.sources.instances).toHaveLength(2);
    h.sources.instances[1]!.fail(2);
    await h.clock.advance(199);
    expect(h.sources.instances).toHaveLength(2);
    await h.clock.advance(1);
    expect(h.sources.instances).toHaveLength(3);
    h.sources.instances[2]!.fail(2);
    await h.clock.advance(300); // capped
    expect(h.sources.instances).toHaveLength(4);
  });

  it('leaves the native reconnect alone when readyState is 0', async () => {
    const h = harness();
    h.client.onDataVersion(() => undefined);
    h.sources.instances[0]!.fail(0);
    await h.clock.runAll();
    expect(h.sources.instances).toHaveLength(1);
    expect(h.sources.instances[0]?.closed).toBe(false);
  });

  it('does not reconnect once the last listener left, nor after dispose', async () => {
    const h = harness();
    const off = h.client.onDataVersion(() => undefined);
    h.sources.instances[0]!.fail(2);
    off();
    await h.clock.runAll();
    expect(h.sources.instances).toHaveLength(1);

    const g = harness();
    g.client.onDataVersion(() => undefined);
    g.sources.instances[0]!.fail(2);
    g.client.dispose();
    await g.clock.runAll();
    expect(g.sources.instances).toHaveLength(1);
  });

  it('refetches once after a reconnect, not before the stream is back', async () => {
    const h = harness();
    h.fetch.always(() => new Response(JSON.stringify({ asOf: 1, data: 'x' }), { status: 200 }));
    const events: unknown[] = [];
    h.client.watch('campagne.listerCampagnes', {}, (e) => events.push(e));
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(1);
    h.sources.instances[0]!.fail(2);
    await h.clock.runAll();
    expect(h.fetch.calls).toHaveLength(1);
    h.sources.instances[1]!.open();
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(2);
    h.sources.instances[1]!.open(); // a second open without an error in between
    await h.clock.tick();
    expect(h.fetch.calls).toHaveLength(2);
  });
});
