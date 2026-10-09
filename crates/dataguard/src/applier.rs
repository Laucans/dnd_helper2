//! The single applier. It holds a session advisory lock on its own
//! connection, so a second instance applies nothing, and it is the only code
//! that bumps `dataVersion`.
//!
//! Each command applies in one transaction: the data change, the cascade, the
//! version, the terminal state and the notification commit together or not
//! at all. An infrastructure error rolls back and leaves the command at its
//! position, to be retried; it is never rejected for it.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::types::Json;
use sqlx::{Connection, PgConnection, Row};
use tracing::warn;
use uuid::Uuid;

use crate::engine::Inner;
use crate::error::EngineError;
use crate::guard;
use crate::invariants::{self, Snapshot};
use crate::manifest::field_names;
use crate::model::{BasedOn, CommandId, Operation, State};
use crate::notify::DATA_VERSION_CHANNEL;
use crate::queue::{self, PARK_TTL, PENDING, message};
use crate::resolver::{self, Plan};
use crate::store;

/// The single applier's lock ("dgapplie").
pub(crate) const APPLIER_LOCK_KEY: i64 = 0x6467_6170_706c_6965;

const POLL: Duration = Duration::from_millis(250);
const BACKOFF: Duration = Duration::from_secs(1);

pub struct Applier {
    inner: Arc<Inner>,
    conn: PgConnection,
}

/// The ready head of every partition — its lowest position among the
/// commands still to apply — and of those, the first enqueued.
const HEAD: &str = "SELECT q.command FROM dataguard_queue q
  WHERE q.state = ANY($1)
    AND NOT EXISTS (
      SELECT 1 FROM dataguard_queue p
      WHERE p.partition = q.partition AND p.state = ANY($1) AND p.position < q.position)
  ORDER BY q.enqueue_seq
  LIMIT 1";

struct Head {
    command: Uuid,
    by: String,
    state: State,
    mode: String,
    touches: Vec<String>,
    effects: Vec<String>,
    op: Operation,
    based_on: BasedOn,
    has_projection: bool,
}

impl Applier {
    pub(crate) async fn acquire(inner: Arc<Inner>) -> Result<Option<Self>, EngineError> {
        let mut conn = inner.pool.acquire().await?.detach();
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(APPLIER_LOCK_KEY)
            .fetch_one(&mut conn)
            .await?;
        if !locked {
            let _ = conn.close().await;
            return Ok(None);
        }
        Ok(Some(Self { inner, conn }))
    }

    /// Applies, rejects or parks the next ready head, in one transaction.
    /// `None` when no command is ready. On an error the transaction is rolled
    /// back before returning, so no lock outlives a failed apply. The failed
    /// head stays the head: nothing enqueued after it applies first (rules 25
    /// and 28), and a command that keeps failing stops the applier, visibly,
    /// until a human acts.
    pub async fn apply_next(&mut self) -> Result<Option<CommandId>, EngineError> {
        let inner = self.inner.clone();
        loop {
            let mut tx = self.conn.begin().await?;
            match step(&inner, &mut tx).await {
                Ok(Step::Idle) => {
                    tx.commit().await?;
                    return Ok(None);
                }
                Ok(Step::Raced) => tx.rollback().await?,
                Ok(Step::Settled(command)) => {
                    tx.commit().await?;
                    return Ok(Some(CommandId(command)));
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    return Err(e);
                }
            }
        }
    }
}

enum Step {
    /// No command is ready.
    Idle,
    /// (a) The head settled or parked between the pick and the lock.
    Raced,
    Settled(Uuid),
}

async fn step(inner: &Inner, conn: &mut PgConnection) -> Result<Step, EngineError> {
    let head: Option<Uuid> = sqlx::query_scalar(HEAD)
        .bind(PENDING)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(command) = head else {
        return Ok(Step::Idle);
    };
    settle_head(inner, conn, command).await
}

