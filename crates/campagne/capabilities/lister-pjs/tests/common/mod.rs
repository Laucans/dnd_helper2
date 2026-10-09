//! A fresh, migrated database per test, created and dropped through
//! `DATABASE_URL`, and a Capability wired to it through the read-only login.
//! A missing variable fails the test: a database test never skips.
//!
//! The migrations are the serveur's files, applied raw: depending on the
//! serveur's runner would pull the write side into this crate.
#![allow(dead_code)] // each test binary uses its own part of this module

use std::path::{Path, PathBuf};

use campagne_lister_pjs::{Executor, ListerPjs, registry_dir};
use campagne_persisted_query::DATABASE_URL_READONLY;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

pub fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for database tests")
}

pub fn readonly_url() -> String {
    std::env::var(DATABASE_URL_READONLY)
        .expect("DATABASE_URL_READONLY must be set for database tests")
}

/// `url` with its database name replaced by `database`.
pub fn url_with_database(url: &str, database: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let authority_start = base.find("://").map_or(0, |i| i + 3);
    let base = match base[authority_start..].find('/') {
        Some(slash) => &base[..authority_start + slash],
        None => base,
    };
    match query {
        Some(query) => format!("{base}/{database}?{query}"),
        None => format!("{base}/{database}"),
    }
}

pub fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../serveur/migrations")
}

pub struct TestDb {
    pub name: String,
    pub pool: PgPool,
    admin: PgPool,
}

impl TestDb {
    /// A new database with every migration applied, as the migrating role.
    pub async fn create() -> Self {
        let admin = PgPool::connect(&database_url()).await.unwrap();
        let name = format!("lp_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .unwrap();
        let pool = PgPool::connect(&url_with_database(&database_url(), &name))
            .await
            .unwrap();
        let db = Self { name, pool, admin };
        db.migrate().await;
        db
    }

    async fn migrate(&self) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(migrations_dir())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "sql"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "no migration found");
        for file in files {
            let body = std::fs::read_to_string(&file).unwrap();
            // A migration file of this repository.
            sqlx::raw_sql(AssertSqlSafe(body))
                .execute(&self.pool)
                .await
                .unwrap_or_else(|e| panic!("migration {} failed: {e}", file.display()));
        }
    }

    /// The lookup the Capability reads: the read-only login of the
    /// environment, pointed at this database.
    pub fn vars(&self) -> impl Fn(&str) -> Option<String> + use<> {
        let url = url_with_database(&readonly_url(), &self.name);
        move |name| (name == DATABASE_URL_READONLY).then(|| url.clone())
    }

    pub async fn lister(&self) -> ListerPjs<Executor> {
        self.lister_in(&registry_dir()).await
    }

    pub async fn lister_in(&self, dir: &Path) -> ListerPjs<Executor> {
        let executor = Executor::from_variables(dir, self.vars())
            .await
            .expect("the executor starts on the test database");
        ListerPjs::new(executor)
    }

    pub async fn drop_db(self) {
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {} WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await
        .unwrap();
        self.admin.close().await;
    }
}

/// An invented campaign, seeded as the migrating role.
pub async fn campagne(pool: &PgPool, id: Uuid, nom: &str, cree_le: &str, archived: bool) -> Uuid {
    sqlx::query(
        r#"INSERT INTO campagne (id, nom, "creeLe", "archiveLe")
           VALUES ($1, $2, $3::timestamptz, CASE WHEN $4 THEN now() END)"#,
    )
    .bind(id)
    .bind(nom)
    .bind(cree_le)
    .bind(archived)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// An invented PC, seeded as the migrating role.
pub struct Seed<'a> {
    pub id: Uuid,
    pub campagne: Uuid,
    pub nom: &'a str,
    pub classe: &'a str,
    pub niveau: i32,
    pub cree_le: &'a str,
    pub archived: bool,
}

impl Seed<'_> {
    pub async fn insert(self, pool: &PgPool) -> Uuid {
        sqlx::query(
            r#"INSERT INTO pj (id, "campagneId", nom, classe, niveau, "creeLe", "archiveLe")
               VALUES ($1, $2, $3, $4, $5, $6::timestamptz, CASE WHEN $7 THEN now() END)"#,
        )
        .bind(self.id)
        .bind(self.campagne)
        .bind(self.nom)
        .bind(self.classe)
        .bind(self.niveau)
        .bind(self.cree_le)
        .bind(self.archived)
        .execute(pool)
        .await
        .unwrap();
        self.id
    }
}

/// An active PC with the least to say.
pub fn active<'a>(
    id: u128,
    campagne: Uuid,
    nom: &'a str,
    classe: &'a str,
    niveau: i32,
    cree_le: &'a str,
) -> Seed<'a> {
    Seed {
        id: Uuid::from_u128(id),
        campagne,
        nom,
        classe,
        niveau,
        cree_le,
        archived: false,
    }
}
