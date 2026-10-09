import { describe, expect, it, vi } from 'vitest';
import { ClientDisposedError } from '../src/index';
import { CMD, accepted, dataVersionEvent, harness, json, pending, settled } from './fakes';
import type { EventSourceLike } from '../src/index';

describe('client lifecycle (rule 34)', () => {
  it('after dispose every method throws, the stream is closed, and nothing runs again', async () => {
    const h = harness();
    h.fetch.always(() => json({ asOf: 1, data: 'v' }));
    h.client.onDataVersion(() => undefined);
    h.client.watch('campagne.listerPjs', {}, () => undefined);
    await h.clock.tick();
    const calls = h.fetch.calls.length;
    h.client.dispose();
    h.client.dispose(); // idempotent
    expect(h.sources.instances[0]?.closed).toBe(true);

    expect(() => h.client.onDataVersion(() => undefined)).toThrow(ClientDisposedError);
    expect(() => h.client.watch('campagne.listerPjs', {}, () => undefined)).toThrow(ClientDisposedError);
    await expect(h.client.read('campagne.listerPjs')).rejects.toBeInstanceOf(ClientDisposedError);
    await expect(h.client.submit({ dataCapability: 'campagne.modifierPJ', version: 1, mode: 'relative', payload: {} })).rejects.toBeInstanceOf(ClientDisposedError);
    await expect(h.client.lookup(CMD)).rejects.toBeInstanceOf(ClientDisposedError);
    await expect(h.client.awaitResult(CMD)).rejects.toBeInstanceOf(ClientDisposedError);
    await expect(h.client.act(CMD, 'cancel')).rejects.toBeInstanceOf(ClientDisposedError);
    await h.clock.runAll();
    expect(h.fetch.calls).toHaveLength(calls);
  });

  it('writes nothing to the console without a diagnostic hook, and logs no payload or key', async () => {
    const spies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) => vi.spyOn(console, m).mockImplementation(() => undefined));
    const h = harness({ onDiagnostic: undefined as never });
    h.fetch.always((call) => (call.url.includes('/capabilities/') ? json({ asOf: 1, data: 'v' }) : json({ error: 'unexpected' }, 500)));
    h.client.onDataVersion(() => {
      throw new Error('boom');
    });
    h.client.watch('campagne.listerPjs', {}, () => undefined);
    h.sources.instances[0]!.emit('dataVersion', 'garbage');
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(1));
    await h.clock.tick();
    h.fetch.enqueue(accepted(), pending('queued'), settled('applied'));
    const s = await h.client.submit({ dataCapability: 'campagne.modifierPJ', version: 1, mode: 'relative', payload: { secret: 'x' } });
    const w = h.client.awaitResult(s.commandId);
    await h.clock.runAll();
    await w;
    h.client.dispose();
    for (const spy of spies) expect(spy).not.toHaveBeenCalled();
    vi.restoreAllMocks();
  });

  it('diagnostics carry a kind only — never a payload value or a key', () => {
    const h = harness();
    h.client.onDataVersion(() => {
      throw new Error('secret-payload');
    });
    h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(1));
    h.sources.instances[0]!.emit('dataVersion', 'secret-key');
    expect(JSON.stringify(h.diagnostics)).not.toMatch(/secret/);
  });

  it('a throwing diagnostic hook cannot break the shell', () => {
    const h = harness({
      onDiagnostic: () => {
        throw new Error('hook');
      },
    });
    h.client.onDataVersion(() => {
      throw new Error('boom');
    });
    expect(() => h.sources.instances[0]!.emit('dataVersion', dataVersionEvent(1))).not.toThrow();
  });

  it('the real EventSource type is accepted by the factory option (type level)', () => {
    const factory: (url: string) => EventSourceLike = (url) => new EventSource(url);
    expect(typeof factory).toBe('function');
  });

  it('calls the injected fetch detached, not as a method of the options', async () => {
    let detachedCall = false;
    const detached = function (this: unknown): Promise<Response> {
      detachedCall = this === undefined;
      return Promise.resolve(json({ asOf: 0, data: 1 }));
    };
    const h = harness({ fetch: detached as unknown as typeof fetch });
    await h.client.read('campagne.listerCampagnes');
    expect(detachedCall).toBe(true);
  });
});
