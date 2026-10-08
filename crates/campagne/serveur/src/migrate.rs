//! The migration runner: applies the embedded migrations in order, each in its
//! own transaction with its bookkeeping row, and refuses to go on when an
//! applied migration changed or disappeared.
//!
//! Errors carry a file name and a SQLSTATE, never the server's message, which
//! can quote a role, a host or a database name.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use sqlx::pool::PoolConnection;
use sqlx::{AssertSqlSafe, Connection, PgPool, Postgres};
use tracing::info;

pub const BOOKKEEPING_TABLE: &str = "schema_migrations";

/// Key of the advisory lock that serialises two runners on one database.
const LOCK_KEY: i64 = 0x6361_6d70_6d69_6772; // "campmigr"

/// How long a runner waits for another one before it gives up.
pub const LOCK_WAIT: Duration = Duration::from_secs(60);
const LOCK_POLL: Duration = Duration::from_millis(100);

pub struct Migration<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

impl std::fmt::Debug for Migration<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("name", &self.name)
            .field("len", &self.bytes.len())
            .finish()
    }
}

impl Migration<'_> {
    /// Lowercase hex SHA-256 of the exact raw bytes.
    pub fn checksum(&self) -> String {
        hex::encode(Sha256::digest(self.bytes))
    }

    fn prefix(&self) -> u16 {
        // Only called after `validate`, which proved four ASCII digits.
        self.name[..4].parse().expect("validated prefix")
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub applied: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MigrationError {
    #[error("migration file {0} does not match NNNN_snake_name.sql")]
    InvalidName(String),
    #[error("migration file {0} duplicates the numeric prefix of another file")]
    DuplicatePrefix(String),
    #[error("migration file {0} is not valid UTF-8")]
    NotUtf8(String),
    #[error("applied migration {0} has changed since it was applied")]
    ChecksumMismatch(String),
    #[error("applied migration {0} is missing from this build")]
    MissingFromBuild(String),
    #[error("migration {0} is older than the last applied migration")]
    OutOfOrder(String),
    #[error("migration {name} failed (SQLSTATE {sqlstate})")]
    Failed { name: String, sqlstate: String },
    #[error("migration bookkeeping failed (SQLSTATE {0})")]
    Bookkeeping(String),
    #[error("another migration runner held the migration lock for {0:?}")]
    LockBusy(Duration),
}

/// The SQLSTATE of a driver error, or `n/a`. Nothing else of it is kept.
pub fn sqlstate(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .map_or_else(|| "n/a".to_owned(), |code| code.into_owned())
}

fn bookkeeping(error: sqlx::Error) -> MigrationError {
    MigrationError::Bookkeeping(sqlstate(&error))
}

fn valid_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".sql") else {
        return false;
    };
    let bytes = stem.as_bytes();
    bytes.len() > 5
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'_'
        && bytes[5..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// Checks every name, prefix and encoding, and returns the set sorted by its
/// numeric prefix. Touches no database.
pub fn validate<'a>(set: &'a [Migration<'a>]) -> Result<Vec<&'a Migration<'a>>, MigrationError> {
    let mut seen = HashSet::new();
    for migration in set {
        if !valid_name(migration.name) {
            return Err(MigrationError::InvalidName(migration.name.to_owned()));
        }
        if !seen.insert(migration.prefix()) {
            return Err(MigrationError::DuplicatePrefix(migration.name.to_owned()));
        }
        if std::str::from_utf8(migration.bytes).is_err() {
            return Err(MigrationError::NotUtf8(migration.name.to_owned()));
        }
    }
    let mut sorted: Vec<_> = set.iter().collect();
    sorted.sort_by_key(|m| m.prefix());
    Ok(sorted)
}

/// Applies every pending migration of `set`, in order. When nothing is
/// pending, writes nothing.
pub async fn run(pool: &PgPool, set: &[Migration<'_>]) -> Result<MigrationReport, MigrationError> {
    run_with_lock_wait(pool, set, LOCK_WAIT).await
}

/// [`run`], waiting at most `wait` for another runner's lock.
pub async fn run_with_lock_wait(
    pool: &PgPool,
    set: &[Migration<'_>],
    wait: Duration,
) -> Result<MigrationReport, MigrationError> {
    let sorted = validate(set)?;

    let mut conn = pool.acquire().await.map_err(bookkeeping)?;
    lock(&mut conn, wait).await?;

    let result = run_locked(&mut conn, &sorted).await;

    let unlocked = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(LOCK_KEY)
        .execute(&mut *conn)
        .await;
    if unlocked.is_err() {
        // A connection that may still hold the lock never goes back to the pool.
        conn.close_on_drop();
    }
    result
}

/// Takes the runner's advisory lock, polling so a stuck holder ends in an
/// error rather than a startup that never returns.
async fn lock(conn: &mut PoolConnection<Postgres>, wait: Duration) -> Result<(), MigrationError> {
    let deadline = Instant::now() + wait;
    loop {
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(LOCK_KEY)
            .fetch_one(&mut **conn)
            .await
            .map_err(bookkeeping)?;
        if locked {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(MigrationError::LockBusy(wait));
        }
        tokio::time::sleep(LOCK_POLL).await;
    }
}

async fn run_locked(
    conn: &mut PoolConnection<Postgres>,
    sorted: &[&Migration<'_>],
) -> Result<MigrationReport, MigrationError> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
           name text PRIMARY KEY,
           checksum text NOT NULL,
           applied_at timestamptz NOT NULL DEFAULT now()
         )",
    )
    .execute(&mut **conn)
    .await
    .map_err(bookkeeping)?;

    let applied: Vec<(String, String)> =
        sqlx::query_as("SELECT name, checksum FROM schema_migrations ORDER BY name")
            .fetch_all(&mut **conn)
            .await
            .map_err(bookkeeping)?;

    // Every applied migration is checked before anything is applied.
    let by_name: HashMap<&str, &Migration<'_>> = sorted.iter().map(|m| (m.name, *m)).collect();
    for (name, checksum) in &applied {
        match by_name.get(name.as_str()) {
            None => return Err(MigrationError::MissingFromBuild(name.clone())),
            Some(m) if m.checksum() != *checksum => {
                return Err(MigrationError::ChecksumMismatch(name.clone()));
            }
            Some(_) => {}
        }
    }

    let applied_names: HashSet<&str> = applied.iter().map(|(n, _)| n.as_str()).collect();
    let highest_applied = sorted
        .iter()
        .filter(|m| applied_names.contains(m.name))
        .map(|m| m.prefix())
        .max();
    let pending: Vec<&Migration<'_>> = sorted
        .iter()
        .copied()
        .filter(|m| !applied_names.contains(m.name))
        .collect();
    if let (Some(highest), Some(first)) = (highest_applied, pending.first())
        && first.prefix() < highest
    {
        return Err(MigrationError::OutOfOrder(first.name.to_owned()));
    }

    let mut report = MigrationReport::default();
    for migration in pending {
        apply(conn, migration).await?;
        info!("applied migration {}", migration.name);
        report.applied.push(migration.name.to_owned());
    }
    info!("{} migrations applied", report.applied.len());
    Ok(report)
}

/// One migration and its bookkeeping row, committed together or not at all.
async fn apply(
    conn: &mut PoolConnection<Postgres>,
    migration: &Migration<'_>,
) -> Result<(), MigrationError> {
    let failed = |error: sqlx::Error| MigrationError::Failed {
        name: migration.name.to_owned(),
        sqlstate: sqlstate(&error),
    };
    let body = std::str::from_utf8(migration.bytes).expect("validated UTF-8");

    let mut tx = conn.begin().await.map_err(failed)?;
    let outcome = async {
        // The body is an embedded migration file of this repository.
        sqlx::raw_sql(AssertSqlSafe(body)).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO schema_migrations (name, checksum) VALUES ($1, $2)")
            .bind(migration.name)
            .bind(migration.checksum())
            .execute(&mut *tx)
            .await?;
        Ok::<_, sqlx::Error>(())
    }
    .await;

    match outcome {
        Ok(()) => tx.commit().await.map_err(failed),
        Err(error) => {
            let _ = tx.rollback().await;
            Err(failed(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded::EMBEDDED;

    fn m(name: &'static str) -> Migration<'static> {
        Migration {
            name,
            bytes: b"SELECT 1;",
        }
    }

    #[test]
    fn name_pattern() {
        for ok in ["0001_campagne.sql", "0042_a_b_9.sql", "9999_x.sql"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "1_x.sql",
            "0001_A.sql",
            "0001_.sql",
            "0001-x.sql",
            "0001_x.SQL",
            "0001_x.sql.bak",
            "00001_x.sql",
            "abcd_x.sql",
            "0001_é.sql",
            "README.md",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn invalid_name_is_named() {
        let set = [m("0001_ok.sql"), m("0002_Bad.sql")];
        assert_eq!(
            validate(&set).unwrap_err(),
            MigrationError::InvalidName("0002_Bad.sql".into())
        );
        assert_eq!(
            validate(&set).unwrap_err().to_string(),
            "migration file 0002_Bad.sql does not match NNNN_snake_name.sql"
        );
    }

    #[test]
    fn duplicate_prefix_is_named() {
        let set = [m("0001_a.sql"), m("0001_b.sql")];
        assert_eq!(
            validate(&set).unwrap_err(),
            MigrationError::DuplicatePrefix("0001_b.sql".into())
        );
    }

    #[test]
    fn non_utf8_is_named() {
        let set = [Migration {
            name: "0001_a.sql",
            bytes: &[0xff, 0xfe],
        }];
        assert_eq!(
            validate(&set).unwrap_err(),
            MigrationError::NotUtf8("0001_a.sql".into())
        );
    }

    #[test]
    fn sorted_by_numeric_prefix() {
        let set = [m("0010_c.sql"), m("0002_b.sql"), m("0001_a.sql")];
        let names: Vec<_> = validate(&set).unwrap().iter().map(|m| m.name).collect();
        assert_eq!(names, ["0001_a.sql", "0002_b.sql", "0010_c.sql"]);
    }

    #[test]
    fn checksum_is_sha256_of_raw_bytes() {
        let lf = Migration {
            name: "0001_a.sql",
            bytes: b"SELECT 1;\n",
        };
        let crlf = Migration {
            name: "0001_a.sql",
            bytes: b"SELECT 1;\r\n",
        };
        assert_eq!(lf.checksum().len(), 64);
        assert_eq!(
            Migration {
                name: "0001_a.sql",
                bytes: b""
            }
            .checksum(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_ne!(lf.checksum(), crlf.checksum());
    }

    #[test]
    fn embedded_set_is_valid() {
        let names: Vec<_> = validate(EMBEDDED).unwrap().iter().map(|m| m.name).collect();
        assert_eq!(
            names,
            [
                "0001_campagne.sql",
                "0002_pj.sql",
                "0003_vues_lecture.sql",
                "0004_role_lecture.sql"
            ]
        );
    }

    #[test]
    fn errors_name_the_file_and_sqlstate_only() {
        let err = MigrationError::Failed {
            name: "0002_pj.sql".into(),
            sqlstate: "42P07".into(),
        };
        assert_eq!(
            err.to_string(),
            "migration 0002_pj.sql failed (SQLSTATE 42P07)"
        );
    }
}
