//! A fresh database per test, created and dropped through `DATABASE_URL`.
//! A missing variable fails the test: a database test never skips.
#![allow(dead_code)] // each test binary uses its own part of this module

use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

pub fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for database tests")
}

pub fn readonly_url() -> String {
    std::env::var("DATABASE_URL_READONLY")
        .expect("DATABASE_URL_READONLY must be set for the read-only role test")
}

/// `url` with its database name replaced by `database`.
pub fn url_with_database(url: &str, database: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let authority_end = base.find("://").map_or(0, |i| i + 3);
    let base = match base[authority_end..].find('/') {
        Some(i) => &base[..authority_end + i],
        None => base,
    };
    match query {
        Some(query) => format!("{base}/{database}?{query}"),
        None => format!("{base}/{database}"),
    }
}

pub fn readonly_options(database: &str) -> PgConnectOptions {
    PgConnectOptions::from_str(&readonly_url())
        .expect("DATABASE_URL_READONLY is a PostgreSQL URL")
        .database(database)
}

pub struct TestDb {
    pub name: String,
    pub url: String,
    pub pool: PgPool,
    admin: PgPool,
}

impl TestDb {
    pub async fn create() -> Self {
        let admin_url = database_url();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(PgConnectOptions::from_str(&admin_url).unwrap())
            .await
            .expect("connect to DATABASE_URL");
        let name = format!("t_{}", uuid::Uuid::new_v4().simple());
        sqlx::raw_sql(AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .execute(&admin)
            .await
            .expect("create the test database");
        let url = url_with_database(&admin_url, &name);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(PgConnectOptions::from_str(&url).unwrap())
            .await
            .expect("connect to the test database");
        Self {
            name,
            url,
            pool,
            admin,
        }
    }

    pub async fn drop_db(self) {
        self.pool.close().await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP DATABASE \"{}\" WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await
        .expect("drop the test database");
        self.admin.close().await;
    }
}

/// The SQLSTATE of a failed statement; panics when it succeeded.
pub fn code<T>(result: Result<T, sqlx::Error>) -> String {
    match result {
        Ok(_) => panic!("the statement succeeded; a database error was expected"),
        Err(e) => e
            .as_database_error()
            .and_then(|d| d.code())
            .map(|c| c.into_owned())
            .unwrap_or_else(|| "n/a".into()),
    }
}

pub async fn table_exists(pool: &PgPool, table: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
        .bind(table)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[test]
fn url_with_database_swaps_only_the_name() {
    assert_eq!(
        url_with_database("postgres://u:p@h:5432/app", "t_1"),
        "postgres://u:p@h:5432/t_1"
    );
    assert_eq!(
        url_with_database("postgres://u:p@h/app?sslmode=disable", "t_1"),
        "postgres://u:p@h/t_1?sslmode=disable"
    );
    assert_eq!(url_with_database("postgres://h", "t_1"), "postgres://h/t_1");
}
