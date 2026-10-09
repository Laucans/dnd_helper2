//! The `dataVersion` push: the applier notifies inside its transaction, so a
//! notification is delivered only once the data it announces is committed;
//! one listener per process turns them into a [`watch`] value that only ever
//! grows. Its payload is the version and nothing else.

use std::future::Future;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::watch;

use crate::engine::current_version;
use crate::error::EngineError;

pub const DATA_VERSION_CHANNEL: &str = "data_version";

pub struct VersionFeed;

impl VersionFeed {
    /// Seeds with the current version, then follows the notifications. After
    /// a reconnect, when notifications may have been missed, it reads the
    /// version again. Coalescing is allowed; going back is not. The task ends
    /// when every receiver is dropped, or at `shutdown`: the receivers then
    /// see the feed close, so a stream built on it ends too.
    pub async fn spawn(
        pool: PgPool,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<watch::Receiver<i64>, EngineError> {
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.listen(DATA_VERSION_CHANNEL).await?;
        let seed = current_version(&mut *pool.acquire().await?).await?;
        let (tx, rx) = watch::channel(seed);
        tokio::spawn(async move {
            tokio::pin!(shutdown);
            loop {
                tokio::select! {
                    _ = &mut shutdown => return,
                    _ = tx.closed() => return,
                    next = listener.try_recv() => match next {
                        Ok(Some(n)) => {
                            if let Ok(v) = n.payload().parse::<i64>() {
                                raise(&tx, v);
                            }
                        }
                        Ok(None) => {
                            if let Ok(mut conn) = pool.acquire().await
                                && let Ok(v) = current_version(&mut conn).await
                            {
                                raise(&tx, v);
                            }
                        }
                        Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
                    },
                }
            }
        });
        Ok(rx)
    }
}

/// Publishes `v` only when it is ahead of what subscribers have seen.
fn raise(tx: &watch::Sender<i64>, v: i64) {
    tx.send_if_modified(|seen| {
        if v > *seen {
            *seen = v;
            true
        } else {
            false
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subscriber_never_sees_a_version_go_back() {
        let (tx, mut rx) = watch::channel(3);
        raise(&tx, 2);
        assert!(!rx.has_changed().unwrap());
        raise(&tx, 5);
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), 5);
        raise(&tx, 5);
        assert!(!rx.has_changed().unwrap());
    }
}
