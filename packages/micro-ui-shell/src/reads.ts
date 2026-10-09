// Reads by identifier (rules 13–19) and live reads that refetch by
// themselves when the dataVersion moves (rules 46–51). A read sends an
// identifier and variables, never query text; the shell adds no campaign id,
// no filter and no sort.

import type { JsonObject, JsonValue } from './contracts';
import type { DataVersionHub, Unsubscribe } from './data-version';
import { ClientDisposedError, ProtocolError, TransportError } from './errors';
import type { Http, Lifecycle } from './http';
import { assertIdentifier } from './identifier';
import { ROUTES, type ReadBody } from './protocol';
import { sleep, type Diagnostic, type Timers } from './timers';
import { parseReadAnswer } from './validate';

export type ReadOutcome<T> = { kind: 'found'; data: T; asOf: number | null } | { kind: 'not-found' };

export type WatchEvent<T> =
  | { kind: 'data'; data: T; asOf: number | null }
  | { kind: 'not-found' }
  | { kind: 'stale'; asOf: number; wanted: number }
  | { kind: 'error'; error: TransportError | ProtocolError };

export interface ReadsConfig {
  staleRead: { retries: number; delayMs: number; pushWaitMs: number };
}

export interface ReadsDeps {
  http: Http;
  hub: DataVersionHub;
  timers: Timers;
  life: Lifecycle;
  diagnostic: (d: Diagnostic) => void;
  config: ReadsConfig;
}

interface Entry {
  capability: string;
  variables: JsonObject;
  listeners: Set<(e: WatchEvent<unknown>) => void>;
  last: WatchEvent<unknown> | undefined;
  inflight: boolean;
  dirty: boolean;
  /** The version the next presented data must be at least (the known one when scheduled). */
  wanted: number | null;
  abort: AbortController;
}

/** Keys with the same content in any order serialise the same. */
export function stableStringify(value: JsonValue): string {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const keys = Object.keys(value).sort();
    return `{${keys.map((k) => `${JSON.stringify(k)}:${stableStringify(value[k] as JsonValue)}`).join(',')}}`;
  }
  return JSON.stringify(value);
}

export class Reads {
  private readonly entries = new Map<string, Entry>();
  private flushTimer: unknown = null;

  constructor(private readonly deps: ReadsDeps) {
    deps.hub.onRefetch(() => this.markAllDirty());
  }

  async read<T>(capability: string, variables: JsonObject = {}, opts?: { signal?: AbortSignal }): Promise<ReadOutcome<T>> {
    this.deps.life.assertLive();
    assertIdentifier(capability);
    const body: ReadBody = { variables };
    const res = await this.deps.http.request('POST', ROUTES.capability(capability), {
      body: JSON.stringify(body),
      signal: opts?.signal,
    });
    if (res.kind === 'not-found') return { kind: 'not-found' };
    const answer = parseReadAnswer(res.body);
    return { kind: 'found', data: answer.data as T, asOf: answer.asOf };
  }

  watch<T>(capability: string, variables: JsonObject, listener: (e: WatchEvent<T>) => void): Unsubscribe {
    this.deps.life.assertLive();
    assertIdentifier(capability);
    // Entries are keyed by identifier plus the full variables (rule 16).
    const key = `${capability}\u0000${stableStringify(variables)}`;
    let entry = this.entries.get(key);
    const fresh = entry === undefined;
    if (entry === undefined) {
      entry = {
        capability,
        variables,
        listeners: new Set(),
        last: undefined,
        inflight: false,
        dirty: false,
        wanted: null,
        abort: new AbortController(),
      };
      this.entries.set(key, entry);
    }
    const wrapped = listener as (e: WatchEvent<unknown>) => void;
    entry.listeners.add(wrapped);
    const release = this.deps.hub.retain();
    if (fresh) this.markDirty(entry);
    else if (entry.last !== undefined) this.emitTo(wrapped, entry.last);

    const owner = entry;
    let done = false;
    return () => {
      if (done) return;
      done = true;
      owner.listeners.delete(wrapped);
      release();
      if (owner.listeners.size === 0) {
        owner.abort.abort();
        if (this.entries.get(key) === owner) this.entries.delete(key);
      }
    };
  }