async fn settle_head(
    inner: &Inner,
    conn: &mut PgConnection,
    command: Uuid,
) -> Result<Step, EngineError> {
    let row = sqlx::query(
        "SELECT author, state, mode, touches, effects, operation, based_on,
           projection IS NOT NULL AS has_projection
         FROM dataguard_queue WHERE command = $1 FOR UPDATE",
    )
    .bind(command)
    .fetch_one(&mut *conn)
    .await?;
    let state: String = row.try_get("state")?;
    let state = State::parse(&state).ok_or(EngineError::Corrupt("state"))?;
    if !PENDING.contains(&state.as_str()) {
        return Ok(Step::Raced);
    }
    let operation: String = row.try_get("operation")?;
    let based_on: String = row.try_get("based_on")?;
    let based_on: BasedOn =
        serde_json::from_str(&based_on).map_err(|_| EngineError::Corrupt("based_on"))?;
    let head = Head {
        command,
        by: row.try_get("author")?,
        state,
        mode: row.try_get("mode")?,
        touches: row.try_get("touches")?,
        effects: row.try_get("effects")?,
        op: serde_json::from_str(&operation).map_err(|_| EngineError::Corrupt("operation"))?,
        based_on,
        has_projection: row.try_get("has_projection")?,
    };
    settle(inner, conn, head).await?;
    Ok(Step::Settled(command))
}

/// Rule order: (b) no-op archive, (c) invariants on the real state,
/// (d) staleness of an unconfirmed `confirm_on_stale`, (e) apply.
async fn settle(inner: &Inner, conn: &mut PgConnection, head: Head) -> Result<(), EngineError> {
    let now = inner.clock.now();
    let op = &head.op;
    let snapshot = match store::scope_campaign(conn, op).await? {
        Some(c) => store::load_scope(conn, c, true).await?,
        None => Snapshot::default(),
    };
    let current: i64 = sqlx::query_scalar("SELECT version FROM dataguard_version FOR UPDATE")
        .fetch_one(&mut *conn)
        .await?;
    let holds = holds_on(conn, &op.partition(), now).await?;
    let pending = pending_behind(conn, head.command, &op.partition()).await?;
    let plan = |decision, violations: &[String], cascade, reasons| {
        Plan {
            command: head.command,
            op,
            holds: holds.clone(),
            pending: pending.clone(),
            cascade,
            violations,
            decision,
            reasons,
        }
        .to_json()
    };

    // (b) Archiving an archived row changes nothing and is not refused.
    if op.kind == crate::model::OpKind::Archive && snapshot.is_archived(op) {
        let reasons = vec!["already-archived".to_owned()];
        let impact = plan("auto", &[], Vec::new(), reasons);
        let outcome = Outcome {
            state: State::Applied,
            version: Some(current),
            violations: &[],
            cascaded: None,
            impact,
            message: message(
                "CommandApplied",
                &head.by,
                head.command,
                json!({ "dataVersion": current }),
            ),
        };
        finish(conn, head.command, outcome, now).await?;
        return Ok(());
    }

    // (c) The application-time check decides.
    let violations = invariants::all_violations(&inner.aggregates, op, &snapshot);
    if !violations.is_empty() {
        let impact = plan("rejected", &violations, Vec::new(), violations.clone());
        let outcome = Outcome {
            state: State::Rejected,
            version: None,
            violations: &violations,
            cascaded: None,
            impact,
            message: message(
                "CommandRejected",
                &head.by,
                head.command,
                json!({ "violations": violations }),
            ),
        };
        finish(conn, head.command, outcome, now).await?;
        return Ok(());
    }

    // (d) Stale and unconfirmed: parked, out of the queue, for 24 h.
    if head.mode == "confirm_on_stale" && head.state != State::Confirmed {
        let based_on = i64::try_from(head.based_on.version).unwrap_or(i64::MAX);
        let conflicts =
            guard::conflicts(conn, head.command, &head.effects, based_on, false).await?;
        if let Some(ahead) = conflicts.first() {
            let touched = field_names(&head.touches);
            let mut messages = Vec::new();
            if !head.has_projection {
                messages.push(guard::value_declared_ahead(
                    &head.by,
                    head.command,
                    op,
                    &head.effects,
                    ahead,
                ));
            }
            messages.push(message(
                "Parked",
                &head.by,
                head.command,
                json!({
                    "reason": "stale-at-head",
                    "actions": ["confirm_overwrite", "cancel"],
                }),
            ));
            let projection = guard::projection(conn, op, &touched, &conflicts).await?;
            let impact = plan("confirm", &[], Vec::new(), vec!["stale".to_owned()]);
            sqlx::query(
                "UPDATE dataguard_queue SET state = 'parked', parked = $2, parked_at = $3,
                       your_value = $4, projection = $5, impact_plan = $6,
                       messages = messages || $7
                     WHERE command = $1",
            )
            .bind(head.command)
            .bind(Json(json!({ "ttl": PARK_TTL, "onExpire": "drop" })))
            .bind(now)
            .bind(Json(guard::your_value(op)))
            .bind(Json(projection))
            .bind(Json(impact))
            .bind(Json(messages))
            .execute(&mut *conn)
            .await?;
            return Ok(());
        }
    }

    // (e) Apply: rows, cascade, version and notification, one commit.
    let applied = resolver::apply(conn, &inner.aggregates, op, now).await?;
    let cascade = resolver::cascade_entries(&inner.aggregates, op, &applied);
    let version: i64 =
        sqlx::query_scalar("UPDATE dataguard_version SET version = version + 1 RETURNING version")
            .fetch_one(&mut *conn)
            .await?;
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(DATA_VERSION_CHANNEL)
        .bind(version.to_string())
        .execute(&mut *conn)
        .await?;
    let impact = plan("auto", &[], cascade, Vec::new());
    let outcome = Outcome {
        state: State::Applied,
        version: Some(version),
        violations: &[],
        cascaded: Some(&applied.effects),
        impact,
        message: message(
            "CommandApplied",
            &head.by,
            head.command,
            json!({ "dataVersion": version }),
        ),
    };
    finish(conn, head.command, outcome, now).await?;
    Ok(())
}

