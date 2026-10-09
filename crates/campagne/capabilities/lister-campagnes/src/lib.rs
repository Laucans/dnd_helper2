//! The Capability `campagne.listerCampagnes`: the active campaigns of the GM,
//! newest first, as `{ id, name }`.
//!
//! It is a projection and nothing more. It asks the read executor for the
//! registered query [`QUERY`], which already hides the archived campaigns and
//! orders the rows; the Capability keeps that order and renames `nom` to
//! `name`. It filters, sorts and trims nothing, and it connects through
//! `DATABASE_URL_READONLY` only.

use std::future::Future;
use std::path::Path;

use campagne_persisted_query::{Executor, registry_dir};
pub use campagne_persisted_query::{QueryError, StartError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The identifier a Micro-UI calls, as in `capability.json`.
pub const CAPABILITY: &str = "campagne.listerCampagnes";

/// The one persisted query this Capability reads through.
pub const QUERY: &str = "ListeCampagnes";

/// One entry of the answer: the campaign's id as the store spells it, and its
/// name as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Campagne {
    pub id: String,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ListerError {
    /// The read failed or was refused. Never an empty list.
    #[error(transparent)]
    Query(#[from] QueryError),
    /// The query answered with something other than its registered shape.
    #[error("ListeCampagnes: unexpected answer shape")]
    Shape,
}

/// What the Capability reads through: [`Executor`] in production, a fake in
/// tests. Declared with an explicit `impl Future` so the trait can be public;
/// `Sync` keeps the future `Send`, so a server may await it on any thread.
pub trait Lecture: Sync {
    fn run(
        &self,
        query: &str,
        variables: &Map<String, Value>,
    ) -> impl Future<Output = Result<Value, QueryError>> + Send;
}

impl Lecture for Executor {
    async fn run(&self, query: &str, variables: &Map<String, Value>) -> Result<Value, QueryError> {
        Executor::run(self, query, variables).await
    }
}

/// Starts on `DATABASE_URL_READONLY` and the repository's query registry.
pub async fn demarrer() -> Result<Executor, StartError> {
    Executor::from_env(&registry_dir()).await
}

/// The same from any lookup and any registry directory, so a test starts the
/// Capability without touching the process environment.
pub async fn demarrer_avec(
    dir: &Path,
    vars: impl Fn(&str) -> Option<String>,
) -> Result<Executor, StartError> {
    Executor::from_variables(dir, vars).await
}

#[derive(Deserialize)]
struct Answer {
    campagnes: Vec<Row>,
}

/// `creeLe` is read by the query to order the rows and is not part of the
/// output, so it is not read here.
#[derive(Deserialize)]
struct Row {
    id: String,
    nom: String,
}

/// The active campaigns, in the order the query returned them.
pub async fn lister_campagnes(lecture: &impl Lecture) -> Result<Vec<Campagne>, ListerError> {
    let answer = lecture.run(QUERY, &Map::new()).await?;
    let Answer { campagnes } = serde_json::from_value(answer).map_err(|_| ListerError::Shape)?;
    Ok(campagnes
        .into_iter()
        .map(|Row { id, nom }| Campagne { id, name: nom })
        .collect())
}
