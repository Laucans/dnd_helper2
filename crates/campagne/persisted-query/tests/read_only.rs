//! Rule 12: a write through the executor's connection fails at the database,
//! because the role lacks the privilege, not only because the session says so.
//! The test refuses to pass as a superuser.

mod common;

use campagne_persisted_query::connect_options;
use common::{TestDb, new_id};
use sqlx::{Connection, PgConnection};

async fn sqlstate(conn: &mut PgConnection, statement: &str) -> String {
    let error = sqlx::raw_sql(sqlx::AssertSqlSafe(statement.to_owned()))
        .execute(&mut *conn)
        .await
        .expect_err(&format!("`{statement}` must fail"));
    error
        .as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
        .unwrap_or_else(|| "n/a".into())
}

#[tokio::test]
async fn the_executors_role_is_distinct_and_unprivileged() {
    let db = TestDb::create().await;
    let migrating: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let mut conn = PgConnection::connect_with(&connect_options(db.vars()).unwrap())
        .await
        .unwrap();
    let (user, privileged): (String, bool) = sqlx::query_as(
        "SELECT current_user::text, rolsuper OR rolcreaterole OR rolcreatedb OR rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_ne!(
        user, migrating,
        "the executor must not connect as the migrating role"
    );
    assert!(
        !privileged,
        "the executor's role must hold no privileged attribute"
    );

    let (read_only, zone): (String, String) = sqlx::query_as(
        "SELECT current_setting('default_transaction_read_only'), current_setting('TimeZone')",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!((read_only.as_str(), zone.as_str()), ("on", "UTC"));

    // The role can read what it is meant to read.
    sqlx::query("SELECT count(*) FROM campagne_active")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("SELECT \"dataVersion\" FROM data_version")
        .execute(&mut conn)
        .await
        .unwrap();

    conn.close().await.unwrap();
    db.drop_db().await;
}

#[tokio::test]
async fn every_write_fails_at_the_database() {
    let db = TestDb::create().await;
    let id = new_id();
    let writes = [
        format!("INSERT INTO campagne (id, nom) VALUES ('{id}', 'X')"),
        format!("UPDATE campagne SET nom = 'Y' WHERE id = '{id}'"),
        "DELETE FROM campagne".to_owned(),
        format!(
            "INSERT INTO pj (\"campagneId\", nom, classe, niveau) VALUES ('{id}', 'X', 'Y', 1)"
        ),
        "UPDATE pj SET niveau = 2".to_owned(),
        "DELETE FROM pj".to_owned(),
        "INSERT INTO campagne_active (id, nom) VALUES (gen_random_uuid(), 'X')".to_owned(),
        "UPDATE campagne_active SET nom = 'Y'".to_owned(),
        "DELETE FROM campagne_active".to_owned(),
        "UPDATE data_version SET \"dataVersion\" = 99".to_owned(),
        "DELETE FROM data_version".to_owned(),
        "UPDATE dataguard_version SET version = 99".to_owned(),
        "TRUNCATE campagne".to_owned(),
        "CREATE TABLE pirate (a int)".to_owned(),
    ];

    let mut conn = PgConnection::connect_with(&connect_options(db.vars()).unwrap())
        .await
        .unwrap();

    // 1. The session is read-only: the statement is refused as a write.
    for statement in &writes {
        let code = sqlstate(&mut conn, statement).await;
        assert!(
            ["25006", "42501"].contains(&code.as_str()),
            "`{statement}` failed with {code}"
        );
    }

    // 2. Even a transaction that asks for READ WRITE: the role lacks the
    //    privilege (42501), so the read-only flag is not what protects.
    for statement in &writes {
        sqlx::raw_sql("BEGIN READ WRITE")
            .execute(&mut conn)
            .await
            .unwrap();
        let code = sqlstate(&mut conn, statement).await;
        assert_eq!(code, "42501", "`{statement}` must be a privilege error");
        sqlx::raw_sql("ROLLBACK").execute(&mut conn).await.unwrap();
    }

    // The tables are untouched.
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM campagne")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    conn.close().await.unwrap();
    db.drop_db().await;
}

#[tokio::test]
async fn the_role_cannot_read_a_table() {
    let db = TestDb::create().await;
    let mut conn = PgConnection::connect_with(&connect_options(db.vars()).unwrap())
        .await
        .unwrap();
    for table in ["campagne", "pj", "dataguard_version", "dataguard_queue"] {
        let code = sqlstate(&mut conn, &format!("SELECT * FROM {table}")).await;
        assert_eq!(code, "42501", "{table}");
    }
    conn.close().await.unwrap();
    db.drop_db().await;
}
