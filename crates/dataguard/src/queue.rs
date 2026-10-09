//! The DataQueue: submission (with the enqueue-time check on the projected
//! state), lookup, confirm, cancel, expiry and holds.
//!
//! Submissions serialise on a transaction-scoped advisory lock the applier
//! never takes, so enqueue order is commit order and submission never waits
//! for an apply.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::types::Json;
use sqlx::{Acquire, PgConnection, Row};
use uuid::Uuid;

use crate::GM_IDENTITY;
use crate::engine::{Inner, current_version};
use crate::error::{EngineError, HoldError, OperationError, SubmitError};
use crate::guard;
use crate::invariants::{self, Snapshot};
use crate::manifest::{
    Aggregates, DataCapabilityManifest, Effect, KeyPolicy, Mode, TOMBSTONE, field_names,
    parse_duration,
};
use crate::model::{
    BasedOn, Change, CommandId, CommandResult, Hold, Lookup, Malformed, OpKind, Operation,
    QueueEntry, State, Submission, Submitted,
};
use crate::store;

/// Serialises submissions and requeues ("dgenqueu").
pub(crate) const ENQUEUE_LOCK_KEY: i64 = 0x6467_656e_7175_6575;

/// How long a parked command waits for its author.
pub const PARK_TTL: &str = "24h";

pub(crate) const PENDING: &[&str] = &["queued", "awaiting_confirmation", "confirmed"];

pub fn mode_str(mode: Mode) -> &'static str {
    match mode {
        Mode::ConfirmOnStale => "confirm_on_stale",
        Mode::Overwrite => "overwrite",
        Mode::Relative => "relative",
    }
}

/// What "the same submission" means for a replay: its target and payload.
fn digest(target: Option<Uuid>, payload: &Map<String, Value>) -> String {
    let mut out = String::new();
    write_canonical(&json!({ "target": target, "payload": payload }), &mut out);
    hex::encode(Sha256::digest(out.as_bytes()))
}

/// JSON with keys sorted at every level, so a digest never depends on key
/// order.
fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            out.push('{');
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Checks the write a DataCapability made against its manifest and the
/// aggregates, and pins it to a row: an insert gets its id here.
fn operation(
    aggregates: &Aggregates,
    m: &DataCapabilityManifest,
    target: Option<Uuid>,
    write: crate::model::Write,
) -> Result<Operation, Malformed> {
    if write.aggregate != m.target.aggregate {
        return Err(Malformed("aggregate-mismatch"));
    }
    let touched = m.touched_fields();
    let (kind, id, fields) = match (m.effect, write.change) {
        (Effect::Insert, Change::Insert(fields)) => (OpKind::Insert, Uuid::new_v4(), fields),
        (Effect::Update, Change::Update { id, fields }) => {
            if fields.is_empty() {
                // Nothing to write: no version, no notification, no staleness.
                return Err(Malformed("no-change"));
            }
            if fields.keys().any(|f| !touched.contains(&f.as_str())) {
                return Err(Malformed("field-not-touched"));
            }
            (OpKind::Update, id, fields)
        }
        (Effect::Update, Change::Archive { id }) => {
            if !touched.contains(&TOMBSTONE) {
                return Err(Malformed("tombstone-not-touched"));
            }
            (OpKind::Archive, id, Map::new())
        }
        _ => return Err(Malformed("effect-mismatch")),
    };
    if kind != OpKind::Insert && Some(id) != target {
        return Err(Malformed("target-mismatch"));
    }
    for field in fields.keys() {
        if !invariants::writable(aggregates, &write.aggregate, kind, field) {
            // A row id, a tombstone or a restricted relation field.
            return Err(Malformed("field-not-writable"));
        }
    }
    if kind == OpKind::Insert
        && aggregates.relations().iter().any(|r| {
            r.source == write.aggregate
                && fields
                    .get(&r.field)
                    .and_then(Value::as_str)
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .is_none()
        })
    {
        return Err(Malformed("relation-id"));
    }
    Ok(Operation {
        aggregate: write.aggregate,
        id,
        kind,
        fields,
    })
}

