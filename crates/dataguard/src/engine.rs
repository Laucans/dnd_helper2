//! The engine's front door: submission, lookup, confirm, cancel, expiry,
//! holds, and the one applier.

use std::sync::Arc;
use std::time::Duration;

use sqlx::{PgConnection, PgPool};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::applier::Applier;
use crate::clock::Clock;
use crate::error::{EngineError, HoldError, OperationError, SubmitError};
use crate::manifest::Aggregates;
use crate::model::{CommandId, Hold, Lookup, Submission, Submitted};
use crate::queue;
use crate::registry::Registry;

pub(crate) struct Inner {
    /// The write role (`DATABASE_URL`).
    pub pool: PgPool,
    pub aggregates: Aggregates,
    pub registry: Registry,
    pub clock: Arc<dyn Clock>,
    /// Wakes the applier when a command may have become ready.
    pub wake: Notify,
}

#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl Engine {
    /// Validates the embedded aggregates: a relation the engine cannot
    /// apply stops it here, before anything listens.
    pub fn new(
        pool: PgPool,
        registry: Registry,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, EngineError> {
        Ok(Self::with_aggregates(
            pool,
            Aggregates::embedded()?,
            registry,
            clock,
        ))
    }

    /// An engine on aggregates already validated.
    pub fn with_aggregates(
        pool: PgPool,
        aggregates: Aggregates,
        registry: Registry,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                aggregates,
                registry,
                clock,
                wake: Notify::new(),
            }),
        }
    }

    pub fn aggregates(&self) -> &Aggregates {
        &self.inner.aggregates
    }

    /// Queues a command and returns at once; it never waits for the applier
    /// and is never refused for concurrency.
    pub async fn submit(&self, s: Submission) -> Result<Submitted, SubmitError> {
        let submitted = queue::submit(&self.inner, s).await?;
        self.inner.wake.notify_one();
        Ok(submitted)
    }

    pub async fn lookup(&self, id: CommandId) -> Result<Option<Lookup>, EngineError> {
        let mut conn = self.inner.pool.acquire().await?;
        queue::lookup(&mut conn, id.0).await
    }

    /// Confirms an overwrite. On a parked command, requeues it at the end of
    /// its partition; at or after its ttl, it is expired instead.
    pub async fn confirm(&self, id: CommandId, by: &str) -> Result<Lookup, OperationError> {
        let out = queue::confirm(&self.inner, id.0, by).await?;
        self.inner.wake.notify_one();
        Ok(out)
    }

    /// Cancels a queued, awaiting or parked command. A terminal one keeps its
    /// result, and one already inside its applying transaction ends applied.
    pub async fn cancel(&self, id: CommandId, by: &str) -> Result<Lookup, OperationError> {
        let out = queue::cancel(&self.inner, id.0, by).await?;
        self.inner.wake.notify_one();
        Ok(out)
    }

    /// Parked commands at or past their ttl become `expired`.
    pub async fn expire_due(&self) -> Result<u64, EngineError> {
        queue::expire_due(&self.inner).await
    }

    pub async fn hold(&self, partition: &str, by: &str, ttl: Duration) -> Result<Hold, HoldError> {
        queue::hold(&self.inner, partition, by, ttl).await
    }

    pub async fn renew_hold(&self, hold: Uuid, by: &str, ttl: Duration) -> Result<Hold, HoldError> {
        queue::renew_hold(&self.inner, hold, by, ttl).await
    }

    pub async fn current_version(&self) -> Result<i64, EngineError> {
        let mut conn = self.inner.pool.acquire().await?;
        Ok(current_version(&mut conn).await?)
    }

    /// The single applier, or `None` while another one holds the lock.
    pub async fn applier(&self) -> Result<Option<Applier>, EngineError> {
        Applier::acquire(self.inner.clone()).await
    }
}

pub(crate) async fn current_version(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT version FROM dataguard_version")
        .fetch_one(conn)
        .await
}
