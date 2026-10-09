import { describe, expect, it } from 'vitest';

import { ClientDisposedError, ProtocolError, TransportError } from '../src/errors';
import { createHttp } from '../src/http';
import { BASE, FakeFetch, json } from './fakes';

/** A fetch that never answers but, like the real one, rejects when its signal aborts. */
const hanging: typeof globalThis.fetch = (_input, init) =>
  new Promise<Response>((_resolve, reject) => {
    init?.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')), { once: true });
  });

function setup(fetchFn?: typeof globalThis.fetch): { fetch: FakeFetch; life: AbortController; http: ReturnType<typeof createHttp> } {
  const fetch = new FakeFetch();
  const life = new AbortController();
  const http = createHttp(BASE, fetchFn ?? fetch.fetch, {
    signal: life.signal,
    assertLive: () => {
      if (life.signal.aborted) throw new ClientDisposedError();
    },
  });
  return { fetch, life, http };
}

describe('http boundary (rule 52)', () => {
  it('a 2xx whose body is not JSON is a protocol failure, not an ok', async () => {
    const h = setup();
    h.fetch.enqueue(new Response('<html>', { status: 200 }));
    await expect(h.http.request('GET', '/x')).rejects.toBeInstanceOf(ProtocolError);
  });

  it('404 is "not found" only for the exact not-found error id', async () => {
    const h = setup();
    h.fetch.enqueue(json({ error: 'not-found' }, 404), json({ error: 'unknown-route' }, 404), new Response('', { status: 404 }));
    await expect(h.http.request('GET', '/x')).resolves.toEqual({ kind: 'not-found' });
    await expect(h.http.request('GET', '/x')).rejects.toMatchObject({ kind: 'http', status: 404, errorId: 'unknown-route' });
    await expect(h.http.request('GET', '/x')).rejects.toMatchObject({ kind: 'http', status: 404, errorId: undefined });
  });

  it('a non-404 with the not-found id is still an http failure', async () => {
    const h = setup();
    h.fetch.enqueue(json({ error: 'not-found' }, 500));
    await expect(h.http.request('GET', '/x')).rejects.toBeInstanceOf(TransportError);
  });

  it('a body that cannot be read is a network failure', async () => {
    const h = setup();
    const broken = new Response('x', { status: 200 });
    Object.defineProperty(broken, 'text', { value: () => Promise.reject(new TypeError('terminated')) });
    h.fetch.enqueue(broken);
    await expect(h.http.request('GET', '/x')).rejects.toMatchObject({ name: 'TransportError', kind: 'network' });
  });

  it('a caller abort surfaces as the abort, not as a transport failure', async () => {
    const h = setup(hanging);
    const ac = new AbortController();
    const p = h.http.request('GET', '/x', { signal: ac.signal });
    ac.abort();
    const err = await p.catch((e: unknown) => e);
    expect(err).not.toBeInstanceOf(TransportError);
    expect(err).not.toBeInstanceOf(ClientDisposedError);
  });

  it('a dispose during a request surfaces as ClientDisposedError, then nothing more is sent', async () => {
    let sent = 0;
    const h = setup((input, init) => {
      sent += 1;
      return hanging(input, init);
    });
    const p = h.http.request('GET', '/x');
    h.life.abort();
    await expect(p).rejects.toBeInstanceOf(ClientDisposedError);
    await expect(h.http.request('GET', '/y')).rejects.toBeInstanceOf(ClientDisposedError);
    expect(sent).toBe(1);
  });
});
