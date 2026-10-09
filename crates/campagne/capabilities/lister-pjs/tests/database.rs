//! The Capability through the real executor, the real views and the real
//! read-only role. Isolation, tombstones and order are proven on seeded rows,
//! so a filter missing from the Data layer fails here.

mod common;

use std::path::{Path, PathBuf};

use campagne_lister_pjs::{
    ListerPjs, ListerPjsError, Pj, QueryError, ReadFailure, StartError, registry_dir,
};
use campagne_persisted_query::{DATABASE_URL_READONLY, Executor, connect_options};
use common::{Seed, TestDb, active, campagne, database_url};
use sqlx::{Connection, PgConnection};
use uuid::Uuid;

const T1: &str = "2026-01-01T00:00:00Z";
const T2: &str = "2026-01-02T00:00:00Z";
const T3: &str = "2026-01-03T00:00:00Z";

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn pj(n: u128, name: &str, class: &str, level: i32) -> Pj {
    Pj {
        id: id(n),
        name: name.to_owned(),
        class: class.to_owned(),
        level,
    }
}

fn archived<'a>(
    n: u128,
    campagne: Uuid,
    nom: &'a str,
    classe: &'a str,
    niveau: i32,
    cree_le: &'a str,
) -> Seed<'a> {
    Seed {
        archived: true,
        ..active(n, campagne, nom, classe, niveau, cree_le)
    }
}

#[tokio::test]
async fn a_pc_of_campaign_a_never_appears_in_bs_list() {
    let db = TestDb::create().await;
    let a = campagne(&db.pool, id(0xA), "Alpha", T1, false).await;
    let b = campagne(&db.pool, id(0xB), "Bravo", T1, false).await;
    // "Brune" is shared by the two campaigns on purpose.
    active(1, a, "Aurélien", "Guerrier", 4, T1)
        .insert(&db.pool)
        .await;
    active(2, a, "Brune", "Rôdeuse", 3, T2)
        .insert(&db.pool)
        .await;
    archived(3, a, "Archive-A", "Clerc", 9, T1)
        .insert(&db.pool)
        .await;
    active(11, b, "Brune", "Magicienne", 7, T1)
        .insert(&db.pool)
        .await;
    active(12, b, "Cécile", "Barde", 8, T2)
        .insert(&db.pool)
        .await;
    archived(13, b, "Archive-B", "Voleur", 2, T1)
        .insert(&db.pool)
        .await;

    let lister = db.lister().await;
    let of_a = lister.lister(&a.to_string()).await.unwrap();
    let of_b = lister.lister(&b.to_string()).await.unwrap();

    assert_eq!(
        of_a,
        [
            pj(1, "Aurélien", "Guerrier", 4),
            pj(2, "Brune", "Rôdeuse", 3)
        ]
    );
    assert_eq!(
        of_b,
        [
            pj(11, "Brune", "Magicienne", 7),
            pj(12, "Cécile", "Barde", 8)
        ]
    );
    assert!(of_a.iter().all(|x| of_b.iter().all(|y| x.id != y.id)));

    db.drop_db().await;
}

#[tokio::test]
async fn an_archived_pc_never_appears() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(1), "Alpha", T1, false).await;
    archived(1, c, "Ancien", "Clerc", 5, T1)
        .insert(&db.pool)
        .await;
    active(2, c, "Actif", "Guerrier", 5, T2)
        .insert(&db.pool)
        .await;

    let pjs = db.lister().await.lister(&c.to_string()).await.unwrap();
    assert_eq!(pjs, [pj(2, "Actif", "Guerrier", 5)]);

    db.drop_db().await;
}

#[tokio::test]
async fn a_campaign_without_pcs_is_an_empty_list() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(1), "Vide", T1, false).await;

    let pjs = db.lister().await.lister(&c.to_string()).await.unwrap();
    assert!(pjs.is_empty());

    db.drop_db().await;
}