/// The campaign partition whose rows the invariants of `op` read. A PC known
/// only to a pending insert takes that insert's scope.
async fn scope_of(conn: &mut PgConnection, op: &Operation) -> Result<String, sqlx::Error> {
    if let Some(c) = store::scope_campaign(conn, op).await? {
        return Ok(format!("{}/{c}", invariants::CAMPAGNE));
    }
    let queued: Option<String> = sqlx::query_scalar(
        "SELECT scope FROM dataguard_queue WHERE partition = $1 ORDER BY enqueue_seq LIMIT 1",
    )
    .bind(op.partition())
    .fetch_optional(&mut *conn)
    .await?;
    Ok(queued.unwrap_or_else(|| op.partition()))
}

/// The campaign a `Campagne/<id>` scope names.
fn campaign_of_scope(scope: &str) -> Option<Uuid> {
    scope
        .strip_prefix("Campagne/")
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// Committed rows of the scope, plus every pending command of the scope
/// assumed applied, in enqueue order.
async fn projected(
    conn: &mut PgConnection,
    scope: &str,
    at: DateTime<Utc>,
) -> Result<Snapshot, EngineError> {
    let mut snapshot = match campaign_of_scope(scope) {
        Some(c) => store::load_scope(conn, c, false).await?,
        None => Snapshot::default(),
    };
    let ahead: Vec<String> = sqlx::query_scalar(
        "SELECT operation FROM dataguard_queue
         WHERE scope = $1 AND state = ANY($2) ORDER BY enqueue_seq",
    )
    .bind(scope)
    .bind(PENDING)
    .fetch_all(&mut *conn)
    .await?;
    for text in ahead {
        let op: Operation =
            serde_json::from_str(&text).map_err(|_| EngineError::Corrupt("operation"))?;
        snapshot.apply(&op, at);
    }
    Ok(snapshot)
}

pub(crate) fn message(kind: &str, by: &str, command: Uuid, extra: Value) -> Value {
    let mut m = Map::new();
    m.insert("message".into(), json!(kind));
    m.insert("to".into(), json!([by]));
    m.insert("command".into(), json!(command.to_string()));
    if let Value::Object(extra) = extra {
        m.extend(extra);
    }
    Value::Object(m)
}

pub(crate) async fn submit(inner: &Inner, s: Submission) -> Result<Submitted, SubmitError> {
    let cap = inner
        .registry
        .get(&s.data_capability)
        .ok_or(SubmitError::UnknownCapability)?;
    let m = cap.manifest();
    let target = s.target.as_ref().map(|t| t.id);
    let write_target = match m.effect {
        Effect::Insert if target.is_some() => {
            return Err(SubmitError::Malformed("target-on-insert"));
        }
        Effect::Insert => None,
        _ => Some(target.ok_or(SubmitError::Malformed("target-required"))?),
    };
    let write = cap
        .write(write_target, &s.payload)
        .map_err(|Malformed(why)| SubmitError::Malformed(why))?;
    let op = operation(&inner.aggregates, m, target, write)
        .map_err(|Malformed(why)| SubmitError::Malformed(why))?;
    let key = s.idempotency_key.filter(|k| !k.trim().is_empty());
    if m.idempotency_key == KeyPolicy::Required && key.is_none() {
        return Err(SubmitError::IdempotencyKeyRequired);
    }
    if m.mode == Mode::ConfirmOnStale && s.based_on.is_none() {
        return Err(SubmitError::Malformed("based-on-required"));
    }
    let data_capability = m.key();
    let payload_digest = digest(target, &s.payload);
    let by = GM_IDENTITY;

    let mut conn = inner.pool.acquire().await?;
    let mut tx = conn.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ENQUEUE_LOCK_KEY)
        .execute(&mut *tx)
        .await?;

    if let Some(key) = &key {
        let original: Option<(Uuid, String, String, Vec<String>)> = sqlx::query_as(
            "SELECT command, partition, payload_digest, warnings FROM dataguard_queue
             WHERE author = $1 AND data_capability = $2 AND idempotency_key = $3",
        )
        .bind(by)
        .bind(&data_capability)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((command, partition, stored, warnings)) = original {
            if stored != payload_digest {
                return Err(SubmitError::IdempotencyKeyConflict);
            }
            let lookup = lookup(&mut tx, command)
                .await?
                .ok_or(SubmitError::Internal("replay".into()))?;
            tx.commit().await?;
            return Ok(Submitted {
                command_id: CommandId(command),
                partition,
                replayed: true,
                warnings,
                lookup,
            });
        }
    }

    let current = current_version(&mut tx).await?;
    let based_on = s.based_on.unwrap_or(BasedOn {
        version: u64::try_from(current).unwrap_or(0),
        values: None,
    });
    let based_on_version =
        i64::try_from(based_on.version).map_err(|_| SubmitError::BasedOnAhead)?;
    if based_on_version > current {
        return Err(SubmitError::BasedOnAhead);
    }

    let command = Uuid::new_v4();
    let now = inner.clock.now();
    let seq: i64 = sqlx::query_scalar("SELECT nextval('dataguard_enqueue_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let partition = op.partition();
    let position: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(position), -1) + 1 FROM dataguard_queue WHERE partition = $1",
    )
    .bind(&partition)
    .fetch_one(&mut *tx)
    .await?;
    let scope = scope_of(&mut tx, &op).await?;
    let touched = m.touched_fields();
    let effects = op.effects(touched.iter().copied());

    let mut state = State::Queued;
    let mut violations = Vec::new();
    let mut warnings = Vec::new();
    let mut messages = Vec::new();
    let mut your_value = None;
    let mut projection = None;

    let payload_violations = invariants::payload_violations(&op);
    if !payload_violations.is_empty() {
        // Rejected at once: the payload alone shows it. No version consumed.
        state = State::Rejected;
        violations = inner.aggregates.sort_violations(&payload_violations);
        messages.push(message(
            "CommandRejected",
            by,
            command,
            json!({ "violations": violations }),
        ));
    } else {
        let snapshot = projected(&mut tx, &scope, now).await?;
        warnings = inner
            .aggregates
            .sort_violations(&invariants::state_violations(&op, &snapshot));
        if !warnings.is_empty() {
            messages.push(message(
                "InvariantAtRisk",
                by,
                command,
                json!({ "violations": warnings }),
            ));
        }
        if m.mode == Mode::ConfirmOnStale {
            let conflicts =
                guard::conflicts(&mut tx, command, &effects, based_on_version, true).await?;
            if let Some(ahead) = conflicts.first() {
                state = State::AwaitingConfirmation;
                your_value = Some(guard::your_value(&op));
                projection = Some(guard::projection(&mut tx, &op, &touched, &conflicts).await?);
                messages.push(guard::value_declared_ahead(
                    by, command, &op, &effects, ahead,
                ));
            }
        }
    }

    let operation_text =
        serde_json::to_string(&op).map_err(|_| SubmitError::Malformed("operation"))?;
    let payload_text =
        serde_json::to_string(&s.payload).map_err(|_| SubmitError::Malformed("payload"))?;
    sqlx::query(
        "INSERT INTO dataguard_queue (
           command, enqueue_seq, data_capability, author, partition, position, scope, mode,
           touches, effects, operation, payload, payload_digest, idempotency_key, based_on,
           projection, your_value, state, messages, warnings, violations, enqueued_at, settled_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                 $16, $17, $18, $19, $20, $21, $22, $23)",
    )
    .bind(command)
    .bind(seq)
    .bind(&data_capability)
    .bind(by)
    .bind(&partition)
    .bind(position)
    .bind(&scope)
    .bind(mode_str(m.mode))
    .bind(&m.touches)
    .bind(&effects)
    .bind(operation_text)
    .bind(payload_text)
    .bind(&payload_digest)
    .bind(&key)
    .bind(serde_json::to_string(&based_on).map_err(|_| SubmitError::Malformed("based-on"))?)
    .bind(projection.map(Json))
    .bind(your_value.map(Json))
    .bind(state.as_str())
    .bind(Json(&messages))
    .bind(&warnings)
    .bind(&violations)
    .bind(now)
    .bind(state.is_terminal().then_some(now))
    .execute(&mut *tx)
    .await?;
    let lookup = lookup(&mut tx, command)
        .await?
        .ok_or(SubmitError::Internal("enqueue".into()))?;
    tx.commit().await?;
    Ok(Submitted {
        command_id: CommandId(command),
        partition,
        replayed: false,
        warnings,
        lookup,
    })
}