  dispose(): void {
    if (this.flushTimer !== null) this.deps.timers.clearTimeout(this.flushTimer);
    this.flushTimer = null;
    for (const entry of this.entries.values()) {
      entry.abort.abort();
      entry.listeners.clear();
    }
    this.entries.clear();
  }

  private markAllDirty(): void {
    for (const entry of this.entries.values()) this.markDirty(entry);
  }

  /** Every bump in one tick lands in a single flush, at the latest version. */
  private markDirty(entry: Entry): void {
    entry.dirty = true;
    entry.wanted = this.deps.hub.knownVersion();
    if (this.flushTimer !== null) return;
    this.flushTimer = this.deps.timers.setTimeout(() => {
      this.flushTimer = null;
      for (const e of [...this.entries.values()]) {
        if (e.dirty && !e.inflight) void this.run(e);
      }
    }, 0);
  }

  private async run(entry: Entry): Promise<void> {
    entry.dirty = false;
    entry.inflight = true;
    const floor = entry.wanted;
    let satisfied: number | null = null; // the asOf the presented data carried
    try {
      const event = await this.fetchAtLeast(entry, floor);
      if (event !== null) {
        if (event.kind === 'data') {
          satisfied = event.asOf;
          entry.last = event;
        } else if (event.kind === 'not-found') {
          entry.last = event;
        }
        this.emit(entry, event);
      }
    } catch (e) {
      if (e instanceof TransportError || e instanceof ProtocolError) {
        this.emit(entry, { kind: 'error', error: e });
      } else if (!(e instanceof ClientDisposedError) && !entry.abort.signal.aborted) {
        this.deps.diagnostic({ kind: 'read-failed' });
      }
    } finally {
      entry.inflight = false;
    }
    // A bump during the fetch needs another one, unless the answer already covers it.
    if (entry.dirty && satisfied !== null && entry.wanted !== null && satisfied >= entry.wanted) {
      entry.dirty = false;
    }
    if (entry.dirty && !entry.abort.signal.aborted && this.entries.size > 0) {
      this.markDirty(entry);
    }
  }

  /** Never presents data older than `floor` as fresh (rule 48). `null`: the entry is gone. */
  private async fetchAtLeast(entry: Entry, floor: number | null): Promise<WatchEvent<unknown> | null> {
    const { retries, delayMs, pushWaitMs } = this.deps.config.staleRead;
    const signal = AbortSignal.any([this.deps.life.signal, entry.abort.signal]);
    let waitedForPush = false;
    for (let attempt = 0; ; attempt += 1) {
      const outcome = await this.read<unknown>(entry.capability, entry.variables, { signal });
      if (outcome.kind === 'not-found') return { kind: 'not-found' };
      if (floor === null) return { kind: 'data', data: outcome.data, asOf: outcome.asOf };

      if (outcome.asOf === null) {
        // No version on the read: wait for the push to reach the floor, then read once more.
        if (waitedForPush || (this.deps.hub.pushedVersion() ?? -1) >= floor) {
          return { kind: 'data', data: outcome.data, asOf: null };
        }
        waitedForPush = true;
        await this.deps.hub.whenPushed(floor, pushWaitMs);
        if (signal.aborted) return null;
        continue;
      }
      if (outcome.asOf >= floor) return { kind: 'data', data: outcome.data, asOf: outcome.asOf };

      if (attempt >= retries) return { kind: 'stale', asOf: outcome.asOf, wanted: floor };
      if (!(await sleep(this.deps.timers, delayMs, signal))) return null;
    }
  }

  private emit(entry: Entry, event: WatchEvent<unknown>): void {
    for (const l of [...entry.listeners]) this.emitTo(l, event);
  }

  private emitTo(listener: (e: WatchEvent<unknown>) => void, event: WatchEvent<unknown>): void {
    try {
      listener(event);
    } catch {
      this.deps.diagnostic({ kind: 'listener-threw' });
    }
  }
}
