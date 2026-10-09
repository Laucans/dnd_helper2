// TypeScript mirrors of the contracts the shell speaks: H (command result),
// G (queue entry), L (message) and the fields of F (DataCapability) it uses.
// Field for field, nothing added: `test/contracts.test.ts` compares every
// constant below with the schema files in `contracts/`.

export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

// --- H: command result -----------------------------------------------------

export const TERMINAL_STATUSES = ['applied', 'rejected', 'expired', 'cancelled'] as const;
export type TerminalStatus = (typeof TERMINAL_STATUSES)[number];

export const H_FIELDS = ['commandId', 'status', 'dataVersion', 'violations', 'reviewId'] as const;

export interface CommandResult {
  commandId: string;
  status: TerminalStatus;
  dataVersion: number | null;
  violations: string[];
  reviewId: string | null;
}

// --- G: queue entry --------------------------------------------------------

export const QUEUE_STATES = [
  'queued',
  'awaiting_confirmation',
  'confirmed',
  'awaiting_review',
  'parked',
  'applied',
  'rejected',
  'cancelled',
  'expired',
] as const;
export type QueueState = (typeof QUEUE_STATES)[number];
export type PendingState = Exclude<QueueState, TerminalStatus>;

export const G_FIELDS = [
  'command',
  'dataCapability',
  'by',
  'partition',
  'position',
  'basedOn',
  'projection',
  'yourValue',
  'state',
  'confirmation',
  'parked',
  'requeuedFrom',
] as const;

export interface QueueEntry {
  command: string;
  dataCapability: string;
  by: string;
  partition: string;
  position: number;
  basedOn: { version: number; values?: JsonObject };
  projection?: JsonObject;
  yourValue?: JsonValue;
  state: QueueState;
  confirmation?: { by: string } | null;
  parked?: { ttl: string; onExpire: 'drop' } | null;
  requeuedFrom?: string | null;
}

// --- L: message ------------------------------------------------------------

export const MESSAGE_KINDS = [
  'CommandQueued',
  'InvariantAtRisk',
  'ValueDeclaredAhead',
  'BlockedByHold',
  'HoldExpiring',
  'ReviewRequired',
  'Parked',
  'ParkedExpiring',
  'Expired',
  'AheadResolved',
  'CommandApplied',
  'CommandRejected',
] as const;
export type MessageKind = (typeof MESSAGE_KINDS)[number];

export const COMMAND_ACTIONS = [
  'confirm_overwrite',
  'cancel',
  'edit',
  'requeue',
  'renew',
  'approve',
  'reject',
] as const;
export type CommandAction = (typeof COMMAND_ACTIONS)[number];

export interface Message {
  message: MessageKind;
  to: string[];
  command?: string;
  field?: string;
  yourValue?: JsonValue;
  declaredAhead?: { value: JsonValue; by: string; command: string };
  hold?: { id: string; by: string; expires: string };
  dataVersion?: number;
  violations?: string[];
  reason?: string;
  actions?: CommandAction[];
  // L allows extra fields; they are kept, never read.
  [extra: string]: unknown;
}

// --- F: the fields of a DataCapability the shell uses ------------------------

export const MODES = ['confirm_on_stale', 'overwrite', 'relative'] as const;
export type Mode = (typeof MODES)[number];

export const EFFECTS = ['insert', 'update', 'delete', 'upsert'] as const;
export type Effect = (typeof EFFECTS)[number];

export interface DataCapabilityTarget {
  aggregate: string;
  id?: string;
}