const SELECT_ENTRY: &str = "SELECT command, data_capability, author, partition, position,
    based_on, projection, your_value, state, confirmation, parked, requeued_from, messages,
    violations, data_version
  FROM dataguard_queue WHERE command = $1";

fn lookup_of(row: &sqlx::postgres::PgRow) -> Result<Lookup, EngineError> {
    let command: Uuid = row.try_get("command")?;
    let state: String = row.try_get("state")?;
    let state = State::parse(&state).ok_or(EngineError::Corrupt("state"))?;
    if state.is_terminal() {
        return Ok(Lookup::Settled {
            result: CommandResult {
                command_id: command.to_string(),
                status: state,
                data_version: row.try_get("data_version")?,
                violations: row.try_get("violations")?,
                review_id: None,
            },
        });
    }
    let json = |name: &str| -> Result<Option<Value>, EngineError> {
        Ok(row.try_get::<Option<Json<Value>>, _>(name)?.map(|j| j.0))
    };
    let based_on: String = row.try_get("based_on")?;
    let based_on: BasedOn =
        serde_json::from_str(&based_on).map_err(|_| EngineError::Corrupt("based_on"))?;
    let messages: Json<Vec<Value>> = row.try_get("messages")?;
    Ok(Lookup::Pending {
        entry: Box::new(QueueEntry {
            command: command.to_string(),
            data_capability: row.try_get("data_capability")?,
            by: row.try_get("author")?,
            partition: row.try_get("partition")?,
            position: row.try_get("position")?,
            based_on,
            projection: json("projection")?,
            your_value: json("your_value")?,
            state,
            confirmation: json("confirmation")?,
            parked: json("parked")?,
            requeued_from: row
                .try_get::<Option<Uuid>, _>("requeued_from")?
                .map(|u| u.to_string()),
        }),
        messages: messages.0,
    })
}

