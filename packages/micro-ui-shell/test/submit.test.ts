import { describe, expect, it } from 'vitest';
import { MissingBasedOnError, TransportError } from '../src/index';
import { CMD, accepted, entry, harness, json, settled } from './fakes';

const base = { dataCapability: 'campagne.modifierPJ', version: 1, mode: 'relative' as const, payload: { nom: 'Aria' } };

describe('submit (rules 20–27)', () => {
  it('sends the wire id, a key, the payload — no by, no mode, no aggregate', async () => {
    const h = harness();
    h.fetch.enqueue(accepted());
    const s = await h.client.submit({ ...base, target: { aggregate: 'PJ', id: 'abc' } });
    const call = h.fetch.calls[0]!;
    expect(call.method).toBe('POST');
    expect(call.url).toBe('http://127.0.0.1:7878/commands');
    expect(JSON.parse(call.body!)).toEqual({
      dataCapability: 'campagne.modifierPJ@1',
      payload: { nom: 'Aria' },
      idempotencyKey: 'key-1',
      target: { id: 'abc' },
    });
    expect(call.body).not.toMatch(/"by"|"mode"|"aggregate"/);
    expect(s).toMatchObject({ commandId: CMD, replayed: false, idempotencyKey: 'key-1' });
    expect(s.first.kind).toBe('pending');
  });

  it('sends no `target` key for an insert without an id', async () => {
    const h = harness();
    h.fetch.enqueue(accepted());
    await h.client.submit({ ...base, target: { aggregate: 'PJ' } });
    await Promise.resolve();
    expect(JSON.parse(h.fetch.calls[0]!.body!)).not.toHaveProperty('target');
  });

  it('two submits with identical payloads get two distinct keys (no dedupe by content)', async () => {
    const h = harness();
    h.fetch.always(accepted());
    await h.client.submit(base);
    await h.client.submit(base);
    const keys = h.fetch.calls.map((c) => (JSON.parse(c.body!) as { idempotencyKey: string }).idempotencyKey);
    expect(new Set(keys).size).toBe(2);
  });

  it('uses the real UUID generator by default', async () => {
    const h = harness({ newIdempotencyKey: undefined as never });
    h.fetch.enqueue(accepted());
    const s = await h.client.submit(base);
    expect(s.idempotencyKey).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });

  it('sends a caller-supplied key verbatim, so a double click replays', async () => {
    const h = harness();
    h.fetch.enqueue(accepted(), accepted('queued', true));
    await h.client.submit({ ...base, idempotencyKey: 'form-attempt-7' });
    const again = await h.client.submit({ ...base, idempotencyKey: 'form-attempt-7' });
    expect(h.fetch.calls.map((c) => (JSON.parse(c.body!) as { idempotencyKey: string }).idempotencyKey)).toEqual(['form-attempt-7', 'form-attempt-7']);
    expect(again).toMatchObject({ replayed: true, commandId: CMD });
  });

  it('retries a network failure with the same key, bounded', async () => {
    const h = harness({ submitRetries: 2 });
    h.fetch.enqueue(new TypeError('down'), new TypeError('down'), accepted());
    const p = h.client.submit(base);
    await h.clock.runAll();
    await p;
    expect(h.fetch.calls).toHaveLength(3);
    expect(new Set(h.fetch.calls.map((c) => c.body)).size).toBe(1);

    const g = harness({ submitRetries: 1 });
    g.fetch.always(new TypeError('down'));
    const failing = g.client.submit(base).catch((e: unknown) => e);
    await g.clock.runAll();
    expect(await failing).toMatchObject({ name: 'TransportError', kind: 'network' });
    expect(g.fetch.calls).toHaveLength(2);
  });

  it('never retries an HTTP status, and keeps the error id', async () => {
    const h = harness();
    h.fetch.enqueue(json({ error: 'idempotency-key-conflict' }, 400));
    const err = await h.client.submit(base).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(TransportError);
    expect(err).toMatchObject({ kind: 'http', status: 400, errorId: 'idempotency-key-conflict' });
    expect(h.fetch.calls).toHaveLength(1);
  });

  it('an HTTP error body that is not JSON yields a status and no text', async () => {
    const h = harness();
    h.fetch.enqueue(new Response('<html>secret payload</html>', { status: 500 }));
    const err = (await h.client.submit(base).catch((e: unknown) => e)) as TransportError;
    expect(err).toMatchObject({ kind: 'http', status: 500, errorId: undefined });
    expect(err.message).not.toContain('secret');
  });

  it('refuses confirm_on_stale without basedOn locally, and never fills one in', async () => {
    const h = harness();
    await expect(h.client.submit({ ...base, mode: 'confirm_on_stale' })).rejects.toThrow(MissingBasedOnError);
    expect(h.fetch.calls).toHaveLength(0);
    h.fetch.enqueue(accepted());
    await h.client.submit({ ...base, mode: 'confirm_on_stale', basedOn: { version: 0 } });
    expect(JSON.parse(h.fetch.calls[0]!.body!)).toHaveProperty('basedOn', { version: 0 });
  });

  it('sends the payload as given: no trim, no coercion, no default', async () => {
    const h = harness();
    h.fetch.always(accepted());
    await h.client.submit({ ...base, payload: { niveau: '5', nom: '' } });
    await h.client.submit({ ...base, payload: { niveau: 0 } });
    await h.client.submit({ ...base, payload: { nom: '  Aria  ', niveau: 5.5 } });
    const payloads = h.fetch.calls.map((c) => (JSON.parse(c.body!) as { payload: unknown }).payload);
    expect(payloads).toEqual([{ niveau: '5', nom: '' }, { niveau: 0 }, { nom: '  Aria  ', niveau: 5.5 }]);
    expect(h.fetch.calls[1]!.body).toContain('"payload":{"niveau":0}');
  });

  it('a replay that is already settled is the original command, terminal at once', async () => {
    const h = harness();
    h.fetch.enqueue(json({ commandId: CMD, partition: 'PJ/x', replayed: true, warnings: [], result: { commandId: CMD, status: 'applied', dataVersion: 9, violations: [], reviewId: null } }, 202));
    const s = await h.client.submit(base);
    expect(s.first).toMatchObject({ kind: 'settled', result: { commandId: CMD, status: 'applied' } });
    expect(s.commandId).toBe(CMD);
    expect(h.client.knownDataVersion()).toBe(9);
  });

  it('a malformed answer is a protocol error, never an applied result', async () => {
    const h = harness();
    h.fetch.enqueue(json({ commandId: CMD, partition: 'PJ/x', replayed: false, warnings: [], result: { status: 'applied' } }, 202));
    await expect(h.client.submit(base)).rejects.toMatchObject({ name: 'ProtocolError' });
    expect(h.client.knownDataVersion()).toBeNull();
  });

  it('a submit for a queued entry is not a result and carries the state', async () => {
    const h = harness();
    h.fetch.enqueue(json({ commandId: CMD, partition: 'PJ/x', replayed: false, warnings: ['w'], entry: entry('awaiting_confirmation', { yourValue: { niveau: 7 } }), messages: [] }, 202));
    const s = await h.client.submit(base);
    expect(s.warnings).toEqual(['w']);
    expect(s.first).toMatchObject({ kind: 'pending', state: 'awaiting_confirmation', yourValue: { niveau: 7 } });
    expect(settled('applied')).toBeDefined();
  });
});
