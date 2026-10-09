// A hand-written ShellClient: it records every call and answers from a script.
// No socket, no timer, no server. All data is invented.

import {
  ActionNotOfferedError,
  mapViolations,
  type AwaitOptions,
  type AwaitOutcome,
  type CommandView,
  type JsonObject,
  type JsonValue,
  type Message,
  type ReadOutcome,
  type ShellClient,
  type SubmitInput,
  type Submitted,
  type TerminalStatus,
  type Unsubscribe,
  type WatchEvent,
} from '@dnd-helper/micro-ui-shell';
import type { PcRow } from '../src/identifiers';
import { PJ_VIOLATIONS } from '../src/violations';

export interface Deferred<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(error: unknown): void;
}

export function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Lets every queued promise callback run. */
export async function flush(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await new Promise<void>((r) => setImmediate(r));
}

export type SubmitStep = Submitted | Error | (() => Promise<Submitted>);
export type AwaitStep = (opts: AwaitOptions) => Promise<AwaitOutcome>;

export class FakeShell implements ShellClient {
  readonly watches: { capability: string; variables: JsonObject; listener: (e: WatchEvent<never>) => void; live: boolean }[] = [];
  readonly submits: SubmitInput[] = [];
  readonly awaits: { id: string; opts: AwaitOptions }[] = [];
  readonly acts: { id: string; action: string }[] = [];
  readonly lookups: string[] = [];
  readonly reads: string[] = [];
  readonly submitQueue: SubmitStep[] = [];
  readonly awaitScripts = new Map<string, AwaitStep[]>();
  actHandler: (id: string, action: 'confirm_overwrite' | 'cancel') => Promise<CommandView> = (id, action) =>
    Promise.reject(new ActionNotOfferedError(id, action));
  lookupHandler: (id: string) => Promise<CommandView> = (id) => Promise.resolve({ kind: 'not-found', commandId: id });
  known: number | null = null;
  disposed = false;

  read<T = unknown>(capability: string): Promise<ReadOutcome<T>> {
    this.reads.push(capability);
    return Promise.resolve({ kind: 'not-found' });
  }

  watch<T = unknown>(capability: string, variables: JsonObject, listener: (e: WatchEvent<T>) => void): Unsubscribe {
    const w = { capability, variables, listener: listener as (e: WatchEvent<never>) => void, live: true };
    this.watches.push(w);
    return () => {
      w.live = false;
    };
  }

  /** Pushes a watch event to every live watcher. */
  emit(e: WatchEvent<PcRow[]>): void {
    for (const w of this.watches) if (w.live) w.listener(e as WatchEvent<never>);
  }

  async submit(input: SubmitInput): Promise<Submitted> {
    this.submits.push(input);
    const step = this.submitQueue.shift();
    if (step === undefined) throw new Error('FakeShell: no scripted submit');
    if (step instanceof Error) throw step;
    return typeof step === 'function' ? step() : step;
  }

  lookup(commandId: string): Promise<CommandView> {
    this.lookups.push(commandId);
    return this.lookupHandler(commandId);
  }

  awaitResult(commandId: string, opts: AwaitOptions = {}): Promise<AwaitOutcome> {
    this.awaits.push({ id: commandId, opts });
    const step = this.awaitScripts.get(commandId)?.shift();
    if (step !== undefined) return step(opts);
    // Default: a command that never ends; the wait ends only when the caller aborts it.
    return new Promise((resolve) => {
      const done = (): void => {
        resolve({ kind: 'still-pending', commandId, reason: 'aborted', last: null });
      };
      if (opts.signal?.aborted === true) done();
      else opts.signal?.addEventListener('abort', done, { once: true });
    });
  }

  script(commandId: string, ...steps: AwaitStep[]): void {
    this.awaitScripts.set(commandId, [...(this.awaitScripts.get(commandId) ?? []), ...steps]);
  }

  act(commandId: string, action: 'confirm_overwrite' | 'cancel'): Promise<CommandView> {
    this.acts.push({ id: commandId, action });
    return this.actHandler(commandId, action);
  }

  onDataVersion(): Unsubscribe {
    return () => undefined;
  }

  knownDataVersion(): number | null {
    return this.known;
  }

  dispose(): void {
    this.disposed = true;
  }
}

// --- builders ---------------------------------------------------------------

export const row = (id: string, name: string, cls = 'Guerrier', level = 3): PcRow => ({ id, name, class: cls, level });

export function settledView(commandId: string, status: TerminalStatus, violations: string[] = [], dataVersion?: number | null): CommandView {
  return {
    kind: 'settled',
    result: {
      commandId,
      status,
      dataVersion: dataVersion === undefined ? (status === 'applied' ? 9 : null) : dataVersion,
      violations,
      reviewId: null,
    },
  };
}

export function settledOutcome(commandId: string, status: TerminalStatus, violations: string[] = [], dataVersion?: number | null): AwaitOutcome {
  const view = settledView(commandId, status, violations, dataVersion);
  if (view.kind !== 'settled') throw new Error('unreachable');
  return { kind: 'settled', result: view.result, violationMessages: mapViolations(view.result, PJ_VIOLATIONS) };
}

export function pendingView(
  commandId: string,
  state: 'queued' | 'awaiting_confirmation' | 'confirmed',
  extra: { yourValue?: JsonValue; messages?: Message[]; actions?: ('confirm_overwrite' | 'cancel' | 'edit')[]; projection?: JsonObject } = {},
): CommandView {
  return {
    kind: 'pending',
    commandId,
    state,
    entry: {
      command: commandId,
      dataCapability: 'x',
      by: 'gm',
      partition: 'PJ/p',
      position: 1,
      basedOn: { version: 1 },
      state,
      ...(extra.yourValue === undefined ? {} : { yourValue: extra.yourValue }),
      ...(extra.projection === undefined ? {} : { projection: extra.projection }),
    },
    messages: extra.messages ?? [],
    ...(extra.yourValue === undefined ? {} : { yourValue: extra.yourValue }),
    actions: extra.actions ?? [],
  };
}

export function submitted(first: CommandView, commandId: string, key = 'k'): Submitted {
  return { commandId, partition: 'p', replayed: false, warnings: [], idempotencyKey: key, first: first as Exclude<CommandView, { kind: 'not-found' }> };
}

export const queued = (commandId: string): Submitted => submitted(pendingView(commandId, 'queued'), commandId);
