import { Commands, type AwaitOptions, type AwaitOutcome, type CommandView, type SubmitInput, type Submitted } from './commands';
import type { JsonObject } from './contracts';
import { DataVersionHub, type EventSourceLike, type Unsubscribe } from './data-version';
import { assertLoopbackBaseUrl } from './base-url';
import { ClientDisposedError } from './errors';
import { createHttp, type Lifecycle } from './http';
import { Reads, type ReadOutcome, type WatchEvent } from './reads';
import { defaultTimers, type Diagnostic, type Timers } from './timers';

export interface ShellClientOptions {
  /** Loopback only, else `NonLoopbackBaseUrlError`. No default. */
  baseUrl: string;
  /** Required: never read from a global. */
  fetch: typeof globalThis.fetch;
  /** Required factory, e.g. `(url) => new EventSource(url)`. */
  eventSource: (url: string) => EventSourceLike;
  timers?: Timers;
  newIdempotencyKey?: () => string;
  /** The only way diagnostics leave the shell. Nothing is written anywhere by default. */
  onDiagnostic?: (d: Diagnostic) => void;
  poll?: { initialMs?: number; maxMs?: number; factor?: number };
  awaitTimeoutMs?: number;
  submitRetries?: number;
  reconnect?: { initialMs?: number; maxMs?: number };
  staleRead?: { retries?: number; delayMs?: number; pushWaitMs?: number };
}

export interface ShellClient {
  read<T = unknown>(capability: string, variables?: JsonObject, opts?: { signal?: AbortSignal }): Promise<ReadOutcome<T>>;
  watch<T = unknown>(capability: string, variables: JsonObject, listener: (e: WatchEvent<T>) => void): Unsubscribe;
  submit(input: SubmitInput): Promise<Submitted>;
  lookup(commandId: string, opts?: { signal?: AbortSignal }): Promise<CommandView>;
  awaitResult(commandId: string, opts?: AwaitOptions): Promise<AwaitOutcome>;
  act(commandId: string, action: 'confirm_overwrite' | 'cancel', opts?: { signal?: AbortSignal }): Promise<CommandView>;
  onDataVersion(listener: (version: number) => void): Unsubscribe;
  knownDataVersion(): number | null;
  dispose(): void;
}

export function createShellClient(options: ShellClientOptions): ShellClient {
  const baseUrl = assertLoopbackBaseUrl(options.baseUrl);
  const fetchFn = options.fetch;
  const eventSourceFactory = options.eventSource;
  const timers = options.timers ?? defaultTimers;
  const diagnostic = (d: Diagnostic): void => {
    try {
      options.onDiagnostic?.(d);
    } catch {
      // A throwing hook must not break the shell.
    }
  };

  const dispose = new AbortController();
  const life: Lifecycle = {
    signal: dispose.signal,
    assertLive: () => {
      if (dispose.signal.aborted) throw new ClientDisposedError();
    },
  };

  const hub = new DataVersionHub({
    baseUrl,
    eventSource: (url) => eventSourceFactory(url),
    timers,
    reconnect: { initialMs: options.reconnect?.initialMs ?? 500, maxMs: options.reconnect?.maxMs ?? 30_000 },
    diagnostic,
  });
  const http = createHttp(baseUrl, (input, init) => fetchFn(input, init), life);
  const commands = new Commands({
    http,
    hub,
    timers,
    life,
    newIdempotencyKey: options.newIdempotencyKey ?? (() => globalThis.crypto.randomUUID()),
    config: {
      poll: {
        initialMs: options.poll?.initialMs ?? 250,
        maxMs: options.poll?.maxMs ?? 5_000,
        factor: options.poll?.factor ?? 2,
      },
      awaitTimeoutMs: options.awaitTimeoutMs ?? 30_000,
      submitRetries: options.submitRetries ?? 2,
    },
  });
  const reads = new Reads({
    http,
    hub,
    timers,
    life,
    diagnostic,
    config: {
      staleRead: {
        retries: options.staleRead?.retries ?? 5,
        delayMs: options.staleRead?.delayMs ?? 200,
        pushWaitMs: options.staleRead?.pushWaitMs ?? 5_000,
      },
    },
  });

  return {
    read: (capability, variables, opts) => reads.read(capability, variables, opts),
    watch: (capability, variables, listener) => reads.watch(capability, variables, listener),
    submit: (input) => commands.submit(input),
    lookup: (commandId, opts) => commands.lookup(commandId, opts),
    awaitResult: (commandId, opts) => commands.awaitResult(commandId, opts),
    act: (commandId, action, opts) => commands.act(commandId, action, opts),
    onDataVersion: (listener) => {
      life.assertLive();
      return hub.addListener(listener);
    },
    knownDataVersion: () => hub.knownVersion(),
    dispose: () => {
      if (dispose.signal.aborted) return;
      dispose.abort();
      reads.dispose();
      hub.close();
    },
  };
}
