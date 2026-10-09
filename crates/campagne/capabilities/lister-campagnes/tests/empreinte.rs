//! A query file that no longer matches its registered hash is refused by name
//! before any SQL is sent: no data, no fallback. Each test edits a copy of the
//! query directory, never the repository's files.

mod common;

use std::path::{Path, PathBuf};

use campagne_persisted_query::{QueryError, registry_dir};
use common::{TestDb, campagne, id};
use lister_campagnes::{ListerError, QUERY, lister_campagnes};

struct Copy(PathBuf);

impl Copy {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("lc-queries-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for item in std::fs::read_dir(registry_dir()).unwrap() {
            let path = item.unwrap().path();
            if path.is_file() {
                std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
            }
        }
        Self(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn flip_first_byte(path: &Path) -> Vec<u8> {
    let original = std::fs::read(path).unwrap();
    let mut edited = original.clone();
    edited[0] ^= 0x20;
    std::fs::write(path, edited).unwrap();
    original
}

async fn refused_then_restored(file: &str) {
    let db = TestDb::create().await;
    campagne(&db.pool, id(1), "Alpha", "2026-01-01T00:00:00Z", false).await;
    let copy = Copy::new();
    let executor = db.executor_in(&copy.0).await;
    assert_eq!(lister_campagnes(&executor).await.unwrap().len(), 1);

    let original = flip_first_byte(&copy.path(file));
    let result = lister_campagnes(&executor).await;
    match result {
        Err(ListerError::Query(QueryError::Tampered { query })) => assert_eq!(query, QUERY),
        other => panic!("an edited {file} must be refused, got {other:?}"),
    }

    std::fs::write(copy.path(file), original).unwrap();
    assert_eq!(lister_campagnes(&executor).await.unwrap().len(), 1);
    db.drop_db().await;
}

#[tokio::test]
async fn an_edited_graphql_file_is_refused_and_no_data_is_returned() {
    refused_then_restored("liste-campagnes.graphql").await;
}

#[tokio::test]
async fn an_edited_sql_sidecar_is_refused_and_no_data_is_returned() {
    refused_then_restored("liste-campagnes.sql").await;
}
