// The one shared dataVersion subscription (rules 42–45, 50): a monotonic known
// version, one SSE stream shared by every listener and every live read, a
// bounded reconnect, and one refetch signal for whoever re-reads.

import { ClientDisposedError } from './errors';
import { SSE_DATA_VERSION_EVENT, ROUTES } from './protocol';
import { backoff, type Diagnostic, type Timers } from './timers';
import { parseDataVersionEvent } from './validate';

export type Unsubscribe = () => void;

/** The part of `EventSource` the shell uses; the real one satisfies it. */
export interface EventSourceLike {
  readonly readyState: number;
  addEventListener(type: string, listener: (event: { data?: unknown }) => void): void;
  close(): void;
}

const CLOSED = 2;

export interface HubDeps {
  baseUrl: string;
  eventSource: (url: string) => EventSourceLike;
  timers: Timers;
  reconnect: { initialMs: number; maxMs: number };
  diagnostic: (d: Diagnostic) => void;
}

interface Waiter {
  version: number;
  resolve: (reached: boolean) => void;
  handle: unknown;
}

export class DataVersionHub {
  private known: number | null = null;
  private pushed: number | null = null;
  private readonly listeners = new Set<{ fn: (version: number) => void }>();
  private retainers = 0;
  private source: EventSourceLike | null = null;
  private reconnectTimer: unknown = null;
  private attempt = 0;
  private sawError = false;
  private closed = false;
  private readonly refetchers: Array<(force: boolean) => void> = [];
  private waiters: Waiter[] = [];
  private readonly delay: (attempt: number) => number;

  constructor(private readonly deps: HubDeps) {
    this.delay = backoff(deps.reconnect.initialMs, deps.reconnect.maxMs, 2);
  }

  knownVersion(): number | null {
    return this.known;
  }

  pushedVersion(): number | null {
    return this.pushed;
  }

  /** Called with every newly higher version. Idempotent unsubscribe. */
  addListener(fn: (version: number) => void): Unsubscribe {
    if (this.closed) throw new ClientDisposedError();
    const entry = { fn };
    this.listeners.add(entry);
    this.sync();
    return () => {
      if (this.listeners.delete(entry)) this.sync();
    };
  }

  /** A live read keeps the stream open without being a version listener. */
  retain(): Unsubscribe {
    if (this.closed) throw new ClientDisposedError();
    this.retainers += 1;
    this.sync();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.retainers -= 1;
      this.sync();
    };
  }

  /**
   * Registers the function that re-reads everything. `force` is true when the
   * version alone cannot say the data is current (a reconnect, an `applied`
   * result without a version); false on a bump, which a read already at that
   * version may skip.
   */
  onRefetch(fn: (force: boolean) => void): void {
    this.refetchers.push(fn);
  }

  /** Re-read without moving the known version (an `applied` result without one). */
  forceRefetch(): void {
    this.fireRefetch(true);
  }

  /**
   * `push` comes from the stream, `result` from an own `applied` command.
   * A value that is not an integer >= 0, or not above the known one, is ignored.
   */
  observe(version: number | null, source: 'push' | 'result'): void {
    if (this.closed || version === null || !Number.isInteger(version) || version < 0) return;
    if (source === 'push' && (this.pushed === null || version > this.pushed)) {
      this.pushed = version;
      this.releaseWaiters();
    }
    if (this.known !== null && version <= this.known) return;
    this.known = version;
    for (const l of [...this.listeners]) {
      try {
        l.fn(version);
      } catch {
        this.deps.diagnostic({ kind: 'listener-threw' });
      }
    }
    this.fireRefetch(false);
  }

  /** Resolves `true` when a push reached `version`, `false` after `ms` or on close. */
  whenPushed(version: number, ms: number): Promise<boolean> {
    if (this.pushed !== null && this.pushed >= version) return Promise.resolve(true);
    if (this.closed) return Promise.resolve(false);
    return new Promise((resolve) => {
      const waiter: Waiter = { version, resolve, handle: null };
      waiter.handle = this.deps.timers.setTimeout(() => {
        this.waiters = this.waiters.filter((w) => w !== waiter);
        resolve(false);
      }, ms);
      this.waiters.push(waiter);
    });
  }

  close(): void {
    this.closed = true;
    this.listeners.clear();
    this.retainers = 0;
    this.stopStream();
    for (const w of this.waiters) {
      this.deps.timers.clearTimeout(w.handle);
      w.resolve(false);
    }
    this.waiters = [];
  }

  private fireRefetch(force: boolean): void {
    for (const fn of this.refetchers) fn(force);
  }

  private releaseWaiters(): void {
    const pushed = this.pushed;
    if (pushed === null) return;
    const ready = this.waiters.filter((w) => w.version <= pushed);
    this.waiters = this.waiters.filter((w) => w.version > pushed);
    for (const w of ready) {
      this.deps.timers.clearTimeout(w.handle);
      w.resolve(true);
    }
  }

  /** Opens on the first listener, closes when the last one leaves (rule 45). */
  private sync(): void {
    const wanted = !this.closed && this.listeners.size + this.retainers > 0;
    if (wanted) {
      if (this.source === null && this.reconnectTimer === null) this.open();
    } else {
      this.stopStream();
    }
  }

  private stopStream(): void {
    if (this.source !== null) {
      this.source.close();
      this.source = null;
    }
    if (this.reconnectTimer !== null) {
      this.deps.timers.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.attempt = 0;
    this.sawError = false;
  }

  private open(): void {
    const source = this.deps.eventSource(`${this.deps.baseUrl}${ROUTES.dataVersion}`);
    this.source = source;
    source.addEventListener('open', () => {
      if (this.source !== source) return;
      this.attempt = 0;
      if (this.sawError) {
        // Pushes sent during the gap are lost: read again, once.
        this.sawError = false;
        this.fireRefetch(true);
      }
    });
    source.addEventListener(SSE_DATA_VERSION_EVENT, (event) => {
      if (this.source !== source) return;
      const version = typeof event.data === 'string' ? parseDataVersionEvent(event.data) : null;
      if (version === null) {
        this.deps.diagnostic({ kind: 'malformed-data-version-event' });
        return;
      }
      this.observe(version, 'push');
    });
    source.addEventListener('error', () => {
      if (this.source !== source) return;
      this.sawError = true;
      if (source.readyState !== CLOSED) return; // the native reconnect is running
      source.close();
      this.source = null;
      this.reconnectTimer = this.deps.timers.setTimeout(() => {
        this.reconnectTimer = null;
        this.sync();
      }, this.delay(this.attempt));
      this.attempt += 1;
    });
  }
}
