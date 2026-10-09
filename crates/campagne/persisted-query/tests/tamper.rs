//! Rules 16-18: the hash is recomputed from the bytes on disk on every call,
//! and a file that no longer matches the registry is refused by name before
//! any SQL is sent. Each test edits a copy of the query directory, never the
//! repository's files.

mod common;

use std::path::{Path, PathBuf};

use campagne_persisted_query::{QueryError, StartError, registry_dir};
use common::{TestDb, campagne, campagne_id, new_id, no_vars};
use serde_json::Value;

struct Copy(PathBuf);

impl Copy {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("pq-queries-{}", new_id()));
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
    edited[0] ^= 0x20; // 'q' of `query` becomes 'Q'
    std::fs::write(path, edited).unwrap();
    original
}

#[tokio::test]
async fn a_one_byte_edit_is_refused_by_name_and_a_restore_runs_again() {
    let db = TestDb::create().await;
    let dir = Copy::new();
    let exec = db.executor_in(&dir.0).await;
    let id = campagne(&db.pool, new_id(), "Active", "2026-01-01T00:00:00Z", false).await;
    exec.run("ListePjs", &campagne_id(id))
        .await
        .expect("untouched files run");

    let original = flip_first_byte(&dir.path("liste-pjs.graphql"));
    let err = exec.run("ListePjs", &campagne_id(id)).await.unwrap_err();
    assert!(
        matches!(&err, QueryError::Tampered { query } if query == "ListePjs"),
        "{err:?}"
    );
    assert!(err.to_string().contains("ListePjs"));

    // The other registered queries still run.
    exec.run("ListeCampagnes", &no_vars())
        .await
        .expect("another query still runs");
    exec.run("NiveauxPjsActifs", &campagne_id(id))
        .await
        .expect("another query still runs");

    // Restored: the same executor runs it again, so the hash is checked per call.
    std::fs::write(dir.path("liste-pjs.graphql"), original).unwrap();
    exec.run("ListePjs", &campagne_id(id))
        .await
        .expect("a restored file runs again");

    db.drop_db().await;
}

#[tokio::test]
async fn an_edited_sql_sidecar_is_refused_before_any_sql_is_sent() {
    let db = TestDb::create().await;
    let dir = Copy::new();
    let exec = db.executor_in(&dir.0).await;
    let id = campagne(&db.pool, new_id(), "Active", "2026-01-01T00:00:00Z", false).await;

    let original = std::fs::read(dir.path("liste-pjs.sql")).unwrap();
    // Were it sent, this would fail in the database, not as `Tampered`.
    std::fs::write(dir.path("liste-pjs.sql"), "SELECT 1/0").unwrap();
    let err = exec.run("ListePjs", &campagne_id(id)).await.unwrap_err();
    assert!(
        matches!(&err, QueryError::Tampered { query } if query == "ListePjs"),
        "{err:?}"
    );

    // A trailing newline is a change too: there is no normalisation.
    let mut with_newline = original.clone();
    with_newline.push(b'\n');
    std::fs::write(dir.path("liste-pjs.sql"), with_newline).unwrap();
    let err = exec.run("ListePjs", &campagne_id(id)).await.unwrap_err();
    assert!(matches!(err, QueryError::Tampered { .. }), "{err:?}");

    std::fs::write(dir.path("liste-pjs.sql"), original).unwrap();
    exec.run("ListePjs", &campagne_id(id)).await.unwrap();
    db.drop_db().await;
}

#[tokio::test]
async fn a_missing_file_is_unavailable() {
    let db = TestDb::create().await;
    let dir = Copy::new();
    let exec = db.executor_in(&dir.0).await;
    for file in ["liste-campagnes.graphql", "liste-campagnes.sql"] {
        let kept = std::fs::read(dir.path(file)).unwrap();
        std::fs::remove_file(dir.path(file)).unwrap();
        let err = exec.run("ListeCampagnes", &no_vars()).await.unwrap_err();
        assert!(
            matches!(&err, QueryError::Unavailable { query, .. } if query == "ListeCampagnes"),
            "{file}: {err:?}"
        );
        std::fs::write(dir.path(file), kept).unwrap();
    }
    exec.run("ListeCampagnes", &no_vars()).await.unwrap();
    db.drop_db().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_that_escapes_the_directory_is_unavailable() {
    let db = TestDb::create().await;
    let dir = Copy::new();
    let outside = std::env::temp_dir().join(format!("pq-outside-{}", new_id()));
    // The outside file has the very bytes the registry expects: only where it
    // sits is wrong.
    std::fs::copy(dir.path("liste-campagnes.graphql"), &outside).unwrap();
    let exec = db.executor_in(&dir.0).await;
    std::fs::remove_file(dir.path("liste-campagnes.graphql")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.path("liste-campagnes.graphql")).unwrap();

    let err = exec.run("ListeCampagnes", &no_vars()).await.unwrap_err();
    assert!(matches!(&err, QueryError::Unavailable { .. }), "{err:?}");

    // A symlink that stays inside the directory is harmless.
    std::fs::remove_file(dir.path("liste-campagnes.graphql")).unwrap();
    std::fs::copy(&outside, dir.path("copy.graphql")).unwrap();
    std::os::unix::fs::symlink("copy.graphql", dir.path("liste-campagnes.graphql")).unwrap();
    exec.run("ListeCampagnes", &no_vars()).await.unwrap();

    std::fs::remove_file(&outside).unwrap();
    db.drop_db().await;
}

#[tokio::test]
async fn an_unsound_registry_stops_the_start() {
    let db = TestDb::create().await;
    let text = std::fs::read_to_string(registry_dir().join("registry.json")).unwrap();
    let mut registry: Value = serde_json::from_str(&text).unwrap();
    let first = registry["queries"][0].clone();

    // A duplicate name.
    let mut duplicate = registry.clone();
    duplicate["queries"]
        .as_array_mut()
        .unwrap()
        .push(first.clone());
    // A file that leaves the directory.
    registry["queries"][0]["file"] = Value::from("../liste-campagnes.graphql");
    // An entry with a key contract C does not know.
    let mut extra = serde_json::from_str::<Value>(&text).unwrap();
    extra["queries"][0]["note"] = Value::from("x");

    for (what, bad) in [
        ("duplicate", duplicate),
        ("escape", registry),
        ("extra key", extra),
    ] {
        let dir = Copy::new();
        std::fs::write(dir.path("registry.json"), bad.to_string()).unwrap();
        let err = campagne_persisted_query::Executor::from_variables(&dir.0, db.vars())
            .await
            .unwrap_err();
        assert!(matches!(err, StartError::Registry(_)), "{what}: {err:?}");
    }
    db.drop_db().await;
}
