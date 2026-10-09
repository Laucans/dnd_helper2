// The wire contract, in one place (rule 8): every route, the SSE event name
// and the payload shapes. A mismatch with the server is fixed here and in
// `PROTOCOL.md`, nowhere else.

import type { CommandResult, JsonObject, JsonValue, Message, QueueEntry } from './contracts';

export const ROUTES = {
  /** `POST` — submit a command. Landed (`crates/campagne/serveur/src/http.rs`). */
  commands: '/commands',
  /** `GET` — look a command up. Landed. */
  command: (id: string): string => `/commands/${encodeURIComponent(id)}`,
  /** `POST` — confirm an overwrite. Defined by #23, not served yet. */
  confirm: (id: string): string => `/commands/${encodeURIComponent(id)}/confirm`,
  /** `POST` — cancel a command. Defined by #23, not served yet. */
  cancel: (id: string): string => `/commands/${encodeURIComponent(id)}/cancel`,
  /** `POST` — run a persisted-query Capability by identifier. Defined by #23, not served yet. */
  capability: (identifier: string): string => `/capabilities/${identifier}`,
  /** `GET` — server-sent events of the global `dataVersion`. Landed. */
  dataVersion: '/data-version',
} as const;

export const SSE_DATA_VERSION_EVENT = 'dataVersion';

/** The one "not found" answer: `404 {"error":"not-found"}` (rule 18). */
export const NOT_FOUND_ERROR = 'not-found';

/** Body of `POST /commands`. No `by`, no `mode`, no `aggregate`. */
export interface SubmitBody {
  dataCapability: string;
  payload: JsonObject;
  idempotencyKey: string;
  target?: { id: string };
  basedOn?: { version: number; values?: JsonObject };
}

export type SettledAnswer = { result: CommandResult };
export type PendingAnswer = { entry: QueueEntry; messages: Message[] };
/** `GET /commands/{id}` and the confirm / cancel answers. */
export type LookupAnswer = SettledAnswer | PendingAnswer;

export type SubmitAnswer = LookupAnswer & {
  commandId: string;
  partition: string;
  replayed: boolean;
  warnings: string[];
};

export interface ReadBody {
  variables: JsonObject;
}

export interface ReadAnswer {
  asOf: number | null;
  data: JsonValue;
}

/** `data:` of the `dataVersion` event. */
export interface DataVersionEvent {
  dataVersion: number;
}