#[tokio::test]
async fn a_campaign_whose_pcs_are_all_archived_is_an_empty_list() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(1), "Alpha", T1, false).await;
    archived(1, c, "Un", "Clerc", 5, T1).insert(&db.pool).await;
    archived(2, c, "Deux", "Clerc", 5, T2)
        .insert(&db.pool)
        .await;

    let pjs = db.lister().await.lister(&c.to_string()).await.unwrap();
    assert!(pjs.is_empty());

    db.drop_db().await;
}

#[tokio::test]
async fn pcs_come_oldest_first_ties_by_id_and_the_same_every_time() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(1), "Alpha", T1, false).await;
    // Inserted newest first, and the tied pair with the larger id first.
    active(5, c, "Récent", "Barde", 3, T3)
        .insert(&db.pool)
        .await;
    active(2, c, "Jumeau-2", "Barde", 3, T2)
        .insert(&db.pool)
        .await;
    active(1, c, "Jumeau-1", "Barde", 3, T2)
        .insert(&db.pool)
        .await;
    active(9, c, "Ancien", "Barde", 3, T1)
        .insert(&db.pool)
        .await;

    let lister = db.lister().await;
    let first = lister.lister(&c.to_string()).await.unwrap();
    let names: Vec<&str> = first.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Ancien", "Jumeau-1", "Jumeau-2", "Récent"]);
    assert_eq!(lister.lister(&c.to_string()).await.unwrap(), first);

    db.drop_db().await;
}

async fn not_found_for(lister: &ListerPjs<Executor>, campagne_id: &str) -> ListerPjsError {
    let err = lister.lister(campagne_id).await.unwrap_err();
    assert!(matches!(err, ListerPjsError::NotFound), "{err:?}");
    err
}

#[tokio::test]
async fn an_unknown_campaign_is_not_found() {
    let db = TestDb::create().await;
    let lister = db.lister().await;

    not_found_for(&lister, &Uuid::new_v4().to_string()).await;
    // The nil id is well formed, so it is not-found, not invalid input.
    not_found_for(&lister, &Uuid::nil().to_string()).await;

    db.drop_db().await;
}

#[tokio::test]
async fn an_archived_campaign_is_not_found_even_with_active_looking_pcs() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(1), "Close", T1, true).await;
    // A cascade that is half applied: the PC row is still active.
    active(1, c, "Reste", "Guerrier", 5, T1)
        .insert(&db.pool)
        .await;

    not_found_for(&db.lister().await, &c.to_string()).await;

    db.drop_db().await;
}

#[tokio::test]
async fn a_foreign_campaign_id_is_not_found() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(0xA), "Alpha", T1, false).await;
    let pc = active(1, c, "Aurélien", "Guerrier", 4, T1)
        .insert(&db.pool)
        .await;

    // Well formed, present in the database, and not an active campaign.
    not_found_for(&db.lister().await, &pc.to_string()).await;

    db.drop_db().await;
}

#[tokio::test]
async fn the_three_not_founds_are_identical() {
    let db = TestDb::create().await;
    let active_campaign = campagne(&db.pool, id(0xA), "Alpha", T1, false).await;
    let closed = campagne(&db.pool, id(0xC), "Close", T1, true).await;
    let pc = active(1, active_campaign, "Aurélien", "Guerrier", 4, T1)
        .insert(&db.pool)
        .await;
    let lister = db.lister().await;

    let unknown = not_found_for(&lister, &Uuid::new_v4().to_string()).await;
    let archived = not_found_for(&lister, &closed.to_string()).await;
    let foreign = not_found_for(&lister, &pc.to_string()).await;

    assert_eq!(unknown.to_string(), archived.to_string());
    assert_eq!(archived.to_string(), foreign.to_string());
    assert_eq!(format!("{unknown:?}"), format!("{archived:?}"));
    assert_eq!(format!("{archived:?}"), format!("{foreign:?}"));

    db.drop_db().await;
}

struct Copy(PathBuf);

impl Copy {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("lp-queries-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for item in std::fs::read_dir(registry_dir()).unwrap() {
            let path = item.unwrap().path();
            if path.is_file() {
                std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
            }
        }
        Self(dir)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn flip_first_byte(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[0] ^= 0x20;
    std::fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn a_tampered_query_file_fails_closed() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(1), "Alpha", T1, false).await;
    active(1, c, "Aurélien", "Guerrier", 4, T1)
        .insert(&db.pool)
        .await;

