//! The server binary end to end: configuration refusals that leak nothing,
//! loopback health, a second start that changes nothing, a port in use, and
//! the DataCapabilities the binary accepts.

mod common;

use std::process::Stdio;
use std::str::FromStr;
use std::time::Duration;

use campagne_serveur::config::Config;
use campagne_serveur::migrate::{Migration, MigrationError};
use campagne_serveur::startup::{self, StartupError};
use common::TestDb;
use sqlx::postgres::PgConnectOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};

const BIN: &str = env!("CARGO_BIN_EXE_campagne-serveur");

/// A command with an empty environment plus `vars`: nothing leaks in from
/// the test process.
fn server(vars: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.env_clear()
        .envs(vars.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// Runs the binary to its end; it must stop by itself, quickly.
async fn refused(vars: &[(&str, &str)]) -> String {
    let out = tokio::time::timeout(Duration::from_secs(30), server(vars).output())
        .await
        .expect("the server stops by itself")
        .unwrap();
    assert!(!out.status.success(), "exit status {}", out.status);
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains("listening"), "{all}");
    all
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

async fn health(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await.ok()?;
    Some(response)
}

/// `POST /commands` over a bare socket; the whole response, head and body.
async fn post_command(port: u16, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "POST /commands HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

async fn wait_healthy(child: &mut Child, port: u16) {
    for _ in 0..200 {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the server exited early: {status}");
        }
        if let Some(response) = health(port).await {
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            assert!(response.ends_with("\r\n\r\nok"), "{response}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("/health never answered on 127.0.0.1:{port}");
}

async fn bookkeeping(db: &TestDb) -> Vec<(String, String, String, String)> {
    sqlx::query_as(
        "SELECT name, checksum, applied_at::text, xmin::text FROM schema_migrations ORDER BY name",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn missing_or_blank_database_url_refuses_to_start() {
    for vars in [vec![], vec![("DATABASE_URL", "   ")]] {
        let out = refused(&vars).await;
        assert!(out.contains("error: DATABASE_URL is not set"), "{out}");
    }
}

#[tokio::test]
async fn malformed_database_url_names_the_variable_only() {
    let out = refused(&[(
        "DATABASE_URL",
        "postgres://leakuser:leakpw-2@leakhost:notaport/leakdb",
    )])
    .await;
    assert!(out.contains("DATABASE_URL is malformed"), "{out}");
    assert!(!out.contains("leak"), "{out}");
}

#[tokio::test]
async fn unreachable_database_leaks_no_part_of_either_url() {
    let out = refused(&[
        (
            "DATABASE_URL",
            "postgres://leakuser:leakpw-3@127.0.0.1:1/leakdb",
        ),
        (
            "DATABASE_URL_READONLY",
            "postgres://rouser:ropw-4@127.0.0.1:1/rodb",
        ),
        ("RUST_LOG", "trace"),
    ])
    .await;
    assert!(
        out.contains("error: cannot connect to the database named by DATABASE_URL"),
        "{out}"
    );
    for marker in ["leakuser", "leakpw", "leakdb", "rouser", "ropw", "rodb"] {
        assert!(!out.contains(marker), "{marker} leaked in {out}");
    }
}

/// A reachable server that refuses the login quotes the user or the database
/// name in its error; an unreachable one quotes neither, so it cannot catch a
/// driver error passed through.
#[tokio::test]
async fn refused_login_leaks_no_part_of_the_url() {
    let admin = PgConnectOptions::from_str(&common::database_url()).unwrap();
    let authority = format!("{}:{}", admin.get_host(), admin.get_port());
    let unknown_database = format!("leakdb_{}", uuid::Uuid::new_v4().simple());
    for url in [
        format!("postgres://leakuser:leakpw-5@{authority}/leakdb"),
        common::url_with_database(&common::database_url(), &unknown_database),
    ] {
        let out = refused(&[("DATABASE_URL", &url), ("RUST_LOG", "trace")]).await;
        assert!(
            out.contains("error: cannot connect to the database named by DATABASE_URL"),
            "{out}"
        );
        for marker in ["leakuser", "leakpw", "leakdb"] {
            assert!(!out.contains(marker), "{marker} leaked in {out}");
        }
    }
}

#[tokio::test]
async fn serves_health_on_loopback_and_second_start_changes_nothing() {
    let db = TestDb::create().await;
    let port = free_port().await;
    let port_s = port.to_string();
    let vars = [
        ("DATABASE_URL", db.url.as_str()),
        ("SERVER_PORT", port_s.as_str()),
    ];

    let mut first = server(&vars).spawn().unwrap();
    wait_healthy(&mut first, port).await;
    first.kill().await.unwrap();
    let before = bookkeeping(&db).await;
    assert_eq!(before.len(), 7);

    let mut second = server(&vars).spawn().unwrap();
    wait_healthy(&mut second, port).await;
    // SIGTERM is a graceful stop: the server drains and exits 0.
    let pid = second.id().unwrap().to_string();
    let term = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .await
        .unwrap();
    assert!(term.success());
    let status = tokio::time::timeout(Duration::from_secs(10), second.wait())
        .await
        .expect("the server stops on SIGTERM")
        .unwrap();
    assert!(status.success(), "exit status {status}");
    assert_eq!(bookkeeping(&db).await, before);
    db.drop_db().await;
}

/// `run` hands the engine `startup::registry`, not an empty one: the binary
/// itself enqueues each of its five commands. Unknown ids are enough for the
/// mutations and for the add's campaign, since their activity is checked only
/// when the command applies; the create has no target at all.
#[tokio::test]
async fn the_shipped_binary_accepts_its_commands() {
    let db = TestDb::create().await;
    let port = free_port().await;
    let port_s = port.to_string();
    let mut child = server(&[
        ("DATABASE_URL", db.url.as_str()),
        ("SERVER_PORT", port_s.as_str()),
    ])
    .spawn()
    .unwrap();
    wait_healthy(&mut child, port).await;

    let id = uuid::Uuid::new_v4();
    for submission in [
        serde_json::json!({
            "dataCapability": "campagne.ajouterPj@1",
            "payload": { "campagneId": id, "nom": "Ysolde", "classe": "Barde", "niveau": 5 },
            "idempotencyKey": "binaire-ajouter-pj",
        }),
        serde_json::json!({
            "dataCapability": "campagne.creerCampagne@1",
            "payload": { "name": "Les Brumes" },
            "idempotencyKey": "binaire-creer-campagne",
        }),
        serde_json::json!({
            "dataCapability": "campagne.modifierPJ@1",
            "target": { "id": id },
            "payload": { "nom": "Ysolde", "classe": "Barde", "niveau": 5 },
            "basedOn": { "version": 0 },
            "idempotencyKey": "binaire-modifier",
        }),
        serde_json::json!({
            "dataCapability": "campagne.archiverPJ@1",
            "target": { "id": id },
            "idempotencyKey": "binaire-archiver-pj",
        }),
        serde_json::json!({
            "dataCapability": "campagne.archiverCampagne@1",
            "target": { "id": id },
            "idempotencyKey": "binaire-archiver-campagne",
        }),
    ] {
        let response = post_command(port, &submission.to_string()).await;
        assert!(
            response.starts_with("HTTP/1.1 202"),
            "{}: {response}",
            submission["dataCapability"]
        );
    }

    child.kill().await.unwrap();
    db.drop_db().await;
}

#[tokio::test]
async fn port_in_use_refuses_to_start() {
    let db = TestDb::create().await;
    let held = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = held.local_addr().unwrap().port().to_string();
    let out = refused(&[
        ("DATABASE_URL", db.url.as_str()),
        ("SERVER_PORT", port.as_str()),
    ])
    .await;
    assert!(
        out.contains(&format!("cannot listen on 127.0.0.1:{port}")),
        "{out}"
    );
    drop(held);
    db.drop_db().await;
}

#[tokio::test]
async fn failed_migration_means_nothing_listens() {
    let db = TestDb::create().await;
    let port = free_port().await;
    let port_s = port.to_string();
    let url = db.url.clone();
    let config = Config::from_lookup(|name| match name {
        "DATABASE_URL" => Some(url.clone()),
        "SERVER_PORT" => Some(port_s.clone()),
        _ => None,
    })
    .unwrap();
    let bad = [Migration {
        name: "0001_boom.sql",
        bytes: b"SELECT 1/0;",
    }];

    let err = startup::prepare(&config, &bad).await.unwrap_err();
    assert!(
        matches!(
            &err,
            StartupError::Migration(MigrationError::Failed { name, .. }) if name == "0001_boom.sql"
        ),
        "{err:?}"
    );
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    db.drop_db().await;
}
