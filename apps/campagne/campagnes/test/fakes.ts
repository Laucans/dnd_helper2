// Hermetic doubles for the controller: a scriptable shell port, a recording
// setContext and a scripted confirm. No socket, no clock, no DOM.

import {
  mapViolations,
  type AwaitOptions,
  type AwaitOutcome,
  type CommandResult,
  type JsonObject,
  type PendingState,
  type SubmitInput,
  type Submitted,
  type TerminalStatus,
  type WatchEvent,
} from '@dnd-helper/micro-ui-shell';
import { createCampagnesController, type Campagne, type CampagnesController, type ShellPort } from '../src/controller';
import { VIOLATIONS } from '../src/violations';

export const flush = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

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

export const CMD = '11111111-1111-4111-8111-111111111111';

export const pendingView = (state: PendingState, commandId = CMD): Submitted['first'] =>
  ({ kind: 'pending', commandId, state, entry: {}, messages: [], actions: [] }) as unknown as Submitted['first'];

export function submitted(first: Submitted['first'], extra: Partial<Submitted> = {}): Submitted {
  return { commandId: CMD, partition: 'Campagne/x', replayed: false, warnings: [], idempotencyKey: 'k', first, ...extra };
}

export function result(status: TerminalStatus, violations: string[] = [], dataVersion: number | null = status === 'applied' ? 7 : null): CommandResult {
  return { commandId: CMD, status, dataVersion, violations, reviewId: null };
}

export const settledView = (status: TerminalStatus, violations: string[] = [], dataVersion?: number | null): Submitted['first'] => ({
  kind: 'settled',
  result: result(status, violations, dataVersion),
});

export function settledOutcome(status: TerminalStatus, violations: string[] = [], dataVersion?: number | null): AwaitOutcome {
  const r = result(status, violations, dataVersion);
  return { kind: 'settled', result: r, violationMessages: mapViolations(r, VIOLATIONS) };
}

type SubmitReply = Submitted | Error | Promise<Submitted>;
type AwaitReply = AwaitOutcome | Error | Promise<AwaitOutcome> | ((opts: AwaitOptions) => AwaitOutcome | Promise<AwaitOutcome>);

export class FakeShell implements ShellPort {
  readonly watches: { capability: string; variables: JsonObject }[] = [];
  unsubscribed = 0;
  readonly submits: SubmitInput[] = [];
  readonly awaits: { commandId: string; opts: AwaitOptions }[] = [];
  private listener: ((event: WatchEvent<unknown>) => void) | null = null;
  private readonly submitQueue: SubmitReply[] = [];
  private readonly awaitQueue: AwaitReply[] = [];

  replySubmit(...replies: SubmitReply[]): this {
    this.submitQueue.push(...replies);
    return this;
  }

  replyAwait(...replies: AwaitReply[]): this {
    this.awaitQueue.push(...replies);
    return this;
  }

  emit(event: WatchEvent<unknown>): void {
    this.listener?.(event);
  }

  data(rows: readonly Campagne[], asOf: number | null = null): void {
    this.emit({ kind: 'data', data: rows, asOf });
  }

  watch<T = unknown>(capability: string, variables: JsonObject, listener: (event: WatchEvent<T>) => void): () => void {
    this.watches.push({ capability, variables });
    this.listener = listener as (event: WatchEvent<unknown>) => void;
    return () => {
      this.unsubscribed += 1;
      this.listener = null;
    };
  }

  async submit(input: SubmitInput): Promise<Submitted> {
    this.submits.push(input);
    const reply = this.submitQueue.shift();
    if (reply === undefined) throw new Error('FakeShell: no submit reply scripted');
    const resolved = await reply;
    if (resolved instanceof Error) throw resolved;
    return { ...resolved, idempotencyKey: input.idempotencyKey ?? resolved.idempotencyKey };
  }

  async awaitResult(commandId: string, opts: AwaitOptions = {}): Promise<AwaitOutcome> {
    this.awaits.push({ commandId, opts });
    const reply = this.awaitQueue.shift();
    if (reply === undefined) throw new Error('FakeShell: no await reply scripted');
    const resolved = await (typeof reply === 'function' ? reply(opts) : reply);
    if (resolved instanceof Error) throw resolved;
    return resolved;
  }
}

export interface Rig {
  shell: FakeShell;
  controller: CampagnesController;
  contexts: (string | null)[];
  confirms: string[];
  /** The next answers of the confirmation dialog; `true` when none is left. */
  answers: (boolean | Promise<boolean>)[];
  keys: string[];
}

export function rig(): Rig {
  const shell = new FakeShell();
  const contexts: (string | null)[] = [];
  const confirms: string[] = [];
  const answers: (boolean | Promise<boolean>)[] = [];
  const keys: string[] = [];
  let n = 0;
  const controller = createCampagnesController({
    shell,
    setContext: (name, value) => {
      expectContextName(name);
      contexts.push(value);
    },
    confirm: (message) => {
      confirms.push(message);
      return answers.shift() ?? true;
    },
    newIdempotencyKey: () => {
      const key = `key-${String((n += 1))}`;
      keys.push(key);
      return key;
    },
  });
  return { shell, controller, contexts, confirms, answers, keys };
}

function expectContextName(name: string): void {
  if (name !== 'campagne') throw new Error(`setContext called with "${name}"`);
}

export const row = (id: string, name: string): Campagne => ({ id, name });
