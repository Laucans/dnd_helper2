//! Rules 9-11: the executor starts on `DATABASE_URL_READONLY` alone, never
//! falls back, and prints no part of a connection string.
//!
//! This is the one file allowed to name the migrating variable: the refusal it
//! proves has to set it.

mod common;

use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use campagne_persisted_query::{DATABASE_URL_READONLY, Executor, StartError, registry_dir};
use common::readonly_url;
use tokio::time::timeout;

const MIGRATING: &str = "DATABASE_URL";
const A_VALID_URL: &str = "postgres://someone:somepw@127.0.0.1:1/x";

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

async fn start(pairs: &[(&str, &str)]) -> Result<Executor, StartError> {
    // No network may be needed to refuse: a hang here is a failure.
    timeout(
        Duration::from_secs(1),
        Executor::from_variables(&registry_dir(), lookup(pairs)),
    )
    .await
    .expect("the refusal must not wait on the network")
}

#[tokio::test]
async fn refuses_to_start_without_the_read_only_variable_even_with_the_migrating_one() {
    let err = start(&[(MIGRATING, A_VALID_URL)]).await.unwrap_err();
    assert!(
        matches!(err, StartError::Missing(DATABASE_URL_READONLY)),
        "{err:?}"
    );
    assert!(err.to_string().contains(DATABASE_URL_READONLY));
    assert!(!err.to_string().contains(A_VALID_URL));
}

#[tokio::test]
async fn an_empty_or_blank_variable_is_missing() {
    for value in ["", "   ", "\t\n"] {
        let err = start(&[(MIGRATING, A_VALID_URL), (DATABASE_URL_READONLY, value)])
            .await
            .unwrap_err();
        assert!(
            matches!(err, StartError::Missing(DATABASE_URL_READONLY)),
            "{value:?}: {err:?}"
        );
    }
}

#[tokio::test]
async fn a_malformed_value_is_refused_without_echoing_it() {
    let secret = "not a url but sentinelpw42";
    let err = start(&[(DATABASE_URL_READONLY, secret)]).await.unwrap_err();
    assert!(
        matches!(err, StartError::Malformed(DATABASE_URL_READONLY)),
        "{err:?}"
    );
    assert!(!format!("{err} {err:?}").contains("sentinelpw42"));
}

#[tokio::test]
async fn a_failed_connection_quotes_neither_the_password_nor_the_host() {
    let url = "postgres://app_lecture:sentinelpw42@sentinel-host.invalid:5432/x";
    let err = Executor::from_variables(&registry_dir(), lookup(&[(DATABASE_URL_READONLY, url)]))
        .await
        .unwrap_err();
    assert!(matches!(err, StartError::Connect { .. }), "{err:?}");

    let mut seen = format!("{err} | {err:?}");
    let mut source = err.source();
    while let Some(cause) = source {
        seen.push_str(&format!(" | {cause} | {cause:?}"));
        source = cause.source();
    }
    for secret in ["sentinelpw42", "sentinel-host", "app_lecture"] {
        assert!(!seen.contains(secret), "the error quotes {secret}: {seen}");
    }
}

#[tokio::test]
async fn a_built_executor_debug_shows_neither_password_nor_host() {
    let url = readonly_url();
    let executor =
        Executor::from_variables(&registry_dir(), lookup(&[(DATABASE_URL_READONLY, &url)]))
            .await
            .expect("the executor starts on DATABASE_URL_READONLY");
    let debug = format!("{executor:?}");

    assert!(debug.contains("ListePjs"), "{debug}");
    let userinfo = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('@'))
        .map(|(userinfo, _)| userinfo)
        .unwrap_or_default();
    let password = userinfo.split_once(':').map(|(_, p)| p).unwrap_or_default();
    let host = url
        .split_once('@')
        .and_then(|(_, rest)| rest.split(['/', ':']).next())
        .unwrap_or_default();
    assert!(
        !password.is_empty() && !host.is_empty(),
        "the test needs a URL with a password and a host"
    );
    assert!(!debug.contains(password), "{debug}");
    assert!(!debug.contains(host), "{debug}");
}

#[tokio::test]
async fn a_broken_registry_stops_the_start() {
    let dir = std::env::temp_dir().join(format!("pq-registry-{}", common::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("registry.json"), r#"{"queries": "no"}"#).unwrap();
    let url = readonly_url();
    let err = Executor::from_variables(&dir, lookup(&[(DATABASE_URL_READONLY, &url)]))
        .await
        .unwrap_err();
    assert!(matches!(err, StartError::Registry(_)), "{err:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}