pub(crate) async fn lookup(
    conn: &mut PgConnection,
    command: Uuid,
) -> Result<Option<Lookup>, EngineError> {
    let row = sqlx::query(SELECT_ENTRY)
        .bind(command)
        .fetch_optional(conn)
        .await?;
    row.as_ref().map(lookup_of).transpose()
}

async fn author_and_state(
    conn: &mut PgConnection,
    command: Uuid,
) -> Result<Option<(String, State)>, OperationError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT author, state FROM dataguard_queue WHERE command = $1")
            .bind(command)
            .fetch_optional(conn)
            .await?;
    row.map(|(a, s)| {
        State::parse(&s)
            .map(|s| (a, s))
            .ok_or(OperationError::Internal("state".into()))
    })
    .transpose()
}

async fn settled_lookup(conn: &mut PgConnection, command: Uuid) -> Result<Lookup, OperationError> {
    lookup(conn, command).await?.ok_or(OperationError::NotFound)
}

pub(crate) async fn cancel(
    inner: &Inner,
    command: Uuid,
    by: &str,
) -> Result<Lookup, OperationError> {
    let mut conn = inner.pool.acquire().await?;
    let (author, _) = author_and_state(&mut conn, command)
        .await?
        .ok_or(OperationError::NotFound)?;
    if author != by {
        return Err(OperationError::NotAuthor);
    }
    // Waits on the applier's row lock: a command inside its applying
    // transaction ends applied, and this matches nothing.
    sqlx::query(
        "UPDATE dataguard_queue SET state = 'cancelled', settled_at = $2
         WHERE command = $1
           AND state IN ('queued', 'awaiting_confirmation', 'confirmed', 'parked')",
    )
    .bind(command)
    .bind(inner.clock.now())
    .execute(&mut *conn)
    .await?;
    settled_lookup(&mut conn, command).await
}

