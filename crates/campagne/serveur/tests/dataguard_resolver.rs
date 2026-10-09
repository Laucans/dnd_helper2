//! The Resolver: an archive carried along `onDelete: archive`, all or
//! nothing, under one version; a second archive is a no-op; nothing is ever
//! removed.

mod common;

use std::time::Duration;

use common::contracts::{J, assert_valid};
use common::test_commands::Harness;
use dataguard::{Clock, DATA_VERSION_CHANNEL, State};
use serde_json::{Value, json};
use sqlx::postgres::PgListener;
use uuid::Uuid;

async fn counts(h: &Harness) -> (i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM campagne), (SELECT count(*) FROM pj)")
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn archiving_a_campaign_archives_its_active_pcs_under_one_version() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let other = h.campaign(&mut applier, "Le Val").await;
    let active: Vec<Uuid> = [
        h.pc(&mut applier, c, "Ysolde", 3).await,
        h.pc(&mut applier, c, "Brume", 4).await,
        h.pc(&mut applier, c, "Corvin", 5).await,
    ]
    .into();
    let gone = h.pc(&mut applier, c, "Disparu", 2).await;
    h.archive_pc(gone).await;
    applier.drain().await.unwrap();
    let gone_at = h.pc_row(gone).await.2.unwrap();
    let elsewhere = h.pc(&mut applier, other, "Sorcha", 6).await;
    let before = counts(&h).await;
    let version = h.version().await;

    h.clock.advance(Duration::from_secs(3600));
    let at = h.clock.now();
    let archive = h.archive_campaign(c).await;
    assert_eq!(applier.drain().await.unwrap(), 1);
    let r = h.result(archive.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert_eq!(r.data_version, Some(version + 1));
    assert_eq!(h.version().await, version + 1);

    assert_eq!(h.campaign_archived_at(c).await, Some(at));
    for pc in &active {
        assert_eq!(h.pc_row(*pc).await.2, Some(at));
    }
    assert_eq!(h.pc_row(gone).await.2, Some(gone_at), "tombstone kept");
    assert_eq!(h.pc_row(elsewhere).await.2, None);
    assert_eq!(h.campaign_archived_at(other).await, None);
    assert_eq!(counts(&h).await, before, "nothing removed");
    h.assert_no_orphan().await;

    let (plan, effects): (sqlx::types::Json<Value>, Vec<String>) =
        sqlx::query_as("SELECT impact_plan, effects FROM dataguard_queue WHERE command = $1")
            .bind(archive.command_id.0)
            .fetch_one(&h.db.pool)
            .await
            .unwrap();
    assert_valid(J, &plan.0);
    assert_eq!(
        plan.0["cascade"],
        json!([{ "relation": "PJ.campagneId", "rows": 3, "policy": "archive" }])
    );
    assert_eq!(plan.0["decision"], "auto");
    for pc in &active {
        assert!(
            effects.contains(&format!("PJ/{pc}.archivedAt")),
            "{effects:?}"
        );
    }
    assert!(effects.contains(&format!("Campagne/{c}.archivedAt")));
    h.drop_db().await;
}

