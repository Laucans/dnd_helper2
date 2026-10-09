//! The read role (`DATABASE_URL_READONLY`) reads the current `dataVersion`
//! and nothing else of the engine: not the queue, not the keys, not the
//! payloads, and it writes nowhere.

mod common;

use campagne_serveur::embedded::EMBEDDED;
use campagne_serveur::migrate;
use common::{TestDb, code, readonly_options};
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, Executor};

#[tokio::test]
async fn the_read_role_reads_the_version_and_nothing_else_of_the_engine() {
    let db = TestDb::create().await;
    migrate::run(&db.pool, EMBEDDED).await.unwrap();
    let ro = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(readonly_options(&db.name))
        .await
        .expect("connect through DATABASE_URL_READONLY");

    let version: i64 = sqlx::query_scalar("SELECT \"dataVersion\" FROM data_version")
        .fetch_one(&ro)
        .await
        .unwrap();
    assert_eq!(version, 0);

    for table in ["dataguard_queue", "dataguard_hold", "dataguard_version"] {
        let sql = format!("SELECT * FROM {table}");
        assert_eq!(
            code(ro.execute(AssertSqlSafe(sql)).await),
            "42501",
            "{table}"
        );
    }
    for sql in [
        "INSERT INTO dataguard_queue (command) VALUES (gen_random_uuid())",
        "INSERT INTO dataguard_hold (id) VALUES (gen_random_uuid())",
        "INSERT INTO dataguard_version DEFAULT VALUES",
        "UPDATE dataguard_queue SET state = 'applied'",
        "UPDATE dataguard_hold SET author = 'x'",
        "UPDATE dataguard_version SET version = 9",
        "UPDATE data_version SET \"dataVersion\" = 9",
        "DELETE FROM dataguard_queue",
        "DELETE FROM dataguard_hold",
        "DELETE FROM dataguard_version",
        "DELETE FROM data_version",
        "SELECT nextval('dataguard_enqueue_seq')",
    ] {
        assert_eq!(code(ro.execute(sql).await), "42501", "{sql}");
    }
    ro.close().await;

    let version: i64 = sqlx::query_scalar("SELECT version FROM dataguard_version")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(version, 0);
    db.drop_db().await;
}
