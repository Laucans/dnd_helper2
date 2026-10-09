import { describe, expect, it } from 'vitest';
import { ClientDisposedError, TransportError, type CommandView } from '../src/index';
import { CMD, accepted, harness, json, notFound, pending, settled } from './fakes';

const ahead = { message: 'ValueDeclaredAhead', to: ['gm'], yourValue: { niveau: 9 }, actions: ['confirm_overwrite', 'cancel', 'edit'] };
const base = { dataCapability: 'campagne.modifierPJ', version: 1, mode: 'relative' as const, payload: {} };

describe('awaitResult (rules 28–34)', () => {
  it('resolves only on a terminal state and surfaces the others as they are', async () => {
    const h = harness();
    h.fetch.enqueue(
      pending('queued', [{ message: 'CommandQueued', to: ['gm'] }]),
      pending('awaiting_confirmation', [ahead], { yourValue: { niveau: 9 } }),
      pending('parked', [{ message: 'Parked', to: ['gm'], reason: 'stale-at-head', actions: ['confirm_overwrite', 'cancel'] }]),
      settled('applied', { dataVersion: 12 }),
    );
    const states: CommandView[] = [];
    const p = h.client.awaitResult(CMD, { onState: (v) => states.push(v) });
    await h.clock.runAll();
    const outcome = await p;
    expect(outcome).toMatchObject({ kind: 'settled', result: { status: 'applied', dataVersion: 12 }, violationMessages: [] });
    expect(states.map((s) => (s.kind === 'pending' ? s.state : s.kind))).toEqual(['queued', 'awaiting_confirmation', 'parked']);
    const waiting = states[1];
    expect(waiting).toMatchObject({ kind: 'pending', yourValue: { niveau: 9 }, actions: ['confirm_overwrite', 'cancel', 'edit'] });
    expect(h.client.knownDataVersion()).toBe(12);
    expect(h.fetch.calls.every((c) => c.method === 'GET')).toBe(true);
  });

  it('polls with bounded backoff', async () => {
    const h = harness({ poll: { initialMs: 100, maxMs: 400, factor: 2 } });
    h.fetch.always(pending('queued'));
    void h.client.awaitResult(CMD, { timeoutMs: 100_000 });
    await h.clock.advance(0);
    expect(h.fetch.calls).toHaveLength(1);
    await h.clock.advance(100);
    expect(h.fetch.calls).toHaveLength(2);
    await h.clock.advance(200);
    expect(h.fetch.calls).toHaveLength(3);
    await h.clock.advance(400);
    await h.clock.advance(400);
    expect(h.fetch.calls).toHaveLength(5);
    h.client.dispose();
  });

  it('a rejection resolves as a value, with the mapped messages beside it', async () => {
    const h = harness();
    h.fetch.enqueue(settled('rejected', { violations: ['pc-level-range', 'who-knows'] }));
    const outcome = await h.client.awaitResult(CMD, { violationMessages: { 'pc-level-range': { field: 'level', message: 'Niveau 1–20' } } });
    expect(outcome.kind).toBe('settled');
    if (outcome.kind !== 'settled') return;
    expect(Object.keys(outcome.result)).toEqual(['commandId', 'status', 'dataVersion', 'violations', 'reviewId']);
    expect(outcome.result).toMatchObject({ status: 'rejected', dataVersion: null, violations: ['pc-level-range', 'who-knows'] });
    expect(outcome.violationMessages.map((m) => [m.id, m.field, m.mapped])).toEqual([['pc-level-range', 'level', true], ['who-knows', null, false]]);
    expect(h.client.knownDataVersion()).toBeNull();
  });

  it('a timeout is "still pending" with the commandId — no POST, no rejection', async () => {
    const h = harness();
    h.fetch.always(pending('awaiting_confirmation', [ahead]));
    const p = h.client.awaitResult(CMD, { timeoutMs: 1000 });
    await h.clock.advance(1000);
    const outcome = await p;
    expect(outcome).toMatchObject({ kind: 'still-pending', commandId: CMD, reason: 'timeout', last: { kind: 'pending', state: 'awaiting_confirmation' } });
    expect(h.fetch.calls.every((c) => c.method === 'GET')).toBe(true);
    const callsAtTimeout = h.fetch.calls.length;
    await h.clock.advance(60_000);
    expect(h.fetch.calls).toHaveLength(callsAtTimeout); // polling stopped
  });

  it('an abort is "still pending", and a later wait resumes and settles', async () => {
    const h = harness();
    h.fetch.enqueue(pending('queued'));
    const ac = new AbortController();
    const first = h.client.awaitResult(CMD, { signal: ac.signal });
    await h.clock.advance(0);
    ac.abort();
    expect(await first).toMatchObject({ kind: 'still-pending', commandId: CMD, reason: 'aborted' });

    h.fetch.enqueue(settled('applied'));
    const second = h.client.awaitResult(CMD);
    await h.clock.runAll();
    expect(await second).toMatchObject({ kind: 'settled', result: { status: 'applied' } });
  });

  it('a signal that is already aborted ends at once, without a request', async () => {
    const h = harness();
    const ac = new AbortController();
    ac.abort();
    expect(await h.client.awaitResult(CMD, { signal: ac.signal })).toMatchObject({ kind: 'still-pending', reason: 'aborted' });
    expect(h.fetch.calls).toHaveLength(0);
  });

  it('a terminal result is final and cached: no further request, a later answer is ignored', async () => {
    const h = harness();
    h.fetch.enqueue(settled('applied', { dataVersion: 4 }), settled('rejected', { violations: ['x'] }));
    await h.client.awaitResult(CMD);
    const again = await h.client.awaitResult(CMD);
    expect(again).toMatchObject({ kind: 'settled', result: { status: 'applied' } });
    expect(h.fetch.calls).toHaveLength(1);
    const view = await h.client.lookup(CMD);
    expect(view).toMatchObject({ kind: 'settled', result: { status: 'applied' } });
  });

  it('the first terminal state wins over a later pending answer', async () => {
    const h = harness();
    h.fetch.enqueue(settled('expired'), pending('queued'));
    await h.client.lookup(CMD);
    expect(await h.client.lookup(CMD)).toMatchObject({ kind: 'settled', result: { status: 'expired' } });
  });

  it('applied raises the known version; expired and cancelled change nothing', async () => {
    const h = harness();
    h.fetch.enqueue(settled('cancelled'), settled('expired'));
    await h.client.lookup('a');
    await h.client.lookup('b');
    expect(h.client.knownDataVersion()).toBeNull();
  });

  it('404 not-found is its own outcome; 500 is a transport error', async () => {
    const h = harness();
    h.fetch.enqueue(notFound(), json({ error: 'internal' }, 500));
    expect(await h.client.awaitResult(CMD)).toEqual({ kind: 'not-found', commandId: CMD });
    await expect(h.client.awaitResult('other')).rejects.toBeInstanceOf(TransportError);
  });

  it('dispose ends a pending wait as "still pending / aborted" and stops all requests', async () => {
    const h = harness();
    h.fetch.always(pending('queued'));
    const p = h.client.awaitResult(CMD);
    await h.clock.advance(0);
    h.client.dispose();
    expect(await p).toMatchObject({ kind: 'still-pending', reason: 'aborted' });
    const calls = h.fetch.calls.length;
    await h.clock.runAll();
    expect(h.fetch.calls).toHaveLength(calls);
    await expect(h.client.awaitResult(CMD)).rejects.toBeInstanceOf(ClientDisposedError);
  });

  it('an own command applied by submit is final too', async () => {
    const h = harness();
    h.fetch.enqueue(accepted(), settled('applied', { dataVersion: 5 }));
    const s = await h.client.submit(base);
    expect(s.first.kind).toBe('pending');
    expect(await h.client.awaitResult(s.commandId)).toMatchObject({ kind: 'settled' });
    expect(h.client.knownDataVersion()).toBe(5);
  });

  it('survives a short run of network failures while polling, then reports them', async () => {
    const h = harness({ submitRetries: 2 });
    h.fetch.enqueue(new TypeError('down'), new TypeError('down'), settled('applied'));
    const p = h.client.awaitResult(CMD);
    await h.clock.runAll();
    expect(await p).toMatchObject({ kind: 'settled' });

    const g = harness({ submitRetries: 1 });
    g.fetch.always(new TypeError('down'));
    const failing = g.client.awaitResult(CMD).catch((e: unknown) => e);
    await g.clock.runAll();
    expect(await failing).toMatchObject({ name: 'TransportError', kind: 'network' });
    expect(g.fetch.calls).toHaveLength(2);
  });
});
