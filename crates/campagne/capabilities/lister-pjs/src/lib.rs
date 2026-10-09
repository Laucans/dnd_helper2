//! `campagne.listerPjs`: the active PCs of one campaign, oldest first.
//!
//! The Capability reads and only reads, through the one persisted query its
//! manifest declares (`ListePjs`). Isolation, tombstones and order all live in
//! the Data layer: the views expose active rows only and the query orders them.
//! This crate checks that the id it was given is an id, hands it to the read
//! port, and renames the columns of the answer. It filters, sorts and
//! normalises nothing.

use std::future::Future;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

pub use campagne_persisted_query::{Executor, QueryError, StartError, registry_dir};

/// The identifier a caller reaches this Capability by.
pub const CAPABILITY: &str = "campagne.listerPjs";
/// The registered name of the only query this Capability runs.
pub const QUERY: &str = "ListePjs";
/// The query's only variable, and the only key of the Capability's input.
const VARIABLE: &str = "campagneId";

/// The one read the Capability does. [`Executor`] implements it for real.
pub trait ReadPort {
    fn run(
        &self,
        query: &str,
        variables: &Map<String, Value>,
    ) -> impl Future<Output = Result<Value, QueryError>> + Send;
}

impl ReadPort for Executor {
    async fn run(&self, query: &str, variables: &Map<String, Value>) -> Result<Value, QueryError> {
        Executor::run(self, query, variables).await
    }
}

/// A PC as the Capability exposes it, and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pj {
    pub id: Uuid,
    pub name: String,
    pub class: String,
    pub level: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum ListerPjsError {
    /// The campaign id is missing, empty, not a string or not an id, or the
    /// input carries another key. No query ran.
    #[error("invalid input: campagneId must be a campaign id")]
    InvalidInput,
    /// An unknown, archived or foreign campaign: one answer for the three.
    #[error("not found")]
    NotFound,
    /// The read itself failed. Never mapped to [`ListerPjsError::NotFound`]
    /// nor to an empty list.
    #[error("read failed")]
    ReadFailed(#[source] ReadFailure),
}

#[derive(Debug, thiserror::Error)]
pub enum ReadFailure {
    /// Every [`QueryError`] but the not-found answer.
    #[error(transparent)]
    Query(QueryError),
    /// The answer is not `{"pjs": [{id, nom, classe, niveau}, ...]}`.
    #[error("unexpected answer shape")]
    Answer,
}

/// The answer of `ListePjs`; keys not named here (`creeLe`) are dropped.
#[derive(Deserialize)]
struct Answer {
    pjs: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    id: Uuid,
    nom: String,
    classe: String,
    niveau: i32,
}

pub struct ListerPjs<P> {
    port: P,
}

impl ListerPjs<Executor> {
    /// Starts on the read-only connection of the environment only. A missing
    /// or blank variable is a [`StartError`]; there is no other source.
    pub async fn from_env() -> Result<Self, StartError> {
        Ok(Self::new(Executor::from_env(&registry_dir()).await?))
    }
}

impl<P: ReadPort> ListerPjs<P> {
    pub fn new(port: P) -> Self {
        Self { port }
    }

    /// The active PCs of `campagne_id`, in the order the query returns them.
    pub async fn lister(&self, campagne_id: &str) -> Result<Vec<Pj>, ListerPjsError> {
        let id = Uuid::try_parse(campagne_id).map_err(|_| ListerPjsError::InvalidInput)?;
        let mut variables = Map::new();
        variables.insert(VARIABLE.to_owned(), Value::String(id.to_string()));

        let answer = match self.port.run(QUERY, &variables).await {
            Ok(answer) => answer,
            Err(QueryError::NotFound) => return Err(ListerPjsError::NotFound),
            Err(other) => return Err(ListerPjsError::ReadFailed(ReadFailure::Query(other))),
        };
        let Answer { pjs } = serde_json::from_value(answer)
            .map_err(|_| ListerPjsError::ReadFailed(ReadFailure::Answer))?;
        Ok(pjs
            .into_iter()
            .map(|row| Pj {
                id: row.id,
                name: row.nom,
                class: row.classe,
                level: row.niveau,
            })
            .collect())
    }

    /// The by-identifier entry point: `input` is an object holding exactly
    /// the key `campagneId`, a string. Returns the JSON array of PCs.
    pub async fn call(&self, input: &Value) -> Result<Value, ListerPjsError> {
        let Some(object) = input.as_object() else {
            return Err(ListerPjsError::InvalidInput);
        };
        let (1, Some(Value::String(campagne_id))) = (object.len(), object.get(VARIABLE)) else {
            return Err(ListerPjsError::InvalidInput);
        };
        let pjs = self.lister(campagne_id).await?;
        serde_json::to_value(pjs).map_err(|_| ListerPjsError::ReadFailed(ReadFailure::Answer))
    }
}