impl Applier {
    /// Expires what is due, then applies until no command is ready.
    pub async fn drain(&mut self) -> Result<usize, EngineError> {
        queue::expire_due(&self.inner).await?;
        let mut n = 0;
        while self.apply_next().await?.is_some() {
            n += 1;
        }
        Ok(n)
    }

    /// Drains on every wake-up, and every 250 ms in any case, until
    /// `shutdown`. Returns early when its connection, and so its lock, is
    /// lost: the caller acquires a new applier.
    pub async fn run(mut self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        loop {
            let pause = match self.drain().await {
                Ok(_) => POLL,
                Err(e) => {
                    warn!(error = %e, "applier: command left in place, retrying");
                    if self.conn.ping().await.is_err() {
                        warn!("applier: connection lost");
                        return;
                    }
                    BACKOFF
                }
            };
            tokio::select! {
                _ = &mut shutdown => return,
                _ = self.inner.wake.notified() => {}
                _ = tokio::time::sleep(pause) => {}
            }
        }
    }
}

/// How a command settles.
struct Outcome<'a> {
    state: State,
    version: Option<i64>,
    violations: &'a [String],
    /// `Some` adds the cascaded rows to the command's effects; `None` clears
    /// them: a no-op or a rejection wrote nothing, so it makes no one stale.
    cascaded: Option<&'a [String]>,
    impact: Value,
    message: Value,
}

async fn finish(
    conn: &mut PgConnection,
    command: Uuid,
    outcome: Outcome<'_>,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE dataguard_queue SET state = $2, data_version = $3, violations = $4,
           effects = CASE WHEN $5 THEN effects || $6 ELSE '{}' END,
           impact_plan = $7, messages = messages || $8, settled_at = $9
         WHERE command = $1",
    )
    .bind(command)
    .bind(outcome.state.as_str())
    .bind(outcome.version)
    .bind(outcome.violations)
    .bind(outcome.cascaded.is_some())
    .bind(outcome.cascaded.unwrap_or_default())
    .bind(Json(outcome.impact))
    .bind(Json(json!([outcome.message])))
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}

/// Live holds on a partition, for contract J's `dependencies.holds`.
async fn holds_on(
    conn: &mut PgConnection,
    partition: &str,
    now: DateTime<Utc>,
) -> Result<Vec<Value>, sqlx::Error> {
    let rows: Vec<(Uuid, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, author, expires_at FROM dataguard_hold
         WHERE partition = $1 AND expires_at > $2 ORDER BY created_at",
    )
    .bind(partition)
    .bind(now)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, by, expires)| {
            json!({ "id": id.to_string(), "by": by, "expires": expires.to_rfc3339() })
        })
        .collect())
}

/// Commands of the same partition still waiting behind this one.
async fn pending_behind(
    conn: &mut PgConnection,
    command: Uuid,
    partition: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT command FROM dataguard_queue
         WHERE partition = $1 AND command <> $2 AND state = ANY($3) ORDER BY position",
    )
    .bind(partition)
    .bind(command)
    .bind(PENDING)
    .fetch_all(conn)
    .await
}
