export { createShellClient, type ShellClient, type ShellClientOptions } from './client';
export type { AwaitOptions, AwaitOutcome, CommandView, SubmitInput, Submitted } from './commands';
export {
  COMMAND_ACTIONS,
  EFFECTS,
  G_FIELDS,
  H_FIELDS,
  MESSAGE_KINDS,
  MODES,
  QUEUE_STATES,
  TERMINAL_STATUSES,
  type CommandAction,
  type CommandResult,
  type DataCapabilityTarget,
  type Effect,
  type JsonObject,
  type JsonValue,
  type Message,
  type MessageKind,
  type Mode,
  type PendingState,
  type QueueEntry,
  type QueueState,
  type TerminalStatus,
} from './contracts';
export type { EventSourceLike, Unsubscribe } from './data-version';
export {
  ActionNotOfferedError,
  ClientDisposedError,
  InvalidIdentifierError,
  MissingBasedOnError,
  NonLoopbackBaseUrlError,
  ProtocolError,
  TransportError,
} from './errors';
export { IDENTIFIER_PATTERN } from './identifier';
export { ROUTES, SSE_DATA_VERSION_EVENT } from './protocol';
export type { ReadOutcome, WatchEvent } from './reads';
export type { Diagnostic, Timers } from './timers';
export { mapViolations, type MappedViolation, type ViolationMap } from './violations';
