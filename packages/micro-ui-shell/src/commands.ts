// Submit, look up, await and act on commands (rules 20–36). The shell
// transports and observes; the DataGuard decides. A rejection is a value.

import { type CommandAction, type CommandResult, type JsonObject, type JsonValue, type Message, type Mode, type PendingState, type QueueEntry } from './contracts';
import type { DataVersionHub } from './data-version';
import { ActionNotOfferedError, ClientDisposedError, MissingBasedOnError, ProtocolError, TransportError } from './errors';
import type { Http, Lifecycle } from './http';
import { wireDataCapability } from './identifier';
import { ROUTES, type LookupAnswer, type SubmitBody } from './protocol';
import { backoff, sleep, type Timers } from './timers';
import { parseLookupAnswer, parseSubmitAnswer } from './validate';
import { mapViolations, type MappedViolation, type ViolationMap } from './violations';

export interface SubmitInput {
  /** `<system>.<name>`, checked against the D/F identifier pattern. */
  dataCapability: string;
  /** The DataCapability's integer version >= 1; the wire id is `<dataCapability>@<version>`. */
  version: number;
  /** F's mode. Used locally for rule 26 only; the server reads it from the manifest. */
  mode: Mode;
  /** `target: {id}` goes on the wire only when `id` is set; `aggregate` is not sent. */
  target?: { aggregate: string; id?: string };
  /** Sent verbatim: no trim, no NFC, no default, no coercion (rule 21). */
  payload: JsonObject;
  basedOn?: { version: number; values?: JsonObject };
  /** One key for one form attempt. Without it, a fresh key is generated. */
  idempotencyKey?: string;
  signal?: AbortSignal;
}

export type CommandView =
  | {
      kind: 'pending';
      commandId: string;
      state: PendingState;
      entry: QueueEntry;
      messages: Message[];
      yourValue?: JsonValue;
      actions: CommandAction[];
    }
  | { kind: 'settled'; result: CommandResult }
  | { kind: 'not-found'; commandId: string };

export interface Submitted {
  commandId: string;
  partition: string;
  replayed: boolean;
  warnings: string[];
  idempotencyKey: string;
  first: Exclude<CommandView, { kind: 'not-found' }>;
}

export interface AwaitOptions {
  timeoutMs?: number;
  signal?: AbortSignal;
  violationMessages?: ViolationMap;
  onState?: (view: CommandView) => void;
}

export type AwaitOutcome =
  | { kind: 'settled'; result: CommandResult; violationMessages: MappedViolation[] }
  | { kind: 'still-pending'; commandId: string; reason: 'timeout' | 'aborted'; last: CommandView | null }
  | { kind: 'not-found'; commandId: string };

export interface CommandsConfig {
  poll: { initialMs: number; maxMs: number; factor: number };
  awaitTimeoutMs: number;
  submitRetries: number;
}

export interface CommandsDeps {
  http: Http;
  hub: DataVersionHub;
  timers: Timers;
  life: Lifecycle;
  newIdempotencyKey: () => string;
  config: CommandsConfig;
}

type Known = Exclude<CommandView, { kind: 'not-found' }>;

export class Commands {
  /** Per command: the first terminal result is final (rule 33), else the latest pending view. */
  private readonly seen = new Map<string, Known>();

  constructor(private readonly deps: CommandsDeps) {}

  async submit(input: SubmitInput): Promise<Submitted> {
    this.deps.life.assertLive();
    const dataCapability = wireDataCapability(input.dataCapability, input.version);
    if (input.mode === 'confirm_on_stale' && input.basedOn === undefined) {
      throw new MissingBasedOnError(input.dataCapability);
    }
    const idempotencyKey = input.idempotencyKey ?? this.deps.newIdempotencyKey();
    const body: SubmitBody = { dataCapability, payload: input.payload, idempotencyKey };
    if (input.target?.id !== undefined) body.target = { id: input.target.id };
    if (input.basedOn !== undefined) body.basedOn = input.basedOn;
    const serialised = JSON.stringify(body);

    const delay = backoff(this.deps.config.poll.initialMs, this.deps.config.poll.maxMs, this.deps.config.poll.factor);
    for (let attempt = 0; ; attempt += 1) {
      try {
        const res = await this.deps.http.request('POST', ROUTES.commands, { body: serialised, signal: input.signal });
        if (res.kind === 'not-found') throw new TransportError('http', 404, 'not-found');
        const answer = parseSubmitAnswer(res.body);
        const first = this.ingest(answer.commandId, answer);
        return {
          commandId: answer.commandId,
          partition: answer.partition,
          replayed: answer.replayed,
          warnings: answer.warnings,
          idempotencyKey,
          first,
        };
      } catch (e) {
        // Only a failure with no response is retried, and always with the same key.
        if (!(e instanceof TransportError) || e.kind !== 'network' || attempt >= this.deps.config.submitRetries) throw e;
        const signal = input.signal === undefined ? this.deps.life.signal : AbortSignal.any([this.deps.life.signal, input.signal]);
        if (!(await sleep(this.deps.timers, delay(attempt), signal))) {
          if (this.deps.life.signal.aborted) throw new ClientDisposedError();
          throw e;
        }
      }
    }
  }

