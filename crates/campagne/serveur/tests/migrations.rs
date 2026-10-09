//! The migration runner against a real database.

mod common;

use campagne_serveur::embedded::EMBEDDED;
use campagne_serveur::migrate::{self, Migration, MigrationError};
use std::time::Duration;

use common::{TestDb, table_exists};
use sha2::{Digest, Sha256};

const ALL: [&str; 6] = [
    "0001_campagne.sql",
    "0002_pj.sql",
    "0003_vues_lecture.sql",
    "0004_role_lecture.sql",
    "0005_dataguard_version.sql",
    "0006_dataguard_queue.sql",
];

fn mig(name: &'static str, sql: &'static str) -> Migration<'static> {
    Migration {
        name,
        bytes: sql.as_bytes(),
    }
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
async fn first_run_applies_all_in_order() {
    let db = TestDb::create().await;
    let report = migrate::run(&db.pool, EMBEDDED).await.unwrap();
    assert_eq!(report.applied, ALL);

    let rows = bookkeeping(&db).await;
    assert_eq!(rows.len(), ALL.len());
    for ((name, checksum, _, _), migration) in rows.iter().zip(EMBEDDED) {
        assert_eq!(name, migration.name);
        assert_eq!(checksum.len(), 64);
        assert_eq!(*checksum, hex::encode(Sha256::digest(migration.bytes)));
    }
    db.drop_db().await;
}

#[tokio::test]
async fn second_run_applies_nothing_and_writes_nothing() {
    let db = TestDb::create().await;
    migrate::run(&db.pool, EMBEDDED).await.unwrap();
    let before = bookkeeping(&db).await;

    let report = migrate::run(&db.pool, EMBEDDED).await.unwrap();
    assert!(report.applied.is_empty());
    assert_eq!(bookkeeping(&db).await, before);
    db.drop_db().await;
}

#[tokio::test]
async fn changed_applied_file_stops_before_pending() {
    let db = TestDb::create().await;
    let v1 = [
        mig("0001_t1.sql", "CREATE TABLE t1 (x int);\n"),
        mig("0002_t2.sql", "CREATE TABLE t2 (x int);\n"),
    ];
    migrate::run(&db.pool, &v1).await.unwrap();

    // Same statement, CRLF line ending: a change all the same.
    let v2 = [
        mig("0001_t1.sql", "CREATE TABLE t1 (x int);\r\n"),
        mig("0002_t2.sql", "CREATE TABLE t2 (x int);\n"),
        mig("0003_t3.sql", "CREATE TABLE t3 (x int);\n"),
    ];
    let err = migrate::run(&db.pool, &v2).await.unwrap_err();
    assert_eq!(err, MigrationError::ChecksumMismatch("0001_t1.sql".into()));
    assert!(err.to_string().contains("0001_t1.sql"));
    assert!(!table_exists(&db.pool, "t3").await);
    assert_eq!(bookkeeping(&db).await.len(), 2);
    db.drop_db().await;
}

#[tokio::test]
async fn applied_file_missing_from_build_stops() {
    let db = TestDb::create().await;
    let v1 = [
        mig("0001_t1.sql", "CREATE TABLE t1 (x int);"),
        mig("0002_t2.sql", "CREATE TABLE t2 (x int);"),
    ];
    migrate::run(&db.pool, &v1).await.unwrap();

    let v2 = [
        mig("0001_t1.sql", "CREATE TABLE t1 (x int);"),
        mig("0003_t3.sql", "CREATE TABLE t3 (x int);"),
    ];
    let err = migrate::run(&db.pool, &v2).await.unwrap_err();
    assert_eq!(err, MigrationError::MissingFromBuild("0002_t2.sql".into()));
    assert!(!table_exists(&db.pool, "t3").await);
    db.drop_db().await;
}

#[tokio::test]
async fn pending_older_than_applied_stops() {
    let db = TestDb::create().await;
    migrate::run(&db.pool, &[mig("0002_t2.sql", "CREATE TABLE t2 (x int);")])
        .await
        .unwrap();
    let err = migrate::run(
        &db.pool,
        &[
            mig("0001_t1.sql", "CREATE TABLE t1 (x int);"),
            mig("0002_t2.sql", "CREATE TABLE t2 (x int);"),
        ],
    )
    .await
    .unwrap_err();
    assert_eq!(err, MigrationError::OutOfOrder("0001_t1.sql".into()));
    assert!(!table_exists(&db.pool, "t1").await);
    db.drop_db().await;
}

