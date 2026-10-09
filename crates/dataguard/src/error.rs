//! Errors carry an id or a SQLSTATE, never a driver's message: it can quote a
//! role, a host or a database name.

use crate::manifest::ManifestError;

/// The SQLSTATE of a driver error, or `n/a`. Nothing else of it is kept.
pub fn sqlstate(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .map_or_else(|| "n/a".to_owned(), |code| code.into_owned())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EngineError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("database error (SQLSTATE {0})")]
    Database(String),
    #[error("a stored command does not parse: {0}")]
    Corrupt(&'static str),
}

impl From<sqlx::Error> for EngineError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(sqlstate(&e))
    }
}

/// Why a submission was refused before any queue entry exists.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SubmitError {
    #[error("unknown-capability")]
    UnknownCapability,
    #[error("malformed-submission ({0})")]
    Malformed(&'static str),
    #[error("idempotency-key-required")]
    IdempotencyKeyRequired,
    #[error("idempotency-key-conflict")]
    IdempotencyKeyConflict,
    #[error("based-on-ahead")]
    BasedOnAhead,
    #[error("internal (SQLSTATE {0})")]
    Internal(String),
}

impl SubmitError {
    /// The stable id an HTTP answer carries.
    pub fn id(&self) -> &'static str {
        match self {
            SubmitError::UnknownCapability => "unknown-capability",
            SubmitError::Malformed(_) => "malformed-submission",
            SubmitError::IdempotencyKeyRequired => "idempotency-key-required",
            SubmitError::IdempotencyKeyConflict => "idempotency-key-conflict",
            SubmitError::BasedOnAhead => "based-on-ahead",
            SubmitError::Internal(_) => "internal",
        }
    }
}

impl From<EngineError> for SubmitError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Database(code) => Self::Internal(code),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<sqlx::Error> for SubmitError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(sqlstate(&e))
    }
}

/// Confirm and cancel.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OperationError {
    #[error("not-found")]
    NotFound,
    #[error("not-author")]
    NotAuthor,
    #[error("internal (SQLSTATE {0})")]
    Internal(String),
}

impl From<EngineError> for OperationError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Database(code) => Self::Internal(code),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<sqlx::Error> for OperationError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(sqlstate(&e))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HoldError {
    #[error("the ttl is above the aggregate's holds.maxTtl")]
    AboveMaxTtl,
    #[error("the hold has expired")]
    Expired,
    #[error("the aggregate's holds are not renewable")]
    NotRenewable,
    #[error("not-found")]
    NotFound,
    #[error("not-author")]
    NotAuthor,
    #[error("internal (SQLSTATE {0})")]
    Internal(String),
}

impl From<sqlx::Error> for HoldError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(sqlstate(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_driver_message_never_reaches_an_error() {
        // A driver error that is not a database error can quote the URL it
        // was given; only its SQLSTATE, here none, may be kept.
        let url = "postgres://postgres:s3cret@db.internal:5432/app";
        let errors: [fn(&str) -> sqlx::Error; 2] = [
            |u| sqlx::Error::Configuration(u.into()),
            |u| sqlx::Error::Protocol(u.into()),
        ];
        for driver in errors {
            assert_eq!(sqlstate(&driver(url)), "n/a");
            for shown in [
                EngineError::from(driver(url)).to_string(),
                SubmitError::from(driver(url)).to_string(),
                SubmitError::from(EngineError::from(driver(url))).to_string(),
                OperationError::from(driver(url)).to_string(),
                OperationError::from(EngineError::from(driver(url))).to_string(),
                HoldError::from(driver(url)).to_string(),
            ] {
                assert!(!shown.contains("s3cret"), "{shown}");
                assert!(!shown.contains("db.internal"), "{shown}");
                assert!(shown.ends_with("(SQLSTATE n/a)"), "{shown}");
            }
        }
    }
}
