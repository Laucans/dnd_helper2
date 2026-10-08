use std::process::ExitCode;

use campagne_serveur::config::Config;
use campagne_serveur::startup;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            // sqlx logs server notices verbatim at `info`; server text can
            // name a role or a database, so the default keeps sqlx to errors.
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=error")),
        )
        .with_writer(std::io::stderr)
        .init();

    // `main` returns `ExitCode`, not `Result`: an error is printed with
    // `Display` only, which names a variable or a migration, never a value.
    let outcome = match Config::from_env() {
        Ok(config) => startup::run(config).await,
        Err(e) => Err(e.into()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