#[tokio::test]
async fn a_failing_cascade_archives_nothing() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pcs = [
        h.pc(&mut applier, c, "A", 3).await,
        h.pc(&mut applier, c, "B", 3).await,
        h.pc(&mut applier, c, "C", 3).await,
    ];
    sqlx::raw_sql(
        "CREATE FUNCTION panne() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN IF NEW.nom = 'B' THEN RAISE EXCEPTION 'panne'; END IF; RETURN NEW; END $$;
         CREATE TRIGGER panne BEFORE UPDATE ON pj FOR EACH ROW EXECUTE FUNCTION panne();",
    )
    .execute(&h.db.pool)
    .await
    .unwrap();
    let rows = h.rows().await;
    let version = h.version().await;

    let archive = h.archive_campaign(c).await;
    assert!(applier.apply_next().await.is_err());
    assert_eq!(h.rows().await, rows, "neither the campaign nor any PC");
    assert_eq!(h.campaign_archived_at(c).await, None);
    for pc in pcs {
        assert_eq!(h.pc_row(pc).await.2, None);
    }
    assert_eq!(h.state(archive.command_id).await, State::Queued);
    assert_eq!(h.version().await, version);

    sqlx::raw_sql("DROP TRIGGER panne ON pj")
        .execute(&h.db.pool)
        .await
        .unwrap();
    // Its scope is set aside until the backoff is over on the engine clock.
    assert_eq!(applier.drain().await.unwrap(), 0);
    assert_eq!(h.state(archive.command_id).await, State::Queued);
    h.clock.advance(Duration::from_secs(1));
    assert_eq!(applier.drain().await.unwrap(), 1);
    assert_eq!(h.result(archive.command_id).await.status, State::Applied);
    h.assert_no_orphan().await;
    h.drop_db().await;
}

#[tokio::test]
async fn archiving_again_is_an_applied_no_op_that_keeps_the_tombstone_and_notifies_nobody() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let mut listener = PgListener::connect_with(&h.db.pool).await.unwrap();
    listener.listen(DATA_VERSION_CHANNEL).await.unwrap();

    h.archive_pc(pc).await;
    applier.drain().await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(1), listener.recv())
        .await
        .expect("an applied command notifies")
        .unwrap();
    let version = h.version().await;
    assert_eq!(first.payload(), version.to_string());
    let pc_at = h.pc_row(pc).await.2.unwrap();

    h.archive_campaign(c).await;
    applier.drain().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), listener.recv())
        .await
        .expect("an applied command notifies")
        .unwrap();
    let c_at = h.campaign_archived_at(c).await.unwrap();
    let version = h.version().await;

    h.clock.advance(Duration::from_secs(60));
    let rows = h.rows().await;
    let again_pc = h.archive_pc(pc).await;
    let again_c = h.archive_campaign(c).await;
    applier.drain().await.unwrap();
    for s in [&again_pc, &again_c] {
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Applied);
        assert_eq!(r.violations, Vec::<String>::new());
        assert_eq!(
            r.data_version,
            Some(version),
            "the current version, not a new one"
        );
    }
    assert_eq!(h.version().await, version);
    assert_eq!(h.rows().await, rows);
    assert_eq!(h.pc_row(pc).await.2, Some(pc_at));
    assert_eq!(h.campaign_archived_at(c).await, Some(c_at));
    assert!(
        tokio::time::timeout(Duration::from_millis(300), listener.recv())
            .await
            .is_err(),
        "a no-op notified"
    );
    // The listener holds a pool connection: release it before the pool closes.
    drop(listener);
    h.drop_db().await;
}

#[tokio::test]
async fn archiving_an_unknown_row_is_rejected() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.archive_campaign(Uuid::new_v4()).await;
    let pc = h.archive_pc(Uuid::new_v4()).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(c.command_id).await.violations, ["campaign-active"]);
    assert_eq!(h.result(pc.command_id).await.violations, ["pc-active"]);
    assert_eq!(h.version().await, 0);
    h.drop_db().await;
}

#[tokio::test]
async fn an_add_racing_a_campaign_archive_is_settled_by_apply_order() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    // Lands first: archived by the cascade.
    let before = h.add_pc(c, "Ysolde", json!(3)).await;
    let archive = h.archive_campaign(c).await;
    // Lands after: rejected.
    let after = h.add_pc(c, "Brume", json!(3)).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(before.command_id).await.status, State::Applied);
    assert_eq!(h.result(archive.command_id).await.status, State::Applied);
    assert_eq!(
        h.result(after.command_id).await.violations,
        ["campaign-active"]
    );
    let at = h.campaign_archived_at(c).await;
    assert_eq!(h.pc_row(common::test_commands::id_of(&before)).await.2, at);
    h.assert_no_orphan().await;
    h.drop_db().await;
}
