//! The read executor: a registered query name and its variables in, one JSON
//! value out. Nothing else reaches the database.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde_json::{Map, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;

use crate::operation;
use crate::registry::{self, Entry};

/// The only variable the executor reads its connection from.
pub const DATABASE_URL_READONLY: &str = "DATABASE_URL_READONLY";

/// Opens a transaction that sees one snapshot and cannot write.
const BEGIN_SNAPSHOT: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// Unset, empty or blank.
    #[error("{0} is not set")]
    Missing(&'static str),
    /// The URL parser's message is dropped: it could quote the value.
    #[error("{0} is malformed")]
    Malformed(&'static str),
    #[error("invalid query registry: {0}")]
    Registry(String),
    /// `cause` is a coarse label (an SQLSTATE or a kind), never the driver's
    /// message: that one can quote the host or the user.
    #[error("could not connect through DATABASE_URL_READONLY ({cause})")]
    Connect { cause: String },
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// The name is not registered. Never echoes what was asked for.
    #[error("unknown query")]
    Unknown,
    #[error("query {query} does not match its registered hash")]
    Tampered { query: String },
    #[error("query {query}: {reason}")]
    Unavailable { query: String, reason: String },
    #[error("query {query}: invalid variable {variable}")]
    InvalidInput { query: String, variable: String },
    /// An unknown, archived, foreign or malformed campaign id. No payload, so
    /// every cause answers byte for byte the same.
    #[error("not found")]
    NotFound,
    #[error("database error")]
    Database(#[source] sqlx::Error),
}

/// `crates/campagne/queries`, resolved at compile time from this crate's place
/// in the workspace.
pub fn registry_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits under crates/campagne")
        .join("queries")
}

/// The exact options the executor connects with.
///
/// `PgConnectOptions` derives `Debug`, password included: never format it.
#[doc(hidden)]
pub fn connect_options(
    vars: impl Fn(&str) -> Option<String>,
) -> Result<PgConnectOptions, StartError> {
    let url = vars(DATABASE_URL_READONLY)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .ok_or(StartError::Missing(DATABASE_URL_READONLY))?;
    let options = PgConnectOptions::from_str(&url)
        .map_err(|_| StartError::Malformed(DATABASE_URL_READONLY))?;
    Ok(options
        // Defence in depth: the proof of read-only is the role's privileges.
        .options([
            ("default_transaction_read_only", "on"),
            ("TimeZone", "UTC"),
            // A stuck statement or client must not hold a snapshot (and one of
            // the five pooled connections) for ever.
            ("statement_timeout", "10000"),
            ("idle_in_transaction_session_timeout", "10000"),
        ])
        .application_name("campagne-persisted-query"))
}

pub struct Executor {
    pool: PgPool,
    dir: PathBuf,
    entries: Vec<Entry>,
}

impl fmt::Debug for Executor {
    /// The directory and the query names, never the pool: its options hold the
    /// connection string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Executor")
            .field("dir", &self.dir)
            .field(
                "queries",
                &self.entries.iter().map(|e| &e.query).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl Executor {
    /// Reads `DATABASE_URL_READONLY` from the process environment.
    pub async fn from_env(dir: &Path) -> Result<Self, StartError> {
        Self::from_variables(dir, |name| std::env::var(name).ok()).await
    }

    /// The same, from any lookup: tests use it instead of mutating the process
    /// environment.
    pub async fn from_variables(
        dir: &Path,
        vars: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, StartError> {
        let options = connect_options(vars)?;
        let entries = registry::load(dir)?;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(|e| StartError::Connect { cause: coarse(&e) })?;
        Ok(Self {
            pool,
            dir: dir.to_owned(),
            entries,
        })
    }

    /// The only entry point: a registered operation name and its variables.
    /// Nothing reaches the database before the last step.
    pub async fn run(
        &self,
        query: &str,
        variables: &Map<String, Value>,
    ) -> Result<Value, QueryError> {
        // 1. Free text, SQL or GraphQL alike, can only land here.
        let entry = self
            .entries
            .iter()
            .find(|e| e.query == query)
            .ok_or(QueryError::Unknown)?;
        let name = entry.query.as_str();
        let unavailable = |reason: &str| QueryError::Unavailable {
            query: name.to_owned(),
            reason: reason.to_owned(),
        };

        // 2. Both files resolve inside the registry directory.
        let root = self
            .dir
            .canonicalize()
            .map_err(|_| unavailable("the registry directory is missing"))?;
        let graphql_path = self.resolve(&root, Path::new(&entry.file), &unavailable)?;
        let sql_path = self.resolve(
            &root,
            &Path::new(&entry.file).with_extension("sql"),
            &unavailable,
        )?;

        // 3. The hash of the bytes on disk, now. No cache, no earlier answer.
        let graphql = std::fs::read(&graphql_path)
            .map_err(|_| unavailable("the query file cannot be read"))?;
        let sql =
            std::fs::read(&sql_path).map_err(|_| unavailable("the query file cannot be read"))?;
        if registry::digest(&graphql, &sql) != entry.sha256 {
            return Err(QueryError::Tampered {
                query: name.to_owned(),
            });
        }
        let graphql =
            String::from_utf8(graphql).map_err(|_| unavailable("the query file is not UTF-8"))?;
        let sql = String::from_utf8(sql).map_err(|_| unavailable("the query file is not UTF-8"))?;

        // 4. The header declares the variables.
        let operation = operation::parse_header(&graphql).map_err(|reason| unavailable(&reason))?;
        if operation.name != entry.query {
            return Err(unavailable("the operation name differs from the registry"));
        }

        // 5. Exactly the declared variables, each a string.
        let invalid = |variable: &str| QueryError::InvalidInput {
            query: name.to_owned(),
            variable: variable.to_owned(),
        };
        // The name of an undeclared key is the caller's text: not echoed.
        if variables.keys().any(|k| !operation.variables.contains(k)) {
            return Err(invalid("(undeclared)"));
        }
        let mut ids = Vec::with_capacity(operation.variables.len());
        for variable in &operation.variables {
            match variables.get(variable) {
                Some(Value::String(id)) => ids.push(id),
                _ => return Err(invalid(variable)),
            }
        }

        // 6. A string that is not an id is an unknown id: no cast error, no SQL.
        let ids = ids
            .into_iter()
            .map(|id| Uuid::parse_str(id).map_err(|_| QueryError::NotFound))
            .collect::<Result<Vec<_>, _>>()?;

        // 7. One statement, one snapshot, bound parameters only.
        let mut tx = self
            .pool
            .begin_with(BEGIN_SNAPSHOT)
            .await
            .map_err(QueryError::Database)?;
        // The text is a registered file whose hash was just checked.
        let mut statement = sqlx::query(AssertSqlSafe(sql));
        for id in ids {
            statement = statement.bind(id);
        }
        let row = statement
            .fetch_one(&mut *tx)
            .await
            .map_err(QueryError::Database)?;
        let answer: Option<Value> = row.try_get(0).map_err(QueryError::Database)?;
        tx.commit().await.map_err(QueryError::Database)?;
        answer.ok_or(QueryError::NotFound)
    }

    /// `relative` under `root`, canonical, and still inside `root` once
    /// symlinks are followed.
    fn resolve(
        &self,
        root: &Path,
        relative: &Path,
        unavailable: &dyn Fn(&str) -> QueryError,
    ) -> Result<PathBuf, QueryError> {
        let path = root
            .join(relative)
            .canonicalize()
            .map_err(|_| unavailable("the query file is missing"))?;
        if path.starts_with(root) {
            Ok(path)
        } else {
            Err(unavailable(
                "the query file resolves outside the registry directory",
            ))
        }
    }
}

/// A label for a connection failure that quotes neither the host, the user nor
/// the password.
fn coarse(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(e) => format!(
            "database refused, SQLSTATE {}",
            e.code().unwrap_or_default()
        ),
        sqlx::Error::Io(_) => "server unreachable".to_owned(),
        sqlx::Error::Tls(_) => "TLS failure".to_owned(),
        sqlx::Error::PoolTimedOut => "timed out".to_owned(),
        _ => "other".to_owned(),
    }
}
