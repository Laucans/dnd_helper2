//! Startup, strictly in order: configuration, connection, migrations, then
//! bind and listen. Nothing listens on a schema that is not at its latest
//! version.

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tracing::info;

use crate::config::{Config, ConfigError};
use crate::migrate::{self, Migration, MigrationError};
use crate::{embedded, http};

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
}

/// Opens the pool on `DATABASE_URL`. The driver's error is dropped unread:
/// it can carry the host, the user or the database name.
pub async fn connect(config: &Config) -> Result<PgPool, StartupError> {
    let options = config.connect_options()?;
    PgPoolOptions::new()
        .max_connections(5)
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

pub async fn run(config: Config) -> Result<(), StartupError> {
    let (_pool, listener) = prepare(&config, embedded::EMBEDDED).await?;
    info!("listening on 127.0.0.1:{}", config.port);
    http::serve(listener, shutdown_signal())
        .await
        .map_err(|e| StartupError::Serve(e.kind()))
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
