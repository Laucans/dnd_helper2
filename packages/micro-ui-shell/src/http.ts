// The one place the shell calls `fetch`. It adds the dispose signal, maps the
// four outcomes of a call (rule 52) and never puts a body or a key in an error.

import { ClientDisposedError, ProtocolError, TransportError } from './errors';
import { NOT_FOUND_ERROR } from './protocol';
import { asJsonObject } from './validate';

export interface Http {
  /** `ok` carries the parsed JSON body of a 2xx; `not-found` is exactly `404 {"error":"not-found"}`. */
  request(
    method: 'GET' | 'POST',
    path: string,
    init?: { body?: string; signal?: AbortSignal | undefined },
  ): Promise<{ kind: 'ok'; body: unknown } | { kind: 'not-found' }>;
}

export interface Lifecycle {
  /** Aborts when the client is disposed. */
  readonly signal: AbortSignal;
  assertLive(): void;
}

export function createHttp(baseUrl: string, fetchFn: typeof globalThis.fetch, life: Lifecycle): Http {
  return {
    async request(method, path, init) {
      life.assertLive();
      const signal =
        init?.signal === undefined ? life.signal : AbortSignal.any([life.signal, init.signal]);
      const headers: Record<string, string> = { accept: 'application/json' };
      const req: RequestInit = { method, signal, headers };
      if (init?.body !== undefined) {
        headers['content-type'] = 'application/json';
        req.body = init.body;
      }

      let response: Response;
      try {
        response = await fetchFn(`${baseUrl}${path}`, req);
      } catch (e) {
        if (life.signal.aborted) throw new ClientDisposedError();
        if (signal.aborted) throw e; // the caller's own abort: the caller handles it
        throw new TransportError('network');
      }

      let text: string;
      try {
        text = await response.text();
      } catch {
        if (life.signal.aborted) throw new ClientDisposedError();
        if (signal.aborted) throw signal.reason;
        throw new TransportError('network');
      }

      let parsed: unknown;
      let parseable = true;
      try {
        parsed = text === '' ? undefined : JSON.parse(text);
      } catch {
        parseable = false;
      }

      if (response.status >= 200 && response.status < 300) {
        if (!parseable) throw new ProtocolError(path, 'body is not JSON');
        return { kind: 'ok', body: parsed };
      }

      const errorId = parseable ? errorIdOf(parsed) : undefined;
      if (response.status === 404 && errorId === NOT_FOUND_ERROR) return { kind: 'not-found' };
      throw new TransportError('http', response.status, errorId);
    },
  };
}

function errorIdOf(body: unknown): string | undefined {
  const o = asJsonObject(body);
  const id = o?.['error'];
  return typeof id === 'string' ? id : undefined;
}
