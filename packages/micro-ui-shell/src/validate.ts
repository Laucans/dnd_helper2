// Boundary parsers (rule 12): every response and every SSE payload is checked
// against its contract shape before the rest of the shell sees it.

import {
  COMMAND_ACTIONS,
  G_FIELDS,
  H_FIELDS,
  MESSAGE_KINDS,
  QUEUE_STATES,
  TERMINAL_STATUSES,
  type CommandAction,
  type CommandResult,
  type JsonObject,
  type JsonValue,
  type Message,
  type MessageKind,
  type QueueEntry,
  type QueueState,
  type TerminalStatus,
} from './contracts';
import { ProtocolError } from './errors';
import type { DataVersionEvent, LookupAnswer, ReadAnswer, SubmitAnswer } from './protocol';

type Obj = Record<string, unknown>;

function isObject(x: unknown): x is Obj {
  return typeof x === 'object' && x !== null && !Array.isArray(x);
}

function obj(x: unknown, where: string): Obj {
  if (!isObject(x)) throw new ProtocolError(where, 'expected an object');
  return x;
}

function isVersion(x: unknown): x is number {
  return typeof x === 'number' && Number.isInteger(x) && x >= 0;
}

function isStringArray(x: unknown): x is string[] {
  return Array.isArray(x) && x.every((s) => typeof s === 'string');
}

function oneOf<T extends string>(list: readonly T[], x: unknown): x is T {
  return typeof x === 'string' && (list as readonly string[]).includes(x);
}

function isJson(x: unknown): x is JsonValue {
  if (x === null || typeof x === 'string' || typeof x === 'boolean') return true;
  if (typeof x === 'number') return Number.isFinite(x);
  if (Array.isArray(x)) return x.every(isJson);
  return isObject(x) && Object.values(x).every(isJson);
}

export function parseCommandResult(x: unknown, where = 'command result'): CommandResult {
  const o = obj(x, where);
  const keys = Object.keys(o);
  if (keys.length !== H_FIELDS.length || !H_FIELDS.every((k) => k in o)) {
    throw new ProtocolError(where, 'fields differ from contract H');
  }
  if (typeof o['commandId'] !== 'string') throw new ProtocolError(where, 'commandId');
  if (!oneOf<TerminalStatus>(TERMINAL_STATUSES, o['status'])) throw new ProtocolError(where, 'unknown status');
  if (o['dataVersion'] !== null && !isVersion(o['dataVersion'])) throw new ProtocolError(where, 'dataVersion');
  if (!isStringArray(o['violations'])) throw new ProtocolError(where, 'violations');
  if (o['reviewId'] !== null && typeof o['reviewId'] !== 'string') throw new ProtocolError(where, 'reviewId');
  return {
    commandId: o['commandId'],
    status: o['status'],
    dataVersion: o['dataVersion'],
    violations: o['violations'],
    reviewId: o['reviewId'],
  };
}

export function parseQueueEntry(x: unknown, where = 'queue entry'): QueueEntry {
  const o = obj(x, where);
  for (const k of Object.keys(o)) {
    if (!(G_FIELDS as readonly string[]).includes(k)) throw new ProtocolError(where, `unexpected field ${k}`);
  }
  for (const k of ['command', 'dataCapability', 'by', 'partition'] as const) {
    if (typeof o[k] !== 'string') throw new ProtocolError(where, k);
  }
  if (!isVersion(o['position'])) throw new ProtocolError(where, 'position');
  const basedOn = obj(o['basedOn'], where);
  if (!isVersion(basedOn['version'])) throw new ProtocolError(where, 'basedOn.version');
  if (!oneOf<QueueState>(QUEUE_STATES, o['state'])) throw new ProtocolError(where, 'unknown state');
  if (o['yourValue'] !== undefined && !isJson(o['yourValue'])) throw new ProtocolError(where, 'yourValue');
  // The contract's remaining fields are passed through as the server sent them.
  return o as unknown as QueueEntry;
}

/** A message the shell cannot use is dropped, never thrown (rule 12). */
export function parseMessage(x: unknown): Message | null {
  if (!isObject(x)) return null;
  if (!oneOf<MessageKind>(MESSAGE_KINDS, x['message'])) return null;
  const to = x['to'];
  if (!isStringArray(to) || to.length === 0) return null;
  const message: Message = { ...x, message: x['message'], to };
  if ('actions' in x) {
    const actions = x['actions'];
    if (Array.isArray(actions)) {
      // An action this version does not know is ignored, not an error.
      message.actions = actions.filter((a): a is CommandAction => oneOf(COMMAND_ACTIONS, a));
    } else {
      delete message.actions;
    }
  }
  return message;
}

function parseMessages(x: unknown, where: string): Message[] {
  if (!Array.isArray(x)) throw new ProtocolError(where, 'messages is not an array');
  const out: Message[] = [];
  for (const m of x) {
    const parsed = parseMessage(m);
    if (parsed !== null) out.push(parsed);
  }
  return out;
}

/** Exactly one of `{entry, messages}` / `{result}`. */
export function parseLookupAnswer(x: unknown, where = 'command lookup'): LookupAnswer {
  const o = obj(x, where);
  const hasResult = 'result' in o;
  const hasEntry = 'entry' in o;
  if (hasResult === hasEntry) throw new ProtocolError(where, 'expected exactly one of result / entry');
  if (hasResult) return { result: parseCommandResult(o['result'], where) };
  return { entry: parseQueueEntry(o['entry'], where), messages: parseMessages(o['messages'], where) };
}

export function parseSubmitAnswer(x: unknown): SubmitAnswer {
  const where = 'submit answer';
  const o = obj(x, where);
  if (typeof o['commandId'] !== 'string') throw new ProtocolError(where, 'commandId');
  if (typeof o['partition'] !== 'string') throw new ProtocolError(where, 'partition');
  if (typeof o['replayed'] !== 'boolean') throw new ProtocolError(where, 'replayed');
  if (!isStringArray(o['warnings'])) throw new ProtocolError(where, 'warnings');
  const lookup = parseLookupAnswer(o, where);
  return {
    ...lookup,
    commandId: o['commandId'],
    partition: o['partition'],
    replayed: o['replayed'],
    warnings: o['warnings'],
  };
}

export function parseReadAnswer(x: unknown): ReadAnswer {
  const where = 'read answer';
  const o = obj(x, where);
  if (!('data' in o) || !isJson(o['data'])) throw new ProtocolError(where, 'data');
  const asOf = o['asOf'];
  if (asOf !== undefined && asOf !== null && !isVersion(asOf)) throw new ProtocolError(where, 'asOf');
  return { asOf: asOf ?? null, data: o['data'] };
}

/** `data:` of a `dataVersion` event; `null` when it cannot be used (dropped, no refetch). */
export function parseDataVersionEvent(data: string): number | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(data);
  } catch {
    return null;
  }
  if (!isObject(parsed)) return null;
  const v = (parsed as Partial<Record<keyof DataVersionEvent, unknown>>).dataVersion;
  return isVersion(v) ? v : null;
}

/** Narrow an unknown JSON body to an object, for the error-id lookup. */
export function asJsonObject(x: unknown): JsonObject | null {
  return isObject(x) && isJson(x) ? x : null;
}