pub(crate) async fn confirm(
    inner: &Inner,
    command: Uuid,
    by: &str,
) -> Result<Lookup, OperationError> {
    let mut conn = inner.pool.acquire().await?;
    // A confirm can race the applier moving the command on; a few rounds
    // settle it on whatever state the command reached.
    for _ in 0..3 {
        let (author, state) = author_and_state(&mut conn, command)
            .await?
            .ok_or(OperationError::NotFound)?;
        if author != by {
            return Err(OperationError::NotAuthor);
        }
        match state {
            State::AwaitingConfirmation => {
                let done = sqlx::query(
                    "UPDATE dataguard_queue SET state = 'confirmed', confirmation = $2
                     WHERE command = $1 AND state = 'awaiting_confirmation'",
                )
                .bind(command)
                .bind(Json(json!({ "by": by })))
                .execute(&mut *conn)
                .await?
                .rows_affected();
                if done == 1 {
                    return settled_lookup(&mut conn, command).await;
                }
            }
            State::Parked => {
                if requeue(inner, &mut conn, command, by).await? {
                    return settled_lookup(&mut conn, command).await;
                }
            }
            _ => return settled_lookup(&mut conn, command).await,
        }
    }
    settled_lookup(&mut conn, command).await
}

/// `parked_at, partition, operation, touches, effects`.
type ParkedRow = (DateTime<Utc>, String, String, Vec<String>, Vec<String>);

/// A confirmed parked command comes back at the end of its partition, on
/// the current version, and applies without asking again. At or past its
/// ttl it expires instead: expiry wins over a confirm.
async fn requeue(
    inner: &Inner,
    conn: &mut PgConnection,
    command: Uuid,
    by: &str,
) -> Result<bool, OperationError> {
    let mut tx = conn.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ENQUEUE_LOCK_KEY)
        .execute(&mut *tx)
        .await?;
    let row: Option<ParkedRow> = sqlx::query_as(
        "SELECT parked_at, partition, operation, touches, effects FROM dataguard_queue
         WHERE command = $1 AND state = 'parked' FOR UPDATE",
    )
    .bind(command)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((parked_at, partition, operation, touches, effects)) = row else {
        return Ok(false);
    };
    let now = inner.clock.now();
    let ttl = park_ttl();
    if now >= parked_at + ttl {
        expire(&mut tx, command, by, now).await?;
        tx.commit().await?;
        return Ok(true);
    }
    let op: Operation = serde_json::from_str(&operation)
        .map_err(|_| OperationError::Internal("operation".into()))?;
    let current = current_version(&mut tx).await?;
    let seq: i64 = sqlx::query_scalar("SELECT nextval('dataguard_enqueue_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let position: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(position), -1) + 1 FROM dataguard_queue WHERE partition = $1",
    )
    .bind(&partition)
    .fetch_one(&mut *tx)
    .await?;
    let touched = field_names(&touches);
    let conflicts = guard::conflicts(&mut tx, command, &effects, current, true).await?;
    let projection = guard::projection(&mut tx, &op, &touched, &conflicts).await?;
    sqlx::query(
        "UPDATE dataguard_queue SET enqueue_seq = $2, position = $3, based_on = $4,
           projection = $5, state = 'confirmed', confirmation = $6, parked = NULL,
           parked_at = NULL, requeued_from = command
         WHERE command = $1",
    )
    .bind(command)
    .bind(seq)
    .bind(position)
    .bind(json!({ "version": current }).to_string())
    .bind(Json(projection))
    .bind(Json(json!({ "by": by })))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

fn park_ttl() -> chrono::Duration {
    let ttl = parse_duration(PARK_TTL).expect("PARK_TTL is a contract duration");
    chrono::Duration::from_std(ttl).expect("24h fits")
}

async fn expire(
    conn: &mut PgConnection,
    command: Uuid,
    by: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE dataguard_queue SET state = 'expired', settled_at = $2, messages = messages || $3
         WHERE command = $1 AND state = 'parked'",
    )
    .bind(command)
    .bind(now)
    .bind(Json(json!([message("Expired", by, command, json!({}))])))
    .execute(conn)
    .await?;
    Ok(())
}