    for file in ["liste-pjs.graphql", "liste-pjs.sql"] {
        let dir = Copy::new();
        let lister = db.lister_in(&dir.0).await;
        assert_eq!(lister.lister(&c.to_string()).await.unwrap().len(), 1);

        flip_first_byte(&dir.0.join(file));
        let err = lister.lister(&c.to_string()).await.unwrap_err();
        assert!(
            matches!(
                &err,
                ListerPjsError::ReadFailed(ReadFailure::Query(QueryError::Tampered { query }))
                    if query == "ListePjs"
            ),
            "{file}: {err:?}"
        );
    }

    db.drop_db().await;
}

#[tokio::test]
async fn it_starts_on_the_read_only_variable_alone() {
    let db = TestDb::create().await;
    let url = common::url_with_database(&database_url(), &db.name);
    let migrating = url.clone();

    // Only the migrating variable is set: no fallback.
    let Err(err) = Executor::from_variables(&registry_dir(), move |name| {
        (name == "DATABASE_URL").then(|| migrating.clone())
    })
    .await
    else {
        panic!("must not start");
    };
    assert!(
        matches!(err, StartError::Missing(name) if name == DATABASE_URL_READONLY),
        "{err:?}"
    );
    assert!(!err.to_string().contains(&url));

    // A blank value is as good as none.
    let Err(err) = Executor::from_variables(&registry_dir(), |name| {
        (name == DATABASE_URL_READONLY).then(|| "   ".to_owned())
    })
    .await
    else {
        panic!("must not start");
    };
    assert!(matches!(err, StartError::Missing(_)), "{err:?}");

    db.drop_db().await;
}

#[tokio::test]
async fn a_write_through_the_capabilitys_connection_fails_at_the_database() {
    let db = TestDb::create().await;
    let c = campagne(&db.pool, id(1), "Alpha", T1, false).await;
    active(1, c, "Aurélien", "Guerrier", 4, T1)
        .insert(&db.pool)
        .await;

    let mut conn = PgConnection::connect_with(&connect_options(db.vars()).unwrap())
        .await
        .unwrap();
    let writes = [
        format!("INSERT INTO pj (\"campagneId\", nom, classe, niveau) VALUES ('{c}', 'X', 'Y', 1)"),
        "UPDATE pj SET niveau = 20".to_owned(),
        "DELETE FROM pj".to_owned(),
        "UPDATE campagne SET nom = 'Y'".to_owned(),
    ];
    for statement in &writes {
        let error = sqlx::raw_sql(sqlx::AssertSqlSafe(statement.clone()))
            .execute(&mut conn)
            .await
            .expect_err(&format!("`{statement}` must fail"));
        let code = error
            .as_database_error()
            .and_then(|e| e.code())
            .map(|c| c.into_owned())
            .unwrap_or_default();
        assert!(
            ["25006", "42501"].contains(&code.as_str()),
            "`{statement}` failed with {code}"
        );
    }
    conn.close().await.unwrap();

    // Nothing changed.
    let pjs = db.lister().await.lister(&c.to_string()).await.unwrap();
    assert_eq!(pjs, [pj(1, "Aurélien", "Guerrier", 4)]);

    db.drop_db().await;
}

#[tokio::test]
async fn a_malformed_id_is_invalid_input_not_not_found_through_the_real_executor() {
    let db = TestDb::create().await;
    let lister = db.lister().await;

    // The executor itself answers not-found to a non-id; the Capability
    // must refuse it before the read.
    for bad in ["", "abc", "1234"] {
        let err = lister.lister(bad).await.unwrap_err();
        assert!(
            matches!(err, ListerPjsError::InvalidInput),
            "{bad:?}: {err:?}"
        );
    }
    let err = lister
        .call(&serde_json::json!({"campagneId": "abc"}))
        .await
        .unwrap_err();
    assert!(matches!(err, ListerPjsError::InvalidInput), "{err:?}");

    db.drop_db().await;
}
