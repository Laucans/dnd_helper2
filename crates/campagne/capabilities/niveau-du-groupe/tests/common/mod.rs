//! A fresh, migrated database per test, created and dropped through the
//! migrating variable, and a Capability wired to it through the read-only one.
//! A missing variable fails the test: a database test never skips.
//!
//! A trimmed copy of the persisted-query crate's helper: test plumbing, not
//! shared business code. The migrations are the serveur's files, applied raw.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use campagne_niveau_du_groupe::Capability;
use campagne_persisted_query::DATABASE_URL_READONLY;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

pub fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for database tests")
}

pub fn readonly_url() -> String {
    std::env::var(DATABASE_URL_READONLY)
        .expect("DATABASE_URL_READONLY must be set for the read-only role tests")
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
        let name = format!("nvg_{}", Uuid::new_v4().simple());
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

    pub async fn capability(&self) -> Capability {
        Capability::from_variables(self.vars())
            .await
            .expect("the Capability starts on the test database")
    }

    /// The data version, read as the migrating role.
    pub async fn data_version(&self) -> i64 {
        sqlx::query_scalar(r#"SELECT "dataVersion" FROM data_version"#)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// What an applied command does to the counter: one more version.
    pub async fn bump_version(&self) {
        sqlx::query("UPDATE dataguard_version SET version = version + 1")
            .execute(&self.pool)
            .await
            .unwrap();
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
pub async fn campagne(pool: &PgPool, nom: &str, archived: bool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO campagne (id, nom, "archiveLe")
           VALUES ($1, $2, CASE WHEN $3 THEN now() END)"#,
    )
    .bind(id)
    .bind(nom)
    .bind(archived)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// An invented PC, seeded as the migrating role.
pub async fn pj(pool: &PgPool, campagne: Uuid, nom: &str, niveau: i32, archived: bool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO pj (id, "campagneId", nom, classe, niveau, "archiveLe")
           VALUES ($1, $2, $3, 'Guerrier', $4, CASE WHEN $5 THEN now() END)"#,
    )
    .bind(id)
    .bind(campagne)
    .bind(nom)
    .bind(niveau)
    .bind(archived)
    .execute(pool)
    .await
    .unwrap();
    id
}
