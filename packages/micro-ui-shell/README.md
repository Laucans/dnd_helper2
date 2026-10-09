# `@dnd-helper/micro-ui-shell`

The shared client every Micro-UI uses to talk to the local server. It calls a
Capability or a DataCapability **by identifier only**, submits commands, waits
for their result, maps violations to field messages, acts on a pending command,
and follows the server's `dataVersion` push so a Micro-UI refetches by itself.

It holds no business logic: no invariant, no validation rule, no party-level
maths, no filtering, no sorting. It is not a Micro-UI and has no `micro-ui.json`.

The wire contract is in [`PROTOCOL.md`](PROTOCOL.md).

## Where it lives, and why

`packages/micro-ui-shell/`, chosen once. `CLAUDE.md`'s layout has no slot for a
shared TypeScript library: Micro-UIs live in `apps/<system>/<micro-ui>/`, units
in `crates/`. The shell is neither, so it sits at a path no architecture gate
globs (`apps/**/micro-ui.json`, `apps/screens/*.json`, `crates/**`,
`concepts/**`). `system-frontier` and `write-actions` never see it.

The root `package.json` declares `workspaces: ["packages/*"]`; the `typescript`
CI job only runs when a `package.json` exists **at the repo root**. A Micro-UI
joins by adding `apps/*/*` to the root `workspaces` and
`"@dnd-helper/micro-ui-shell": "*"` to its own `package.json`.

The package is consumed as TypeScript source (`exports` → `src/index.ts`); there
is no build step and no `dist/`.

## Rules the code keeps

- Imports nothing from `crates/**`, any Capability, DataCapability, Micro-UI or
  WASM glue — enforced by `no-restricted-imports` and by `test/isolation.test.ts`.
- No runtime dependency: only the platform (`fetch`, `EventSource`, `crypto`,
  `AbortSignal`).
- Reads no environment variable and holds no secret. Writes nothing to the
  console, a file or storage; diagnostics go through the optional `onDiagnostic`
  hook and carry a `kind` only, never a payload value or a key.
- Tests are hermetic: a fake `fetch`, a fake `EventSource` factory and a fake
  clock (`test/fakes.ts`). No test opens a socket.

## Use

```ts
import { createShellClient } from '@dnd-helper/micro-ui-shell';

const shell = createShellClient({
  baseUrl: 'http://127.0.0.1:7878',          // loopback only
  fetch: (input, init) => globalThis.fetch(input, init),
  eventSource: (url) => new EventSource(url),
});
```

### Options

| Option | Default | |
|---|---|---|
| `baseUrl` | — | Required. Loopback only, else `NonLoopbackBaseUrlError` at construction |
| `fetch` | — | Required, never read from a global |
| `eventSource` | — | Required factory `(url) => EventSourceLike` |
| `timers` | `setTimeout` / `clearTimeout` / `Date.now` | Injectable clock |
| `newIdempotencyKey` | `crypto.randomUUID()` | |
| `onDiagnostic` | none | The only way anything leaves the shell |
| `poll` | `{initialMs: 250, maxMs: 5000, factor: 2}` | Bounded backoff of `awaitResult`; also spaces submit retries |
| `awaitTimeoutMs` | `30000` | Default limit of one `awaitResult` |
| `submitRetries` | `2` | Network failures only, same key |
| `reconnect` | `{initialMs: 500, maxMs: 30000}` | Bounded backoff of the SSE stream |
| `staleRead` | `{retries: 5, delayMs: 200, pushWaitMs: 5000}` | Read-after-write |

### Reads

```ts
const out = await shell.read<Pc[]>('campagne.listerPjs', { campagneId });
// { kind: 'found', data, asOf } | { kind: 'not-found' }

const stop = shell.watch<Pc[]>('campagne.listerPjs', { campagneId }, (e) => {
  // e.kind: 'data' | 'not-found' | 'stale' | 'error'
});
```

A read takes an identifier and variables — there is no parameter for query text.
`watch` keys its entry by identifier plus the full variables (key order does not
matter), so data read for campaign A is never served for campaign B. Two
watchers of one key share one fetch. A new highest `dataVersion` marks every
entry dirty; a burst of pushes is one refetch per entry, at the latest version.

### Commands

```ts
const sent = await shell.submit({
  dataCapability: 'campagne.modifierPJ', version: 1, mode: 'confirm_on_stale',
  target: { aggregate: 'PJ', id: pcId },
  payload: { level: 5 },
  basedOn: { version: dataVersionTheAuthorSaw },   // required for confirm_on_stale
  idempotencyKey: formAttemptKey,                    // optional: one key per form attempt
});
sent.first;  // the first state observed — NOT a result

const outcome = await shell.awaitResult(sent.commandId, {
  violationMessages: { 'pc-level-range': { field: 'level', message: '…' } },
  onState: (view) => { /* queued, awaiting_confirmation (yourValue, actions), parked… */ },
});
// 'settled' | 'still-pending' | 'not-found'
```

- Without `idempotencyKey`, each `submit` call mints a fresh one: two identical
  payloads are two commands. The shell never dedupes by content.
- `awaitResult` resolves only on `applied`, `rejected`, `expired` or `cancelled`.
  A timeout or an abort is `still-pending` (with the `commandId`); it never
  cancels, resubmits or reports a rejection, and a later wait resumes.
- The settled `result` is exactly contract H's five fields. The mapped messages
  are returned beside it as `violationMessages`, one per id in server order. An
  id missing from the map, or a rejection without ids, gets a generic message
  (`rejected: <id>` / `rejected`); pass your own copy through `mapViolations`.
- `act(commandId, 'confirm_overwrite' | 'cancel')` posts to the server only when
  the latest known message listed the action, else `ActionNotOfferedError`
  without a request. If the client has not seen the command (after a page
  reload), call `lookup(commandId)` first. The author's gesture is what calls it.
- The first terminal state seen for a `commandId` is final.

### dataVersion

```ts
const off = shell.onDataVersion((version) => { /* strictly greater than the last */ });
shell.knownDataVersion();
```

One SSE stream per client, shared by all listeners and live reads; it opens on
the first and closes when the last leaves. A dropped stream reopens by itself
with bounded backoff, and the first `open` after an error refetches once.

## Outcomes and errors

A call ends in one of four outcomes, never folded together: a command result
(a rejection is a value), `not-found`, `TransportError`, `ProtocolError`. Local
refusals are separate typed errors: `InvalidIdentifierError` (rule 13),
`NonLoopbackBaseUrlError` (rule 10), `MissingBasedOnError` (rule 26),
`ActionNotOfferedError` (rule 35), `ClientDisposedError` (rule 34). There is no
conflict error: a queued or `awaiting_confirmation` command is not a failure.

`dispose()` aborts in-flight requests, clears every timer, closes the stream and
drops all listeners; any later call throws `ClientDisposedError` and a pending
`awaitResult` ends `still-pending` / `aborted`.

## Develop

From the repo root:

```
npm ci
npm run typecheck   # tsc --noEmit
npm run lint        # eslint . --max-warnings=0
npm run test        # vitest run
```
