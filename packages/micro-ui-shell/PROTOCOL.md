# Wire protocol

The one contract between the shell and the local server. The routes, the SSE
event name and every payload are typed in [`src/protocol.ts`](src/protocol.ts);
a mismatch with the server is fixed there and here, nowhere else.

Status of each route:

- **landed** — served today by `crates/campagne/serveur/src/http.rs`. The shell
  adapts to it byte for byte.
- **defined by #23 — server to conform** — specified here, not served yet. The
  server task that adds it conforms to this file.

The base URL is a constructor option and must be on the loopback interface
(`127.0.0.1`, `localhost` or `[::1]`). `http://127.0.0.1:7878` is recommended:
the server binds IPv4 only, so a browser that resolves `localhost` to `::1`
would not connect.

Every body is JSON. Every error answers `{"error": "<stable-id>"}`; the shell
never reads or keeps any other error text.

## Commands

### `POST /commands` — landed

Request:

```json
{
  "dataCapability": "<system>.<name>@<version>",
  "payload": { },
  "idempotencyKey": "<uuid>",
  "target": { "id": "<uuid>" },
  "basedOn": { "version": 3, "values": { } }
}
```

- `dataCapability` is the identifier (`^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*$`,
  contracts D and F) joined to its integer `version` ≥ 1 by `@` (contract G).
  The shell takes the two apart (`dataCapability`, `version`) and checks both
  before any call.
- `payload` goes out exactly as the caller gave it: no trim, no NFC, no default,
  no coercion. Numbers stay numbers, `"5"` stays `"5"`.
- `idempotencyKey` is always sent: the caller's own, or a fresh
  `crypto.randomUUID()` per submit call.
- `target: {"id"}` is sent only when the caller gave an `id`. The shell never
  sends `aggregate` or `mode` (the server reads both from the manifest).
- `basedOn` is sent only when the caller gave it. It is required, locally, for a
  `confirm_on_stale` command.
- The shell sends no `by`: the server owns the identity.

Answers:

| Status | Body |
|---|---|
| `202` | `{commandId, partition, replayed, warnings, entry, messages}` while the command is pending (`entry` is contract G, `messages` is an array of contract L) |
| `202` | `{commandId, partition, replayed, warnings, result}` when already settled — a replay (`result` is contract H) |
| `400` | `{"error": id}`, `id` ∈ `unknown-capability`, `malformed-submission`, `idempotency-key-required`, `idempotency-key-conflict`, `based-on-ahead` |
| `500` | `{"error": "internal"}` |

A transport failure with no response is retried (at most `submitRetries` times,
default 2), always with the same body and so the same key. No HTTP status is
ever retried.

### `GET /commands/{commandId}` — landed

| Status | Body |
|---|---|
| `200` | `{entry, messages}` while pending |
| `200` | `{result}` once settled (contract H) |
| `404` | `{"error": "not-found"}` (also for a malformed id) |

Exactly one of `entry` and `result` is present. L messages travel only here,
beside `entry`; they are never on the SSE stream.

### `POST /commands/{commandId}/confirm` and `POST /commands/{commandId}/cancel` — defined by #23, server to conform

Empty body. Answers the same shapes as `GET /commands/{commandId}`.

| Status | Body |
|---|---|
| `200` | `{entry, messages}` or `{result}` |
| `403` | `{"error": "not-author"}` — a transport error for the shell |
| `404` | `{"error": "not-found"}` |

The engine already has `Engine::confirm` and `Engine::cancel` returning the
same `Lookup`, so these two routes are thin wrappers. The shell sends one of
them only when the latest known message for the command listed the action in
`actions`; it sends no other action (`edit`, `requeue`, `renew`, `approve` and
`reject` are not exposed).

## Reads

### `POST /capabilities/{identifier}` — defined by #23, server to conform

`{identifier}` matches `^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*$`, for example
`campagne.listerPjs`. Contracts B, C and D carry no version, so a read has none.

Request: `{"variables": { }}` — the persisted query's variables, passed through
untouched. The body carries no query text; the shell has no API that could send
one. The shell adds no campaign id, default or filter: scoping is the caller's
variable and the Data layer's job.

| Status | Body |
|---|---|
| `200` | `{"asOf": N, "data": <the Capability's output>}` |
| `404` | `{"error": "not-found"}` — the one "not found" answer: an unknown, archived or foreign campaign id look the same (rule 31) |

`asOf` is the `dataVersion` the read was taken at. **Every read response carries
it** (owner decision); the shell tolerates its absence and then falls back to
waiting for the push (see "Read after write").

## dataVersion push

### `GET /data-version` — landed

Server-sent events. One named event, no other:

```
event: dataVersion
data: {"dataVersion": 12}
```

The current version first, then every greater one. Keep-alive comments are
ignored. No row data ever travels on this stream. The shell does not use the
default `message` event.

A payload that is not JSON, or whose `dataVersion` is not an integer ≥ 0, is
dropped with a diagnostic: no callback, no refetch.

## Outcomes

| Outcome | Shape |
|---|---|
| A command result (including `rejected`) | a value: `{kind: 'settled', result}` |
| Not found | a value: `{kind: 'not-found'}`; exactly `404 {"error":"not-found"}` |
| Transport failure | `TransportError`: `kind: 'network'` (no response) or `kind: 'http'` with `status` and the server's `errorId`. Any other non-2xx, including a `404` whose body is not exactly `{"error":"not-found"}` (a route the server has not added yet) |
| Protocol failure | `ProtocolError`: a 2xx whose body does not match its contract shape |

A rejection is never an exception, an HTTP error status is never reported as a
rejection, and no error message carries a payload value or a key.

## Read after write

After an `applied` result with `dataVersion` N, every live read refetches at
once and refuses a response whose `asOf` is below N: it retries a bounded number
of times, then reports `stale` and keeps the last fresh value. A response with
no `asOf` is held until the push reaches N or a bounded time passes, then read
once more. `applied` with `dataVersion: null` (a no-op) forces one refetch and
does not move the known version.
