//! Startup, strictly in order: configuration, the engine's aggregates,
//! connection, migrations, then bind and listen. Nothing listens on a schema
//! that is not at its latest version, nor with a relation the engine cannot
//! apply.

use std::sync::Arc;
use std::time::Duration;

use dataguard::{Aggregates, Engine, EngineError, Registry, SystemClock, VersionFeed};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::config::{Config, ConfigError};
use crate::migrate::{self, Migration, MigrationError};
use crate::{embedded, http};

/// How long the server waits before trying again for the applier's lock,
/// while another instance holds it.
const APPLIER_RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("cannot connect to the database named by DATABASE_URL")]
    Connect,
    #[error(transparent)]
    Migration(#[from] MigrationError),
    #[error("cannot listen on 127.0.0.1:{port} ({kind:?})")]
    Bind { port: u16, kind: std::io::ErrorKind },
    #[error("the HTTP server stopped ({0:?})")]
    Serve(std::io::ErrorKind),
    #[error(transparent)]
    Engine(#[from] EngineError),
}

/// Opens the pool on `DATABASE_URL`. The driver's error is dropped unread:
/// it can carry the host, the user or the database name. The applier and the
/// version listener each keep a connection of their own.
pub async fn connect(config: &Config) -> Result<PgPool, StartupError> {
    let options = config.connect_options()?;
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
        .map_err(|_| StartupError::Connect)
}

/// Connects, migrates with `set`, then binds — in that order.
pub async fn prepare(
    config: &Config,
    set: &[Migration<'_>],
) -> Result<(PgPool, TcpListener), StartupError> {
    let pool = connect(config).await?;
    migrate::run(&pool, set).await?;
    let listener = http::bind_loopback(config.port)
        .await
        .map_err(|e| StartupError::Bind {
            port: config.port,
            kind: e.kind(),
        })?;
    Ok((pool, listener))
}

/// The shipped server registers no DataCapability: every command submitted
/// to it is an unknown capability until a later task adds one.
pub async fn run(config: Config) -> Result<(), StartupError> {
    let aggregates = Aggregates::embedded().map_err(EngineError::from)?;
    let (pool, listener) = prepare(&config, embedded::EMBEDDED).await?;
    let engine = Arc::new(Engine::with_aggregates(
        pool.clone(),
        aggregates,
        Registry::empty(),
        Arc::new(SystemClock),
    ));
    let (stop, stopped) = watch::channel(false);
    // An open `/data-version` stream would hold graceful shutdown forever:
    // the feed closes at shutdown, and the streams end with it.
    let mut feed_stopped = stopped.clone();
    let versions = VersionFeed::spawn(pool, async move {
        let _ = feed_stopped.wait_for(|s| *s).await;
    })
    .await?;
    tokio::spawn(run_applier(engine.clone(), stopped));
    info!("listening on 127.0.0.1:{}", config.port);
    let app = http::app(http::AppState { engine, versions });
    http::serve(listener, app, async move {
        shutdown_signal().await;
        let _ = stop.send(true);
    })
    .await
    .map_err(|e| StartupError::Serve(e.kind()))
}

/// Keeps one applier running until shutdown: takes the lock when it is free,
/// and takes it again if its connection is lost.
async fn run_applier(engine: Arc<Engine>, mut stopped: watch::Receiver<bool>) {
    loop {
        if *stopped.borrow() {
            return;
        }
        match engine.applier().await {
            Ok(Some(applier)) => {
                let mut stop = stopped.clone();
                applier
                    .run(async move {
                        let _ = stop.wait_for(|s| *s).await;
                    })
                    .await;
            }
            Ok(None) => info!("another applier holds the lock; waiting"),
            Err(e) => warn!(error = %e, "cannot start the applier"),
        }
        tokio::select! {
            _ = stopped.wait_for(|s| *s) => return,
            _ = tokio::time::sleep(APPLIER_RETRY) => {}
        }
    }
}

/// Ctrl-C, or SIGTERM where there is one (`kill`, launchd, systemd).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