pub(crate) async fn expire_due(inner: &Inner) -> Result<u64, EngineError> {
    let now = inner.clock.now();
    let mut conn = inner.pool.acquire().await?;
    let due: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT command, author FROM dataguard_queue
         WHERE state = 'parked' AND parked_at <= $1 ORDER BY enqueue_seq",
    )
    .bind(now - park_ttl())
    .fetch_all(&mut *conn)
    .await?;
    for (command, by) in &due {
        expire(&mut conn, *command, by, now).await?;
    }
    Ok(due.len() as u64)
}

fn max_ttl(inner: &Inner, partition: &str) -> Result<(Duration, bool), HoldError> {
    let aggregate = partition.split_once('/').map(|(a, _)| a);
    let holds = &aggregate
        .and_then(|a| inner.aggregates.get(a))
        .ok_or(HoldError::NotFound)?
        .holds;
    let max = parse_duration(&holds.max_ttl).map_err(|_| HoldError::Internal("maxTtl".into()))?;
    Ok((max, holds.renewable))
}

fn expires(now: DateTime<Utc>, ttl: Duration) -> Result<DateTime<Utc>, HoldError> {
    chrono::Duration::from_std(ttl)
        .ok()
        .and_then(|d| now.checked_add_signed(d))
        .ok_or(HoldError::AboveMaxTtl)
}

