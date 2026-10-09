# campagne-serveur

The local server of the app. On start it validates its configuration and the
DataGuard's aggregates, connects to PostgreSQL, applies every pending
migration, then listens on `127.0.0.1` only. It runs the one DataGuard
applier (`crates/dataguard`) and one `LISTEN data_version` connection.

## Endpoints

| Route | Answer |
|---|---|
| `GET /health` | `ok`. |
| `POST /commands` | Queues a command and answers `202` at once with `{commandId, partition, replayed, warnings}` and either `{entry, messages}` or `{result}`. Never waits for the apply, never refuses for concurrency. `400 {"error": id}` for `unknown-capability`, `malformed-submission`, `idempotency-key-required`, `idempotency-key-conflict`, `based-on-ahead`; `500 {"error":"internal"}`. |
| `GET /commands/{commandId}` | `{entry, messages}` while pending (contracts G and L), `{result}` once settled (contract H); `404 {"error":"not-found"}`. |
| `GET /data-version` | Server-sent events: `event: dataVersion`, `data: {"dataVersion": N}` — the current version first, then each greater one. No row data. |

The body of a submission is
`{dataCapability: "<system>.<name>@<version>", target?: {id}, payload, basedOn?: {version, values?}, idempotencyKey?}`.
A `by` in it is ignored: every command is the GM's (`mj-local`). This server
registers three DataCapabilities: `campagne.modifierPJ@1` (payload
`{nom, classe, niveau}`, all three, with `basedOn`), `campagne.archiverPJ@1`
and `campagne.archiverCampagne@1` (empty payload); each needs an
`idempotencyKey` and takes its row id in `target.id`. Any other is an
`unknown-capability`.

## Environment

Connection strings come from the environment only — no file, flag or default.
No error, log or response ever prints one.

| Variable | Required | Meaning |
|---|---|---|
| `DATABASE_URL` | yes | The role that migrates and, later, writes. It needs `CREATEROLE` and ownership of the database (migration 0004 creates the read-only role); the tests also need `CREATEDB`. |
| `DATABASE_URL_READONLY` | no (here) | The read-only login, for the Capabilities and the read-only role test. Never a fallback for `DATABASE_URL`. |
| `SERVER_PORT` | no | The port on `127.0.0.1`, default `7878`. A port already in use stops startup. |

## The read-only role

Migration 0004 creates `app_lecture` with `SELECT` on the views
`campagne_active` and `pj_actif`, and nothing else. It is created **without
login and without password**. After the first start, give it a login by hand,
with a password you choose locally and never commit:

```sql
ALTER ROLE app_lecture LOGIN PASSWORD '<chosen locally>';
```

Then build `DATABASE_URL_READONLY` from it:
`postgres://app_lecture:<chosen locally>@localhost:5432/<database>`.

CI does the same in its "Provision the read-only login" step, with a random
password per run.

## Migrations

Files live in `migrations/`, named `NNNN_snake_name.sql`, and are embedded in
the binary at build time. They are applied in numeric order, each in its own
transaction with its row in `schema_migrations` (name, SHA-256 of the raw
bytes, time applied).

- Add a migration as the next number. Never edit or remove an applied file:
  startup stops on a changed checksum or a missing file, and names it.
- No statement that cannot run in a transaction (`CREATE INDEX CONCURRENTLY`).
- No data, no seed row, no password.
- Expand/contract: a destructive change is its own later migration, after the
  code stopped using the old shape.

## Tests

Database tests create and drop their own database through `DATABASE_URL`, and
fail — never skip — when it is missing. The read-only role tests also need
`DATABASE_URL_READONLY` pointing at the `app_lecture` login.

The DataGuard engine's database tests (`tests/dataguard_*.rs`) live here, where
its migrations are, and drive it with test-only commands
(`tests/common/test_commands.rs`) and a manual clock. No test command is a
`data-capability.json`.
