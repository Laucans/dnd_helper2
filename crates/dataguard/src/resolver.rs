//! The Resolver: what a command does to the rows, cascade included, and the
//! impact plan (contract J) it leaves behind. It runs inside the applying
//! transaction, on the real state, never at enqueue.
//!
//! The cascade is read from the relations, not written per aggregate: an
//! archive of a row archives, in the same transaction and at the same
//! instant, every active row whose relation points at it with `onDelete:
//! archive`. One failure rolls the whole command back.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::manifest::{Aggregates, OnDelete, TOMBSTONE};
use crate::model::{OpKind, Operation};
use crate::store;

/// What applying a command wrote beyond its own row.
#[derive(Debug, Default)]
pub struct Applied {
    /// Rows archived per relation, for contract J's `cascade`.
    pub cascade: BTreeMap<String, u64>,
    /// `<Aggregate>/<id>.archivedAt` of every cascaded row.
    pub effects: Vec<String>,
}

/// Writes `op`, and its cascade when it is an archive.
pub async fn apply(
    conn: &mut PgConnection,
    aggregates: &Aggregates,
    op: &Operation,
    at: DateTime<Utc>,
) -> Result<Applied, sqlx::Error> {
    let mut out = Applied::default();
    match op.kind {
        OpKind::Insert => store::insert(conn, op).await?,
        OpKind::Update => {
            store::update(conn, op).await?;
        }
        OpKind::Archive => {
            store::set_archived(conn, &op.aggregate, &[op.id], at).await?;
            let mut pending = vec![(op.aggregate.clone(), op.id)];
            while let Some((aggregate, id)) = pending.pop() {
                for r in aggregates
                    .relations()
                    .iter()
                    .filter(|r| r.target == aggregate && r.on_delete == OnDelete::Archive)
                {
                    let ids = store::active_ids_where(conn, &r.source, &r.field, id).await?;
                    store::set_archived(conn, &r.source, &ids, at).await?;
                    *out.cascade.entry(r.from.clone()).or_default() += ids.len() as u64;
                    for child in ids {
                        out.effects
                            .push(format!("{}/{child}.{TOMBSTONE}", r.source));
                        pending.push((r.source.clone(), child));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Contract J's `cascade`, every `onDelete: archive` relation into the
/// archived aggregate listed, with zero rows when none was active.
pub fn cascade_entries(aggregates: &Aggregates, op: &Operation, applied: &Applied) -> Vec<Value> {
    if op.kind != OpKind::Archive {
        return Vec::new();
    }
    aggregates
        .relations()
        .iter()
        .filter(|r| r.target == op.aggregate && r.on_delete == OnDelete::Archive)
        .map(|r| {
            json!({
                "relation": r.from,
                "rows": applied.cascade.get(&r.from).copied().unwrap_or(0),
                "policy": "archive",
            })
        })
        .collect()
}

/// Contract J.
pub struct Plan<'a> {
    pub command: Uuid,
    pub op: &'a Operation,
    pub holds: Vec<Value>,
    pub pending: Vec<Uuid>,
    pub cascade: Vec<Value>,
    pub violations: &'a [String],
    pub decision: &'a str,
    pub reasons: Vec<String>,
}

impl Plan<'_> {
    pub fn to_json(&self) -> Value {
        let classification = match self.op.kind {
            OpKind::Insert => "insert",
            OpKind::Update | OpKind::Archive => "update",
        };
        json!({
            "command": self.command.to_string(),
            "classification": classification,
            "dependencies": {
                // The persisted-query registry that would name readers is
                // not part of the engine yet.
                "readers": [],
                "holds": self.holds,
                "pendingCommands": self.pending.iter().map(Uuid::to_string).collect::<Vec<_>>(),
            },
            "cascade": self.cascade,
            "invalidates": [],
            "invariantsAfter": if self.violations.is_empty() { "ok" } else { "violated" },
            "decision": self.decision,
            "reasons": self.reasons,
        })
    }
}
