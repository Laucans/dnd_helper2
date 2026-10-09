//! The Capability starts on `DATABASE_URL_READONLY` alone, and a write through
//! its connection fails at the database.

mod common;

use campagne_persisted_query::{DATABASE_URL_READONLY, StartError, connect_options, registry_dir};
use common::TestDb;
use lister_campagnes::demarrer_avec;
use sqlx::postgres::PgPoolOptions;

const SECRET_URL: &str = "postgres://gm:s3cr3t-pw@db.internal:5432/app";

fn code(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|e| e.code().map(|c| c.into_owned()))
        .unwrap_or_else(|| format!("not a database error: {error}"))
}

#[tokio::test]
async fn it_does_not_start_without_the_read_only_variable() {
    for lookup in [None, Some(""), Some("   ")] {
        let result = demarrer_avec(&registry_dir(), |name| {
            (name == DATABASE_URL_READONLY)
                .then(|| lookup.map(str::to_owned))
                .flatten()
        })
        .await;
        assert!(
            matches!(result, Err(StartError::Missing(DATABASE_URL_READONLY))),
            "{lookup:?} gave {result:?}"
        );
    }
}

#[tokio::test]
async fn it_never_falls_back_to_the_migrating_variable() {
    let result = demarrer_avec(&registry_dir(), |name| {
        (name == "DATABASE_URL").then(|| SECRET_URL.to_owned())
    })
    .await;

    let Err(error) = result else {
        panic!("the Capability started on DATABASE_URL");
    };
    assert!(matches!(error, StartError::Missing(DATABASE_URL_READONLY)));
    let text = error.to_string();
    assert!(
        !text.contains("s3cr3t-pw") && !text.contains("db.internal"),
        "{text}"
    );
}

#[tokio::test]
async fn a_malformed_url_is_refused_without_quoting_it() {
    let result = demarrer_avec(&registry_dir(), |name| {
        (name == DATABASE_URL_READONLY).then(|| "not a url s3cr3t-pw".to_owned())
    })
    .await;

    let Err(error) = result else {
        panic!("a malformed url started the Capability");
    };
    assert!(!error.to_string().contains("s3cr3t-pw"), "{error}");
}

#[tokio::test]
async fn a_write_through_its_connection_fails_at_the_database() {
    let db = TestDb::create().await;
    let options = connect_options(db.vars()).unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();

    // The session is read-only by default: refused before any privilege check.
    let default_session = sqlx::query(
        "INSERT INTO campagne (id, nom) VALUES ('00000000-0000-0000-0000-0000000000aa', 'Intruse')",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(code(&default_session), "25006");

    // Even a read-write transaction is refused: the role has no privilege.
    let mut connection = pool.acquire().await.unwrap();
    sqlx::query("BEGIN READ WRITE")
        .execute(&mut *connection)
        .await
        .unwrap();
    let privilege = sqlx::query(
        "INSERT INTO campagne (id, nom) VALUES ('00000000-0000-0000-0000-0000000000ab', 'Intruse')",
    )
    .execute(&mut *connection)
    .await
    .unwrap_err();
    assert_eq!(code(&privilege), "42501");
    drop(connection);
    pool.close().await;

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM campagne")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    db.drop_db().await;
}
