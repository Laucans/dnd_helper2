// Hermetic fakes: no socket, no environment, no real clock.

import { createShellClient, type EventSourceLike, type ShellClient, type ShellClientOptions, type Timers } from '../src/index';

export interface Call {
  url: string;
  method: string;
  body: string | undefined;
  headers: Record<string, string>;
}

export type FakeReply = Response | Error | ((call: Call) => Response | Error | Promise<Response | Error>);

export function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

export class FakeFetch {
  readonly calls: Call[] = [];
  private readonly queue: FakeReply[] = [];
  private fallback: FakeReply | null = null;

  /** Replies are consumed in order. */
  enqueue(...replies: FakeReply[]): this {
    this.queue.push(...replies);
    return this;
  }

  /** Used once the queue is empty. */
  always(reply: FakeReply): this {
    this.fallback = reply;
    return this;
  }

  readonly fetch: typeof globalThis.fetch = async (input, init) => {
    const call: Call = {
      url: typeof input === 'string' ? input : input instanceof URL ? input.href : input.url,
      method: init?.method ?? 'GET',
      body: typeof init?.body === 'string' ? init.body : undefined,
      headers: (init?.headers ?? {}) as Record<string, string>,
    };
    this.calls.push(call);
    const signal = init?.signal ?? undefined;
    if (signal?.aborted) throw new DOMException('aborted', 'AbortError');
    const queued = this.queue.shift();
    // A response body can be read once: the fallback hands out clones.
    const reply = queued ?? (this.fallback instanceof Response ? this.fallback.clone() : this.fallback);
    if (reply === null) throw new TypeError(`FakeFetch: no reply for ${call.method} ${call.url}`);
    const resolved = typeof reply === 'function' ? await reply(call) : reply;
    if (resolved instanceof Error) throw resolved;
    return resolved;
  };
}

interface Timer {
  id: number;
  at: number;
  fn: () => void;
}

const flush = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

export class FakeClock implements Timers {
  private time = 0;
  private nextId = 1;
  private timers: Timer[] = [];

  now(): number {
    return this.time;
  }

  setTimeout(fn: () => void, ms: number): unknown {
    const timer = { id: this.nextId++, at: this.time + ms, fn };
    this.timers.push(timer);
    return timer.id;
  }

  clearTimeout(handle: unknown): void {
    this.timers = this.timers.filter((t) => t.id !== handle);
  }

  get pending(): number {
    return this.timers.length;
  }

  /** Lets resolved promises run, then fires due timers in order, flushing between them. */
  async advance(ms: number): Promise<void> {
    const target = this.time + ms;
    await flush();
    for (;;) {
      const due = this.timers.filter((t) => t.at <= target).sort((a, b) => a.at - b.at || a.id - b.id)[0];
      if (due === undefined) break;
      this.timers = this.timers.filter((t) => t !== due);
      this.time = Math.max(this.time, due.at);
      due.fn();
      await flush();
    }
    this.time = target;
    await flush();
  }

  /** Fires every timer that is due now (delay 0), then lets promises settle. */
  async tick(): Promise<void> {
    await this.advance(0);
  }

  /** Runs timers until none remain (bounded, so a reconnect loop cannot hang a test). */
  async runAll(limit = 1000): Promise<void> {
    await flush();
    for (let i = 0; i < limit && this.timers.length > 0; i += 1) {
      const next = Math.min(...this.timers.map((t) => t.at));
      await this.advance(Math.max(0, next - this.time));
    }
  }
}

type Listener = (event: { data?: unknown }) => void;

export class FakeEventSource implements EventSourceLike {
  readyState = 0;
  closed = false;
  private readonly listeners = new Map<string, Listener[]>();

  constructor(readonly url: string) {}

  addEventListener(type: string, listener: Listener): void {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  close(): void {
    this.closed = true;
    this.readyState = 2;
  }

  open(): void {
    this.readyState = 1;
    this.dispatch('open', {});
  }

  emit(event: string, data: string): void {
    this.dispatch(event, { data });
  }

  /** `readyState` 2: the browser gave up; 0: it is retrying by itself. */
  fail(readyState: 0 | 2): void {
    this.readyState = readyState;
    this.dispatch('error', {});
  }

  private dispatch(type: string, event: { data?: unknown }): void {
    for (const l of this.listeners.get(type) ?? []) l(event);
  }
}

export function fakeEventSourceFactory(): { create: (url: string) => FakeEventSource; instances: FakeEventSource[] } {
  const instances: FakeEventSource[] = [];
  return {
    instances,
    create: (url) => {
      const source = new FakeEventSource(url);
      instances.push(source);
      return source;
    },
  };
}

export const BASE = 'http://127.0.0.1:7878';

export interface Harness {
  fetch: FakeFetch;
  clock: FakeClock;
  sources: ReturnType<typeof fakeEventSourceFactory>;
  diagnostics: unknown[];
  client: ShellClient;
}

export function harness(overrides: Partial<ShellClientOptions> = {}): Harness {
  const fetch = new FakeFetch();
  const clock = new FakeClock();
  const sources = fakeEventSourceFactory();
  const diagnostics: unknown[] = [];
  let keys = 0;
  const client = createShellClient({
    baseUrl: BASE,
    fetch: fetch.fetch,
    eventSource: sources.create,
    timers: clock,
    newIdempotencyKey: () => `key-${String(++keys)}`,
    onDiagnostic: (d) => diagnostics.push(d),
    ...overrides,
  });
  return { fetch, clock, sources, diagnostics, client };
}

// --- canned server answers -------------------------------------------------

export const CMD = '11111111-1111-4111-8111-111111111111';

export function entry(state: string, extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    command: CMD,
    dataCapability: 'campagne.modifierPJ@1',
    by: 'gm',
    partition: 'PJ/22222222-2222-4222-8222-222222222222',
    position: 0,
    basedOn: { version: 3 },
    state,
    confirmation: null,
    parked: null,
    requeuedFrom: null,
    ...extra,
  };
}

export function result(status: string, extra: Record<string, unknown> = {}): Record<string, unknown> {
  return { commandId: CMD, status, dataVersion: status === 'applied' ? 4 : null, violations: [], reviewId: null, ...extra };
}

export const pending = (state: string, messages: unknown[] = [], extra: Record<string, unknown> = {}): Response =>
  json({ entry: entry(state, extra), messages });

export const settled = (status: string, extra: Record<string, unknown> = {}): Response =>
  json({ result: result(status, extra) });

export const accepted = (state = 'queued', replayed = false, id = CMD): Response =>
  json({ commandId: id, partition: 'PJ/22222222-2222-4222-8222-222222222222', replayed, warnings: [], entry: entry(state, { command: id }), messages: [] }, 202);

export const notFound = (): Response => json({ error: 'not-found' }, 404);

export const dataVersionEvent = (v: number): string => JSON.stringify({ dataVersion: v });
