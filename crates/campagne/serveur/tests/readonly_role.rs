//! The read-only role, reached through `DATABASE_URL_READONLY`: it reads the
//! views and nothing else, and every write through it fails at the database.

mod common;

use campagne_serveur::embedded::EMBEDDED;
use campagne_serveur::migrate;
use common::{TestDb, code, readonly_options};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, Executor};

async fn counts(pool: &PgPool) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM campagne), (SELECT count(*) FROM pj),
                (SELECT count(*) FROM schema_migrations)",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn readonly_role_reads_views_and_writes_nothing() {
    let db = TestDb::create().await;
    migrate::run(&db.pool, EMBEDDED).await.unwrap();
    let campagne: sqlx::types::Uuid =
        sqlx::query_scalar("INSERT INTO campagne (nom) VALUES ('Les Brumes') RETURNING id")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO pj (\"campagneId\", nom, classe, niveau) VALUES ($1, 'Ysolde', 'Barde', 3)",
    )
    .bind(campagne)
    .execute(&db.pool)
    .await
    .unwrap();
    let before = counts(&db.pool).await;

    let ro = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(readonly_options(&db.name))
        .await
        .expect("connect through DATABASE_URL_READONLY");

    // Never the superuser, never the role that ran the migrations.
    let (me, superuser, createrole, createdb): (String, bool, bool, bool) = sqlx::query_as(
        "SELECT rolname::text, rolsuper, rolcreaterole, rolcreatedb FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&ro)
    .await
    .unwrap();
    let admin: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_ne!(
        me, admin,
        "DATABASE_URL_READONLY must not be the migrating role"
    );
    assert!(
        !superuser && !createrole && !createdb,
        "{me} has a write-capable attribute"
    );

    for view in ["campagne_active", "pj_actif"] {
        let n: i64 = sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {view}")))
            .fetch_one(&ro)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "read {view}: SQLSTATE {:?}",
                    e.as_database_error().and_then(|d| d.code())
                )
            });
        assert!(n >= 1, "{view} is empty");
    }

    for table in ["campagne", "pj", "schema_migrations"] {
        let sql = format!("SELECT * FROM {table}");
        assert_eq!(
            code(ro.execute(AssertSqlSafe(sql)).await),
            "42501",
            "{table}"
        );
    }

    let writes = |rel: &str| {
        [
            match rel {
                "campagne" | "campagne_active" => format!("INSERT INTO {rel} (nom) VALUES ('x')"),
                "pj" | "pj_actif" => format!(
                    "INSERT INTO {rel} (\"campagneId\", nom, classe, niveau) VALUES ('{campagne}', 'x', 'y', 1)"
                ),
                _ => format!("INSERT INTO {rel} (name, checksum) VALUES ('x', 'y')"),
            },
            match rel {
                "schema_migrations" => format!("UPDATE {rel} SET checksum = 'x'"),
                _ => format!("UPDATE {rel} SET nom = 'x'"),
            },
            format!("DELETE FROM {rel}"),
            format!("TRUNCATE {rel}"),
        ]
    };
    for table in ["campagne", "pj", "schema_migrations"] {
        for sql in writes(table) {
            assert_eq!(
                code(ro.execute(AssertSqlSafe(sql.clone())).await),
                "42501",
                "{sql}"
            );
        }
    }
    for view in ["campagne_active", "pj_actif"] {
        for sql in writes(view) {
            let refused = code(ro.execute(AssertSqlSafe(sql.clone())).await);
            // `campagne_active` is auto-updatable: only the missing privilege
            // stops it. `pj_actif` is a join view, which the rewriter may
            // refuse first (55000). TRUNCATE never applies to a view (42809).
            let accepted: &[&str] = match (view, sql.starts_with("TRUNCATE")) {
                (_, true) => &["42501", "42809"],
                ("campagne_active", false) => &["42501"],
                _ => &["42501", "55000"],
            };
            assert!(
                accepted.contains(&refused.as_str()),
                "{sql}: SQLSTATE {refused}"
            );
        }
    }

    for sql in ["CREATE TABLE x (i int)", "CREATE TEMP TABLE y (i int)"] {
        assert_eq!(code(ro.execute(sql).await), "42501", "{sql}");
    }

    ro.close().await;
    assert_eq!(counts(&db.pool).await, before);
    db.drop_db().await;
}

/// A caller's predicate never runs on a hidden row: the views are security
/// barriers, so an error raised by a predicate cannot reveal a tombstoned
/// campaign or PC.
#[tokio::test]
async fn views_are_security_barriers() {
    let db = TestDb::create().await;
    migrate::run(&db.pool, EMBEDDED).await.unwrap();
    let archived: sqlx::types::Uuid = sqlx::query_scalar(
        "INSERT INTO campagne (nom, \"archiveLe\") VALUES ('Cachee', now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO pj (\"campagneId\", nom, classe, niveau) VALUES ($1, 'Cache', 'Barde', 3)",
    )
    .bind(archived)
    .execute(&db.pool)
    .await
    .unwrap();

    let barriers: Vec<String> = sqlx::query_scalar(
        "SELECT relname::text FROM pg_class
         WHERE relname IN ('campagne_active', 'pj_actif')
           AND 'security_barrier=true' = ANY (reloptions)
         ORDER BY relname",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(barriers, ["campagne_active", "pj_actif"]);

    let ro = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(readonly_options(&db.name))
        .await
        .expect("connect through DATABASE_URL_READONLY");
    for sql in [
        "SELECT count(*) FROM campagne_active WHERE 1 / (CASE WHEN nom = 'Cachee' THEN 0 ELSE 1 END) = 1",
        "SELECT count(*) FROM pj_actif WHERE 1 / (CASE WHEN nom = 'Cache' THEN 0 ELSE 1 END) = 1",
    ] {
        let n: i64 = sqlx::query_scalar(sql)
            .fetch_one(&ro)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "{sql}: SQLSTATE {:?}",
                    e.as_database_error().and_then(|d| d.code())
                )
            });
        assert_eq!(n, 0, "{sql}");
    }
    ro.close().await;
    db.drop_db().await;
}
