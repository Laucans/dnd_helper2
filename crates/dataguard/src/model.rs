//! What a command is, from submission to its terminal result: the submission
//! a client sends, the write a DataCapability turns it into, the queue entry
//! (contract G) and the result (contract H).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommandId(pub Uuid);

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for CommandId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

/// Contract G's states. `awaiting_review` is declared but unreachable here:
/// every invariant rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    AwaitingConfirmation,
    Confirmed,
    AwaitingReview,
    Parked,
    Applied,
    Rejected,
    Cancelled,
    Expired,
}

impl State {
    pub const ALL: [State; 9] = [
        State::Queued,
        State::AwaitingConfirmation,
        State::Confirmed,
        State::AwaitingReview,
        State::Parked,
        State::Applied,
        State::Rejected,
        State::Cancelled,
        State::Expired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            State::Queued => "queued",
            State::AwaitingConfirmation => "awaiting_confirmation",
            State::Confirmed => "confirmed",
            State::AwaitingReview => "awaiting_review",
            State::Parked => "parked",
            State::Applied => "applied",
            State::Rejected => "rejected",
            State::Cancelled => "cancelled",
            State::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == s)
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            State::Applied | State::Rejected | State::Cancelled | State::Expired
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub id: Uuid,
}

/// The `dataVersion` a command was built on, and what its author saw then.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasedOn {
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Map<String, Value>>,
}

/// What a client submits. Unknown fields, a `by` among them, are ignored: the
/// engine sets the author.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Submission {
    /// `<system>.<name>@<version>`.
    pub data_capability: String,
    #[serde(default)]
    pub target: Option<Target>,
    #[serde(default)]
    pub payload: Map<String, Value>,
    #[serde(default)]
    pub based_on: Option<BasedOn>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Why a DataCapability, or the engine, cannot make a write of a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed(pub &'static str);

/// The write a DataCapability asks for, in its aggregate's field names.
#[derive(Debug, Clone, PartialEq)]
pub struct Write {
    pub aggregate: String,
    pub change: Change,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Every field of the new row but the tombstone; the engine mints its id.
    Insert(Map<String, Value>),
    /// The fields present change; an absent field is unchanged.
    Update {
        id: Uuid,
        fields: Map<String, Value>,
    },
    /// Sets the tombstone to the engine's clock; the cascade follows.
    Archive { id: Uuid },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Insert,
    Update,
    Archive,
}

/// A write as the queue stores it: on a row known by id, the insert's id
/// minted at submission so its partition is known before it applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub aggregate: String,
    pub id: Uuid,
    pub kind: OpKind,
    pub fields: Map<String, Value>,
}

impl Operation {
    pub fn partition(&self) -> String {
        format!("{}/{}", self.aggregate, self.id)
    }

    /// `<Aggregate>/<id>.<field>` for each touched field: what staleness
    /// compares.
    pub fn effects<'a>(&self, touched: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        touched
            .into_iter()
            .map(|f| format!("{}.{f}", self.partition()))
            .collect()
    }
}

/// Contract G.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    pub command: String,
    pub data_capability: String,
    pub by: String,
    pub partition: String,
    pub position: i64,
    pub based_on: BasedOn,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub your_value: Option<Value>,
    pub state: State,
    pub confirmation: Option<Value>,
    pub parked: Option<Value>,
    pub requeued_from: Option<String>,
}

/// Contract H.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub command_id: String,
    pub status: State,
    pub data_version: Option<i64>,
    pub violations: Vec<String>,
    pub review_id: Option<String>,
}

/// A command as its author can see it: its queue entry while it is pending,
/// its result once it is settled. Contract G forbids extra properties, so the
/// messages travel beside the entry, not in it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Lookup {
    Pending {
        entry: Box<QueueEntry>,
        messages: Vec<Value>,
    },
    Settled {
        result: CommandResult,
    },
}

impl Lookup {
    pub fn state(&self) -> State {
        match self {
            Lookup::Pending { entry, .. } => entry.state,
            Lookup::Settled { result } => result.status,
        }
    }

    pub fn result(&self) -> Option<&CommandResult> {
        match self {
            Lookup::Settled { result } => Some(result),
            Lookup::Pending { .. } => None,
        }
    }

    pub fn entry(&self) -> Option<&QueueEntry> {
        match self {
            Lookup::Pending { entry, .. } => Some(entry),
            Lookup::Settled { .. } => None,
        }
    }
}

/// The answer to a submission: it never waits for the applier.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Submitted {
    pub command_id: CommandId,
    /// `<Aggregate>/<id>`: a create's id is minted at submission, so the
    /// client knows it at once, and a replay returns the same one.
    pub partition: String,
    pub replayed: bool,
    /// Invariants at risk on the projected state; the application decides.
    pub warnings: Vec<String>,
    #[serde(flatten)]
    pub lookup: Lookup,
}

/// A hold on a partition: recorded, reported in impact plans, and never in
/// the way of a command.
#[derive(Debug, Clone, PartialEq)]
pub struct Hold {
    pub id: Uuid,
    pub partition: String,
    pub by: String,
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_round_trip_and_four_are_terminal() {
        for state in State::ALL {
            assert_eq!(State::parse(state.as_str()), Some(state));
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                Value::String(state.as_str().into())
            );
        }
        let terminal: Vec<_> = State::ALL.into_iter().filter(|s| s.is_terminal()).collect();
        assert_eq!(
            terminal,
            [
                State::Applied,
                State::Rejected,
                State::Cancelled,
                State::Expired
            ]
        );
    }

    #[test]
    fn a_submission_ignores_a_client_by() {
        let s: Submission = serde_json::from_str(
            r#"{"dataCapability":"test.x@1","payload":{"a":1},"by":"intrus","idempotencyKey":"k"}"#,
        )
        .unwrap();
        assert_eq!(s.data_capability, "test.x@1");
        assert_eq!(s.idempotency_key.as_deref(), Some("k"));
        assert!(
            serde_json::from_str::<Submission>(
                r#"{"dataCapability":"t.x@1","basedOn":{"version":-1}}"#
            )
            .is_err()
        );
    }
}
