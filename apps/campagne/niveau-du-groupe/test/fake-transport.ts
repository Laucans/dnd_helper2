// Hand-written fakes for the shell's own injection points (`fetch`, `EventSource`, the clock):
// no socket, no real timer. The shell's test fakes are outside its package exports.

import { createShellClient, type EventSourceLike, type ShellClient, type Timers } from '@dnd-helper/micro-ui-shell';

export interface Call {
  method: string;
  url: string;
  body: string | undefined;
}

export type Reply = Response | Error | (() => Response | Error | Promise<Response | Error>);

export const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

export const answer = (level: number | null, pcCount: number, asOf: number): Response =>
  json({ asOf, data: { level, pcCount, model: 'v1', asOf } });

export class FakeFetch {
  readonly calls: Call[] = [];
  private readonly queue: Reply[] = [];

  enqueue(...replies: Reply[]): this {
    this.queue.push(...replies);
    return this;
  }

  readonly fetch: typeof globalThis.fetch = async (input, init) => {
    const body = init?.body;
    this.calls.push({
      method: init?.method ?? 'GET',
      url: typeof input === 'string' ? input : input instanceof URL ? input.href : input.url,
      body: typeof body === 'string' ? body : undefined,
    });
    const reply = this.queue.shift();
    if (reply === undefined) throw new TypeError('FakeFetch: no reply queued');
    const resolved = typeof reply === 'function' ? await reply() : reply;
    if (resolved instanceof Error) throw resolved;
    return resolved;
  };
}

type Handler = (event: { data?: unknown }) => void;

export class FakeEventSource implements EventSourceLike {
  readyState = 1;
  private readonly handlers = new Map<string, Handler[]>();

  addEventListener(type: string, listener: Handler): void {
    this.handlers.set(type, [...(this.handlers.get(type) ?? []), listener]);
  }

  close(): void {
    this.readyState = 2;
  }

  push(version: number): void {
    for (const handler of this.handlers.get('dataVersion') ?? []) handler({ data: JSON.stringify({ dataVersion: version }) });
  }
}

interface Timer {
  id: number;
  at: number;
  fn: () => void;
}

const settle = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

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

  /** Lets resolved promises run, fires the timers due within `ms` in order, and lets promises run again. */
  async advance(ms: number): Promise<void> {
    const target = this.time + ms;
    await settle();
    for (;;) {
      const due = this.timers.filter((t) => t.at <= target).sort((a, b) => a.at - b.at || a.id - b.id)[0];
      if (due === undefined) break;
      this.timers = this.timers.filter((t) => t !== due);
      this.time = Math.max(this.time, due.at);
      due.fn();
      await settle();
    }
    this.time = target;
    await settle();
  }

  tick(): Promise<void> {
    return this.advance(0);
  }
}

export interface Wire {
  shell: ShellClient;
  fetch: FakeFetch;
  clock: FakeClock;
  sources: FakeEventSource[];
}

export function wire(): Wire {
  const fetch = new FakeFetch();
  const clock = new FakeClock();
  const sources: FakeEventSource[] = [];
  const shell = createShellClient({
    baseUrl: 'http://127.0.0.1:7878',
    fetch: fetch.fetch,
    eventSource: () => {
      const source = new FakeEventSource();
      sources.push(source);
      return source;
    },
    timers: clock,
  });
  return { shell, fetch, clock, sources };
}