  async lookup(commandId: string, opts?: { signal?: AbortSignal }): Promise<CommandView> {
    this.deps.life.assertLive();
    const res = await this.deps.http.request('GET', ROUTES.command(commandId), { signal: opts?.signal });
    if (res.kind === 'not-found') return { kind: 'not-found', commandId };
    return this.ingest(commandId, parseLookupAnswer(res.body));
  }

  async act(commandId: string, action: 'confirm_overwrite' | 'cancel', opts?: { signal?: AbortSignal }): Promise<CommandView> {
    this.deps.life.assertLive();
    const view = this.seen.get(commandId);
    if (view?.kind !== 'pending' || !view.actions.includes(action)) throw new ActionNotOfferedError(commandId, action);
    const path = action === 'confirm_overwrite' ? ROUTES.confirm(commandId) : ROUTES.cancel(commandId);
    const res = await this.deps.http.request('POST', path, { signal: opts?.signal });
    if (res.kind === 'not-found') return { kind: 'not-found', commandId };
    return this.ingest(commandId, parseLookupAnswer(res.body));
  }

  async awaitResult(commandId: string, opts: AwaitOptions = {}): Promise<AwaitOutcome> {
    this.deps.life.assertLive();
    const settled = (result: CommandResult): AwaitOutcome => ({
      kind: 'settled',
      result,
      violationMessages: mapViolations(result, opts.violationMessages ?? {}),
    });
    const cached = this.seen.get(commandId);
    if (cached?.kind === 'settled') return settled(cached.result);

    // One controller ends the wait: caller abort, timeout or dispose.
    const stop = new AbortController();
    let timedOut = false;
    const timeoutMs = opts.timeoutMs ?? this.deps.config.awaitTimeoutMs;
    const timer = this.deps.timers.setTimeout(() => {
      timedOut = true;
      stop.abort();
    }, timeoutMs);
    const sources = [this.deps.life.signal, ...(opts.signal === undefined ? [] : [opts.signal])];
    const link = AbortSignal.any(sources);
    const onLink = (): void => stop.abort();
    if (link.aborted) stop.abort();
    else link.addEventListener('abort', onLink, { once: true });

    const delay = backoff(this.deps.config.poll.initialMs, this.deps.config.poll.maxMs, this.deps.config.poll.factor);
    let last: CommandView | null = cached ?? null;
    let signature = last === null ? '' : signatureOf(last);
    const pending = (): AwaitOutcome => ({
      kind: 'still-pending',
      commandId,
      reason: timedOut ? 'timeout' : 'aborted',
      last,
    });
    try {
      for (let attempt = 0; !stop.signal.aborted; attempt += 1) {
        let view: CommandView;
        try {
          view = await this.lookup(commandId, { signal: stop.signal });
        } catch (e) {
          if (stop.signal.aborted || e instanceof ClientDisposedError) return pending();
          throw e;
        }
        if (view.kind === 'not-found') return view;
        if (view.kind === 'settled') return settled(view.result);
        last = view;
        const sig = signatureOf(view);
        if (sig !== signature) {
          signature = sig;
          opts.onState?.(view);
        }
        await sleep(this.deps.timers, delay(attempt), stop.signal);
      }
      return pending();
    } finally {
      this.deps.timers.clearTimeout(timer);
      link.removeEventListener('abort', onLink);
    }
  }

  /** Merges an answer into what the client knows; the first terminal result wins. */
  private ingest(commandId: string, answer: LookupAnswer): Known {
    const known = this.seen.get(commandId);
    if (known?.kind === 'settled') return known;

    if ('result' in answer) {
      const view: Known = { kind: 'settled', result: answer.result };
      this.seen.set(commandId, view);
      if (answer.result.status === 'applied') {
        const { dataVersion } = answer.result;
        if (dataVersion === null) this.deps.hub.forceRefetch();
        else this.deps.hub.observe(dataVersion, 'result');
      }
      return view;
    }

    const { entry, messages } = answer;
    if (!isPending(entry.state)) throw new ProtocolError('command lookup', 'pending answer with a terminal state');
    const offered = [...messages].reverse().find((m) => m.actions !== undefined);
    const withValue = [...messages].reverse().find((m) => m.yourValue !== undefined);
    const yourValue = entry.yourValue ?? withValue?.yourValue;
    const view: Known = {
      kind: 'pending',
      commandId,
      state: entry.state,
      entry,
      messages,
      actions: offered?.actions ?? [],
      ...(yourValue === undefined ? {} : { yourValue }),
    };
    this.seen.set(commandId, view);
    return view;
  }
}

function isPending(state: QueueEntry['state']): state is PendingState {
  return state !== 'applied' && state !== 'rejected' && state !== 'expired' && state !== 'cancelled';
}

function signatureOf(view: CommandView): string {
  if (view.kind === 'pending') return `${view.state}|${view.actions.join(',')}`;
  return view.kind === 'settled' ? `settled:${view.result.status}` : 'not-found';
}