/// Records a hold. Refused, not clamped, above `holds.maxTtl`.
pub(crate) async fn hold(
    inner: &Inner,
    partition: &str,
    by: &str,
    ttl: Duration,
) -> Result<Hold, HoldError> {
    let (max, _) = max_ttl(inner, partition)?;
    if ttl > max {
        return Err(HoldError::AboveMaxTtl);
    }
    let now = inner.clock.now();
    let hold = Hold {
        id: Uuid::new_v4(),
        partition: partition.to_owned(),
        by: by.to_owned(),
        expires_at: expires(now, ttl)?,
    };
    sqlx::query(
        "INSERT INTO dataguard_hold (id, partition, author, expires_at, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(hold.id)
    .bind(&hold.partition)
    .bind(&hold.by)
    .bind(hold.expires_at)
    .bind(now)
    .execute(&inner.pool)
    .await?;
    Ok(hold)
}

/// Extends a live hold to `ttl` from now, still capped at `holds.maxTtl`.
pub(crate) async fn renew_hold(
    inner: &Inner,
    id: Uuid,
    by: &str,
    ttl: Duration,
) -> Result<Hold, HoldError> {
    let row: Option<(String, String, DateTime<Utc>)> =
        sqlx::query_as("SELECT partition, author, expires_at FROM dataguard_hold WHERE id = $1")
            .bind(id)
            .fetch_optional(&inner.pool)
            .await?;
    let (partition, author, expires_at) = row.ok_or(HoldError::NotFound)?;
    if author != by {
        return Err(HoldError::NotAuthor);
    }
    let (max, renewable) = max_ttl(inner, &partition)?;
    if !renewable {
        return Err(HoldError::NotRenewable);
    }
    let now = inner.clock.now();
    if now >= expires_at {
        return Err(HoldError::Expired);
    }
    if ttl > max {
        return Err(HoldError::AboveMaxTtl);
    }
    let expires_at = expires(now, ttl)?;
    // A hold that lapsed since it was read stays lapsed.
    let renewed =
        sqlx::query("UPDATE dataguard_hold SET expires_at = $2 WHERE id = $1 AND expires_at > $3")
            .bind(id)
            .bind(expires_at)
            .bind(now)
            .execute(&inner.pool)
            .await?
            .rows_affected();
    if renewed == 0 {
        return Err(HoldError::Expired);
    }
    Ok(Hold {
        id,
        partition,
        by: author,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_ignores_key_order_but_not_number_kind() {
        let a: Map<String, Value> = serde_json::from_str(r#"{"name":"Y","level":5}"#).unwrap();
        let b: Map<String, Value> = serde_json::from_str(r#"{"level":5,"name":"Y"}"#).unwrap();
        let c: Map<String, Value> = serde_json::from_str(r#"{"level":5.0,"name":"Y"}"#).unwrap();
        assert_eq!(digest(None, &a), digest(None, &b));
        assert_ne!(digest(None, &a), digest(None, &c));
        assert_ne!(digest(None, &a), digest(Some(Uuid::nil()), &a));
    }

    #[test]
    fn the_park_ttl_is_a_day() {
        assert_eq!(park_ttl(), chrono::Duration::hours(24));
    }

    fn manifest(effect: &str, aggregate: &str, touches: &[&str]) -> DataCapabilityManifest {
        serde_json::from_value(json!({
            "dataCapability": "test.op",
            "owner": "dataguard",
            "version": 1,
            "effect": effect,
            "target": {"aggregate": aggregate},
            "touches": touches,
            "payload": {},
            "mode": "overwrite",
            "invariants": [],
            "permissions": [],
            "callableBy": ["campagne"],
            "idempotencyKey": "optional"
        }))
        .unwrap()
    }

    fn write(aggregate: &str, change: Change) -> crate::model::Write {
        crate::model::Write {
            aggregate: aggregate.into(),
            change,
        }
    }

    fn fields(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_write_never_sets_the_tombstone_or_a_row_id_itself() {
        let a = Aggregates::embedded().unwrap();
        let pc = Uuid::new_v4();
        let campaign = Uuid::new_v4().to_string();
        // Even a capability that touches the tombstone only archives: an
        // update cannot rewrite or clear `archivedAt`.
        let archiver = manifest("update", "PJ", &["PJ.archivedAt"]);
        for value in [json!(null), json!("2026-01-01T00:00:00Z")] {
            let clear = Change::Update {
                id: pc,
                fields: fields(json!({ "archivedAt": value })),
            };
            assert_eq!(
                operation(&a, &archiver, Some(pc), write("PJ", clear)),
                Err(Malformed("field-not-writable"))
            );
        }
        let adder = manifest("insert", "PJ", &[]);
        for extra in ["archivedAt", "id"] {
            let mut f =
                fields(json!({"campagneId": campaign, "name": "Y", "class": "B", "level": 1}));
            f.insert(extra.into(), json!(Uuid::new_v4().to_string()));
            assert_eq!(
                operation(&a, &adder, None, write("PJ", Change::Insert(f))),
                Err(Malformed("field-not-writable")),
                "{extra}"
            );
        }
        // The engine mints the id of an insert.
        let ok = fields(json!({"campagneId": campaign, "name": "Y", "class": "B", "level": 1}));
        let first = operation(&a, &adder, None, write("PJ", Change::Insert(ok.clone()))).unwrap();
        let second = operation(&a, &adder, None, write("PJ", Change::Insert(ok))).unwrap();
        assert_eq!(first.kind, OpKind::Insert);
        assert_ne!(first.id, second.id);
        assert!(!first.id.is_nil());
    }

    #[test]
    fn a_write_stays_on_its_manifest_and_its_partition() {
        let a = Aggregates::embedded().unwrap();
        let (target, other) = (Uuid::new_v4(), Uuid::new_v4());
        let editor = manifest("update", "PJ", &["PJ.level"]);
        let level = |id| Change::Update {
            id,
            fields: fields(json!({"level": 3})),
        };

        let edit = operation(&a, &editor, Some(target), write("PJ", level(target))).unwrap();
        assert_eq!((edit.kind, edit.id), (OpKind::Update, target));

        // A row other than the submission's target would apply outside the
        // partition it was queued in.
        assert_eq!(
            operation(&a, &editor, Some(target), write("PJ", level(other))),
            Err(Malformed("target-mismatch"))
        );
        let archiver = manifest("update", "PJ", &["PJ.archivedAt"]);
        assert_eq!(
            operation(
                &a,
                &archiver,
                Some(target),
                write("PJ", Change::Archive { id: other })
            ),
            Err(Malformed("target-mismatch"))
        );
        assert_eq!(
            operation(&a, &editor, Some(target), write("Campagne", level(target))),
            Err(Malformed("aggregate-mismatch"))
        );
        assert_eq!(
            operation(
                &a,
                &editor,
                Some(target),
                write("PJ", Change::Archive { id: target })
            ),
            Err(Malformed("tombstone-not-touched"))
        );
        let adder = manifest("insert", "PJ", &[]);
        assert_eq!(
            operation(&a, &adder, Some(target), write("PJ", level(target))),
            Err(Malformed("effect-mismatch"))
        );
    }
}
