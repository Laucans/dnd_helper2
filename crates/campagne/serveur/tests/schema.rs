//! The schema the embedded migrations produce. Seeded rows are invented.

mod common;

use std::collections::BTreeSet;

use campagne_serveur::embedded::EMBEDDED;
use campagne_serveur::migrate;
use common::{TestDb, code};
use sqlx::PgPool;
use sqlx::types::Uuid;

async fn migrated() -> TestDb {
    let db = TestDb::create().await;
    migrate::run(&db.pool, EMBEDDED).await.unwrap();
    db
}

async fn columns(pool: &PgPool, table: &str) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = $1
         ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn campagne(pool: &PgPool, nom: &str, archived: bool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO campagne (nom, \"archiveLe\") VALUES ($1, CASE WHEN $2 THEN now() END)
         RETURNING id",
    )
    .bind(nom)
    .bind(archived)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn pj(pool: &PgPool, campagne: Uuid, nom: &str, archived: bool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO pj (\"campagneId\", nom, classe, niveau, \"archiveLe\")
         VALUES ($1, $2, 'Barde', 3, CASE WHEN $3 THEN now() END)
         RETURNING id",
    )
    .bind(campagne)
    .bind(nom)
    .bind(archived)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn ids(pool: &PgPool, sql: &'static str, scope: Uuid) -> BTreeSet<Uuid> {
    sqlx::query_scalar(sql)
        .bind(scope)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

#[tokio::test]
async fn table_columns_are_exact() {
    let db = migrated().await;
    let expected = |cols: &[(&str, &str, &str)]| -> Vec<(String, String, String)> {
        cols.iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
            .collect()
    };
    assert_eq!(
        columns(&db.pool, "campagne").await,
        expected(&[
            ("id", "uuid", "NO"),
            ("nom", "text", "NO"),
            ("creeLe", "timestamp with time zone", "NO"),
            ("archiveLe", "timestamp with time zone", "YES"),
        ])
    );
    assert_eq!(
        columns(&db.pool, "pj").await,
        expected(&[
            ("id", "uuid", "NO"),
            ("campagneId", "uuid", "NO"),
            ("nom", "text", "NO"),
            ("classe", "text", "NO"),
            ("niveau", "integer", "NO"),
            ("creeLe", "timestamp with time zone", "NO"),
            ("archiveLe", "timestamp with time zone", "YES"),
        ])
    );
    db.drop_db().await;
}

#[tokio::test]
async fn view_columns_are_exact_and_nothing_holds_the_party_level() {
    let db = migrated().await;
    let names = |cols: Vec<(String, String, String)>| -> Vec<String> {
        cols.into_iter().map(|c| c.0).collect()
    };
    assert_eq!(
        names(columns(&db.pool, "campagne_active").await),
        ["id", "nom", "creeLe"]
    );
    assert_eq!(
        names(columns(&db.pool, "pj_actif").await),
        ["id", "campagneId", "nom", "classe", "niveau", "creeLe"]
    );

    let suspicious: Vec<String> = sqlx::query_scalar(
        "SELECT table_name || '.' || column_name FROM information_schema.columns
         WHERE table_schema = 'public' AND column_name ~* '(groupe|party|moyen|mean|avg)'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(suspicious.is_empty(), "{suspicious:?}");

    let relations: BTreeSet<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    assert_eq!(
        relations,
        [
            "campagne",
            "campagne_active",
            "data_version",
            "dataguard_hold",
            "dataguard_queue",
            "dataguard_version",
            "pj",
            "pj_actif",
            "schema_migrations"
        ]
        .map(String::from)
        .into()
    );
    db.drop_db().await;
}

#[tokio::test]
async fn required_fields_have_no_default() {
    let db = migrated().await;
    let c = campagne(&db.pool, "Les Brumes", false).await;
    assert_eq!(
        code(
            sqlx::query("INSERT INTO campagne DEFAULT VALUES")
                .execute(&db.pool)
                .await
        ),
        "23502"
    );
    for sql in [
        "INSERT INTO pj (\"campagneId\", classe, niveau) VALUES ($1, 'Barde', 3)",
        "INSERT INTO pj (\"campagneId\", nom, niveau) VALUES ($1, 'Ysolde', 3)",
        "INSERT INTO pj (\"campagneId\", nom, classe) VALUES ($1, 'Ysolde', 'Barde')",
    ] {
        assert_eq!(
            code(sqlx::query(sql).bind(c).execute(&db.pool).await),
            "23502",
            "{sql}"
        );
    }
    db.drop_db().await;
}

#[tokio::test]
async fn fk_is_restrict_both_ways() {
    let db = migrated().await;
    let (upd, del): (String, String) = sqlx::query_as(
        "SELECT confupdtype::text, confdeltype::text FROM pg_constraint WHERE conname = 'pj_campagne_fk'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((upd.as_str(), del.as_str()), ("r", "r"));

    let c = campagne(&db.pool, "Les Brumes", false).await;
    pj(&db.pool, c, "Ysolde", false).await;
    assert_eq!(
        code(
            sqlx::query("UPDATE campagne SET id = gen_random_uuid() WHERE id = $1")
                .bind(c)
                .execute(&db.pool)
                .await
        ),
        "23503"
    );
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO pj (\"campagneId\", nom, classe, niveau) VALUES ($1, 'X', 'Y', 1)"
            )
            .bind(Uuid::new_v4())
            .execute(&db.pool)
            .await
        ),
        "23503"
    );
    assert_eq!(
        code(
            sqlx::query("DELETE FROM campagne WHERE id = $1")
                .bind(c)
                .execute(&db.pool)
                .await
        ),
        "23503"
    );
    db.drop_db().await;
}

#[tokio::test]
async fn text_must_be_nfc() {
    let db = migrated().await;
    let nfd = "Cl\u{65}\u{301}ment"; // "Clément", decomposed
    let nfc = "Cl\u{e9}ment";
    assert_eq!(
        code(
            sqlx::query("INSERT INTO campagne (nom) VALUES ($1)")
                .bind(nfd)
                .execute(&db.pool)
                .await
        ),
        "23514"
    );
    let c = campagne(&db.pool, nfc, false).await;

    let insert = "INSERT INTO pj (\"campagneId\", nom, classe, niveau) VALUES ($1, $2, $3, 1)";
    for (nom, classe) in [(nfd, nfc), (nfc, nfd)] {
        assert_eq!(
            code(
                sqlx::query(insert)
                    .bind(c)
                    .bind(nom)
                    .bind(classe)
                    .execute(&db.pool)
                    .await
            ),
            "23514"
        );
    }
    sqlx::query(insert)
        .bind(c)
        .bind(nfc)
        .bind(nfc)
        .execute(&db.pool)
        .await
        .unwrap();
    db.drop_db().await;
}

#[tokio::test]
async fn no_domain_range_constraints() {
    let db = migrated().await;
    let long = "a".repeat(300);
    let c = campagne(&db.pool, &long, false).await;
    for niveau in [0, 99, -3] {
        sqlx::query(
            "INSERT INTO pj (\"campagneId\", nom, classe, niveau) VALUES ($1, $2, 'Barde', $3)",
        )
        .bind(c)
        .bind(format!("pj{niveau}"))
        .bind(niveau)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    // Same name twice in one campaign: uniqueness is a DataGuard invariant.
    pj(&db.pool, c, "Ysolde", false).await;
    pj(&db.pool, c, "Ysolde", false).await;
    db.drop_db().await;
}

#[tokio::test]
async fn views_hide_tombstones_and_scope_by_campaign() {
    let db = migrated().await;
    let a = campagne(&db.pool, "Campagne A", false).await;
    let b = campagne(&db.pool, "Campagne B", false).await;
    let c = campagne(&db.pool, "Campagne C", true).await;
    let a1 = pj(&db.pool, a, "a1", false).await;
    pj(&db.pool, a, "a2", true).await;
    let b1 = pj(&db.pool, b, "b1", false).await;
    pj(&db.pool, c, "c1", false).await;
    let unknown = Uuid::new_v4();

    let all: BTreeSet<Uuid> = sqlx::query_scalar("SELECT id FROM campagne_active")
        .fetch_all(&db.pool)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(all, [a, b].into());

    let pcs = "SELECT id FROM pj_actif WHERE \"campagneId\" = $1";
    assert_eq!(ids(&db.pool, pcs, a).await, [a1].into());
    assert_eq!(ids(&db.pool, pcs, b).await, [b1].into());
    assert!(ids(&db.pool, pcs, c).await.is_empty());
    assert!(ids(&db.pool, pcs, unknown).await.is_empty());

    let one = "SELECT id FROM campagne_active WHERE id = $1";
    assert_eq!(ids(&db.pool, one, a).await, [a].into());
    assert!(ids(&db.pool, one, c).await.is_empty());
    assert!(ids(&db.pool, one, unknown).await.is_empty());
    db.drop_db().await;
}
