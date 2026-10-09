//! Staleness of a `confirm_on_stale` command, by declared `touches`, not by
//! payload: a command is stale when an applied command newer than its base,
//! or a pending command ahead of it, touches one of its fields on the same
//! row. And what its author is shown then: the projection, and a
//! `ValueDeclaredAhead` message.

use serde_json::{Map, Value, json};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::model::{Operation, State};
use crate::store;

/// A command whose effects meet ours.
#[derive(Debug, Clone)]
pub struct Conflict {
    pub command: Uuid,
    pub by: String,
    pub value: Value,
    pub state: State,
    pub effects: Vec<String>,
}

fn conflict(row: sqlx::postgres::PgRow) -> Result<Conflict, sqlx::Error> {
    let operation: String = row.try_get("operation")?;
    let value = serde_json::from_str::<Operation>(&operation)
        .map(|op| Value::Object(op.fields))
        .unwrap_or(Value::Null);
    let state: String = row.try_get("state")?;
    Ok(Conflict {
        command: row.try_get("command")?,
        by: row.try_get("author")?,
        value,
        state: State::parse(&state).unwrap_or(State::Queued),
        effects: row.try_get("effects")?,
    })
}

/// Applied commands newer than `based_on`, and every pending command but
/// `me`, whose effects meet `effects`. Oldest first.
pub async fn conflicts(
    conn: &mut PgConnection,
    me: Uuid,
    effects: &[String],
    based_on: i64,
    include_pending: bool,
) -> Result<Vec<Conflict>, sqlx::Error> {
    if effects.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query(
        "SELECT command, author, operation, state, effects FROM dataguard_queue
         WHERE command <> $1 AND effects && $2
           AND ((state = 'applied' AND data_version > $3)
                OR ($4 AND state IN ('queued', 'awaiting_confirmation', 'confirmed')))
         ORDER BY enqueue_seq",
    )
    .bind(me)
    .bind(effects)
    .bind(based_on)
    .bind(include_pending)
    .fetch_all(conn)
    .await?
    .into_iter()
    .map(conflict)
    .collect()
}

/// Contract G's projection: the committed values of the touched fields, and
/// the pending commands ahead that touch them.
pub async fn projection(
    conn: &mut PgConnection,
    op: &Operation,
    touched: &[&str],
    conflicts: &[Conflict],
) -> Result<Value, sqlx::Error> {
    let confirmed = store::committed_values(conn, &op.aggregate, op.id, touched).await?;
    let pending: Vec<Value> = conflicts
        .iter()
        .filter(|c| !c.state.is_terminal())
        .map(|c| {
            json!({
                "command": c.command.to_string(),
                "by": c.by,
                "value": c.value,
                "state": c.state.as_str(),
            })
        })
        .collect();
    Ok(json!({ "confirmed": confirmed, "pendingAhead": pending }))
}

/// The value a stale command proposes: the fields it writes.
pub fn your_value(op: &Operation) -> Value {
    Value::Object(op.fields.clone())
}

/// Contract L: a value was declared ahead of yours. `field` is the first of
/// `mine` that the command ahead also touches, as `<Aggregate>.<field>`.
pub fn value_declared_ahead(
    by: &str,
    command: Uuid,
    op: &Operation,
    mine: &[String],
    ahead: &Conflict,
) -> Value {
    let field = mine
        .iter()
        .find(|e| ahead.effects.contains(e))
        .and_then(|e| e.rsplit_once('.'))
        .map(|(_, f)| format!("{}.{f}", op.aggregate));
    let mut m = Map::new();
    m.insert("message".into(), json!("ValueDeclaredAhead"));
    m.insert("to".into(), json!([by]));
    m.insert("command".into(), json!(command.to_string()));
    if let Some(field) = field {
        m.insert("field".into(), json!(field));
    }
    m.insert("yourValue".into(), your_value(op));
    m.insert(
        "declaredAhead".into(),
        json!({ "value": ahead.value, "by": ahead.by, "command": ahead.command.to_string() }),
    );
    m.insert(
        "actions".into(),
        json!(["confirm_overwrite", "cancel", "edit"]),
    );
    Value::Object(m)
}