#[tokio::test]
async fn failing_migration_leaves_no_trace() {
    let db = TestDb::create().await;
    let broken = [
        mig("0001_t1.sql", "CREATE TABLE t1 (x int);"),
        mig("0002_t2.sql", "CREATE TABLE t2 (x int); SELECT 1/0;"),
        mig("0003_t3.sql", "CREATE TABLE t3 (x int);"),
    ];
    let err = migrate::run(&db.pool, &broken).await.unwrap_err();
    assert_eq!(
        err,
        MigrationError::Failed {
            name: "0002_t2.sql".into(),
            sqlstate: "22012".into()
        }
    );
    assert_eq!(
        err.to_string(),
        "migration 0002_t2.sql failed (SQLSTATE 22012)"
    );
    assert!(table_exists(&db.pool, "t1").await);
    assert!(!table_exists(&db.pool, "t2").await);
    assert!(!table_exists(&db.pool, "t3").await);
    let names: Vec<String> = bookkeeping(&db).await.into_iter().map(|r| r.0).collect();
    assert_eq!(names, ["0001_t1.sql"]);

    let fixed = [
        mig("0001_t1.sql", "CREATE TABLE t1 (x int);"),
        mig("0002_t2.sql", "CREATE TABLE t2 (x int);"),
        mig("0003_t3.sql", "CREATE TABLE t3 (x int);"),
    ];
    let report = migrate::run(&db.pool, &fixed).await.unwrap();
    assert_eq!(report.applied, ["0002_t2.sql", "0003_t3.sql"]);
    assert!(table_exists(&db.pool, "t3").await);
    db.drop_db().await;
}

#[tokio::test]
async fn bad_name_and_duplicate_prefix_apply_nothing() {
    let db = TestDb::create().await;
    let cases: [(Vec<Migration<'static>>, MigrationError); 3] = [
        (
            vec![mig("1_x.sql", "CREATE TABLE t1 (x int);")],
            MigrationError::InvalidName("1_x.sql".into()),
        ),
        (
            vec![mig("0001_A.sql", "CREATE TABLE t1 (x int);")],
            MigrationError::InvalidName("0001_A.sql".into()),
        ),
        (
            vec![
                mig("0001_a.sql", "CREATE TABLE t1 (x int);"),
                mig("0001_b.sql", "CREATE TABLE t2 (x int);"),
            ],
            MigrationError::DuplicatePrefix("0001_b.sql".into()),
        ),
    ];
    for (set, expected) in cases {
        assert_eq!(migrate::run(&db.pool, &set).await.unwrap_err(), expected);
    }
    assert!(!table_exists(&db.pool, "schema_migrations").await);
    assert!(!table_exists(&db.pool, "t1").await);
    db.drop_db().await;
}

#[tokio::test]
async fn concurrent_runners_apply_each_once() {
    let db = TestDb::create().await;
    let (a, b) = tokio::join!(
        migrate::run(&db.pool, EMBEDDED),
        migrate::run(&db.pool, EMBEDDED)
    );
    let mut applied = a.unwrap().applied;
    applied.extend(b.unwrap().applied);
    applied.sort();
    assert_eq!(applied, ALL);
    assert_eq!(bookkeeping(&db).await.len(), ALL.len());
    db.drop_db().await;
}

#[tokio::test]
async fn a_held_lock_ends_in_an_error_not_a_hang() {
    let db = TestDb::create().await;
    let mut holder = db.pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(0x6361_6d70_6d69_6772_i64)
        .execute(&mut *holder)
        .await
        .unwrap();

    let wait = Duration::from_millis(300);
    let err = migrate::run_with_lock_wait(&db.pool, EMBEDDED, wait)
        .await
        .unwrap_err();
    assert_eq!(err, MigrationError::LockBusy(wait));
    assert!(!table_exists(&db.pool, "schema_migrations").await);

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(0x6361_6d70_6d69_6772_i64)
        .execute(&mut *holder)
        .await
        .unwrap();
    drop(holder);
    assert_eq!(migrate::run(&db.pool, EMBEDDED).await.unwrap().applied, ALL);
    db.drop_db().await;
}
