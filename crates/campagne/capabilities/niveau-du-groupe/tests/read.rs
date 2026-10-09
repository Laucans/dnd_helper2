//! The Capability on seeded rows of a real database. Nothing is mocked: the
//! views, the registered query and the read-only role are the real ones.

mod common;

use campagne_niveau_du_groupe::{Error, NiveauDuGroupe};
use campagne_persisted_query::connect_options;
use common::{TestDb, campagne, pj};
use sqlx::{AssertSqlSafe, Connection, PgConnection};
use uuid::Uuid;

#[tokio::test]
async fn an_existing_campaign_without_pc_is_an_empty_party_at_the_version_read() {
    let db = TestDb::create().await;
    let id = campagne(&db.pool, "Alpha", false).await;
    let cap = db.capability().await;

    let out = cap.run(&id.to_string()).await.unwrap();
    assert_eq!(
        out,
        NiveauDuGroupe {
            level: None,
            pc_count: 0,
            model: "v1",
            as_of: u64::try_from(db.data_version().await).unwrap(),
        }
    );
    db.drop_db().await;
}

#[tokio::test]
async fn the_levels_of_the_active_pcs_are_averaged_half_up() {
    let db = TestDb::create().await;
    let id = campagne(&db.pool, "Alpha", false).await;
    pj(&db.pool, id, "Aria", 3, false).await;
    pj(&db.pool, id, "Borek", 4, false).await;
    let cap = db.capability().await;

    let out = cap.run(&id.to_string()).await.unwrap();
    assert_eq!((out.level, out.pc_count), (Some(4), 2));
    db.drop_db().await;
}

#[tokio::test]
async fn an_unknown_an_archived_and_a_malformed_id_answer_the_same() {
    let db = TestDb::create().await;
    let archived = campagne(&db.pool, "Morte", true).await;
    // Active PCs under an archived campaign must not make it answer.
    pj(&db.pool, archived, "Fantôme", 9, false).await;
    let cap = db.capability().await;

    let unknown = cap.run(&Uuid::new_v4().to_string()).await.unwrap_err();
    let tombstoned = cap.run(&archived.to_string()).await.unwrap_err();
    let malformed = cap.run("not-a-uuid").await.unwrap_err();
    // A real id with stray whitespace is a malformed one, not a read failure.
    let padded = cap.run(&format!(" {archived} ")).await.unwrap_err();
    for error in [&unknown, &tombstoned, &malformed, &padded] {
        assert!(matches!(error, Error::NotFound), "{error:?}");
    }
    assert_eq!(unknown.to_string(), tombstoned.to_string());
    assert_eq!(unknown.to_string(), malformed.to_string());
    assert_eq!(unknown.to_string(), padded.to_string());
    db.drop_db().await;
}

#[tokio::test]
async fn a_blank_id_is_refused_before_any_read() {
    let db = TestDb::create().await;
    let cap = db.capability().await;
    for blank in ["", "  ", "\t\n"] {
        let error = cap.run(blank).await.unwrap_err();
        assert!(matches!(error, Error::MissingCampaignId), "{error:?}");
    }
    db.drop_db().await;
}

#[tokio::test]
async fn a_read_after_a_command_never_returns_the_old_level() {
    let db = TestDb::create().await;
    let id = campagne(&db.pool, "Alpha", false).await;
    let first = pj(&db.pool, id, "Aria", 3, false).await;
    pj(&db.pool, id, "Borek", 4, false).await;
    let cap = db.capability().await;

    let before = cap.run(&id.to_string()).await.unwrap();
    assert_eq!((before.level, before.pc_count), (Some(4), 2));

    // An edit, as an applied command leaves the data: new row, one more version.
    sqlx::query("UPDATE pj SET niveau = 1 WHERE id = $1")
        .bind(first)
        .execute(&db.pool)
        .await
        .unwrap();
    db.bump_version().await;
    let edited = cap.run(&id.to_string()).await.unwrap();
    assert_eq!((edited.level, edited.pc_count), (Some(3), 2));
    assert!(edited.as_of > before.as_of);

    // An archive: the PC leaves the party.
    sqlx::query(r#"UPDATE pj SET "archiveLe" = now() WHERE id = $1"#)
        .bind(first)
        .execute(&db.pool)
        .await
        .unwrap();
    db.bump_version().await;
    let archived = cap.run(&id.to_string()).await.unwrap();
    assert_eq!((archived.level, archived.pc_count), (Some(4), 1));
    assert!(archived.as_of > edited.as_of);

    // An addition.
    pj(&db.pool, id, "Cyra", 8, false).await;
    db.bump_version().await;
    let added = cap.run(&id.to_string()).await.unwrap();
    assert_eq!((added.level, added.pc_count), (Some(6), 2));
    assert!(added.as_of > archived.as_of);
    db.drop_db().await;
}

#[tokio::test]
async fn a_pc_of_another_campaign_never_changes_the_result() {
    let db = TestDb::create().await;
    let a = campagne(&db.pool, "Alpha", false).await;
    let b = campagne(&db.pool, "Beta", false).await;
    pj(&db.pool, a, "Aria", 3, false).await;
    let cap = db.capability().await;
    let before = cap.run(&a.to_string()).await.unwrap();

    pj(&db.pool, b, "Borek", 19, false).await;
    pj(&db.pool, b, "Cyra", 20, false).await;
    let after = cap.run(&a.to_string()).await.unwrap();
    assert_eq!((after.level, after.pc_count), (Some(3), 1));
    assert_eq!(before, after);

    let other = cap.run(&b.to_string()).await.unwrap();
    assert_eq!((other.level, other.pc_count), (Some(20), 2));
    db.drop_db().await;
}

#[tokio::test]
async fn a_write_through_the_connection_it_uses_fails_at_the_database() {
    let db = TestDb::create().await;
    let id = campagne(&db.pool, "Alpha", false).await;
    let mut conn = PgConnection::connect_with(&connect_options(db.vars()).unwrap())
        .await
        .unwrap();
    let writes = [
        format!(
            "INSERT INTO campagne (id, nom) VALUES ('{}', 'X')",
            Uuid::new_v4()
        ),
        format!("UPDATE campagne SET nom = 'Y' WHERE id = '{id}'"),
        "DELETE FROM pj".to_owned(),
        "UPDATE dataguard_version SET version = version + 1".to_owned(),
    ];
    for statement in writes {
        let error = sqlx::raw_sql(AssertSqlSafe(statement.clone()))
            .execute(&mut conn)
            .await
            .expect_err(&format!("`{statement}` must fail"));
        let code = error
            .as_database_error()
            .and_then(|e| e.code())
            .map(|c| c.into_owned());
        assert!(
            matches!(code.as_deref(), Some("42501" | "25006")),
            "`{statement}` failed with {code:?}"
        );
    }
    conn.close().await.unwrap();
    db.drop_db().await;
}
