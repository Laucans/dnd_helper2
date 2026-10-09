//! The invariants at both checks: payload-only violations reject at enqueue;
//! every invariant runs again at application on the real state, and that
//! check decides.

mod common;

use common::test_commands::{CREER_CAMPAGNE, Harness, RENOMMER_CAMPAGNE, id_of};
use dataguard::{Lookup, State};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn two_adds_with_the_same_name_end_applied_then_rejected() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    let first = h.add_pc(c, "Ysolde", json!(3)).await;
    let second = h.add_pc(c, "  YSOLDE\u{00A0}", json!(4)).await;
    // The enqueue check saw the first on the projected state: a warning only.
    assert!(first.warnings.is_empty());
    assert_eq!(second.warnings, ["pc-name-unique-in-campaign"]);
    assert_eq!(second.lookup.state(), State::Queued);

    applier.drain().await.unwrap();
    assert_eq!(h.result(first.command_id).await.status, State::Applied);
    let rejected = h.result(second.command_id).await;
    assert_eq!(rejected.status, State::Rejected);
    assert_eq!(rejected.violations, ["pc-name-unique-in-campaign"]);
    assert_eq!(rejected.data_version, None);
    h.drop_db().await;
}

#[tokio::test]
async fn unicode_forms_and_case_are_one_name_at_application() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    h.pc(&mut applier, c, "Élan", 2).await;

    for same in ["élan", "E\u{0301}lan", " ÉLAN "] {
        let s = h.add_pc(c, same, json!(2)).await;
        applier.drain().await.unwrap();
        let r = h.result(s.command_id).await;
        assert_eq!(r.violations, ["pc-name-unique-in-campaign"], "{same:?}");
    }
    // Stored in NFC, trimmed.
    let stored: Vec<String> = sqlx::query_scalar("SELECT nom FROM pj")
        .fetch_all(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(stored, ["\u{00C9}lan"]);
    h.drop_db().await;
}

#[tokio::test]
async fn an_archived_name_or_another_campaigns_name_does_not_block() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let other = h.campaign(&mut applier, "Le Val").await;
    let brume = h.pc(&mut applier, c, "Brume", 2).await;
    h.archive_pc(brume).await;
    h.pc(&mut applier, other, "Sorcha", 5).await;
    applier.drain().await.unwrap();

    h.pc(&mut applier, c, "brume", 2).await;
    h.pc(&mut applier, c, "Sorcha", 2).await;
    h.drop_db().await;
}

#[tokio::test]
async fn an_edit_queued_before_an_archive_and_applied_after_it_is_rejected_pc_active() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;

    // Two edits on the same base: the second waits for its author.
    let first = h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let late = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    // It passed the enqueue check: no invariant was at risk.
    assert!(late.warnings.is_empty());
    assert_eq!(late.lookup.state(), State::AwaitingConfirmation);
    applier.drain().await.unwrap();
    assert_eq!(h.result(first.command_id).await.status, State::Applied);
    assert_eq!(h.state(late.command_id).await, State::Parked);

    // The archive lands while the edit is parked.
    let archive = h.archive_pc(pc).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(archive.command_id).await.status, State::Applied);

    // Confirmed, it comes back after the archive: the real state decides.
    let before = h.rows().await;
    let version = h.version().await;
    h.engine
        .confirm(late.command_id, dataguard::GM_IDENTITY)
        .await
        .unwrap();
    applier.drain().await.unwrap();
    let r = h.result(late.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert_eq!(r.violations, ["pc-active"]);
    assert_eq!(r.data_version, None);
    assert_eq!(h.rows().await, before);
    assert_eq!(h.version().await, version);
    h.drop_db().await;
}

#[tokio::test]
async fn an_add_applied_after_its_campaigns_archive_is_rejected_campaign_active() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    let archive = h.archive_campaign(c).await;
    let add = h.add_pc(c, "Ysolde", json!(3)).await;
    assert_eq!(add.warnings, ["campaign-active"]);
    applier.drain().await.unwrap();
    assert_eq!(h.result(archive.command_id).await.status, State::Applied);
    let r = h.result(add.command_id).await;
    assert_eq!(r.violations, ["campaign-active"]);
    let pcs: i64 = sqlx::query_scalar("SELECT count(*) FROM pj")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(pcs, 0);

    // A PC add to an unknown campaign gets the same answer.
    let unknown = h.add_pc(Uuid::new_v4(), "Ysolde", json!(3)).await;
    applier.drain().await.unwrap();
    assert_eq!(
        h.result(unknown.command_id).await.violations,
        ["campaign-active"]
    );
    h.assert_no_orphan().await;
    h.drop_db().await;
}

#[tokio::test]
async fn pc_level_range_rejects_at_enqueue_without_a_version() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let version = h.version().await;

    for ok in [json!(1), json!(20)] {
        h.pc(&mut applier, c, &format!("Pj {ok}"), ok.as_i64().unwrap())
            .await;
    }
    let version_after_ok = h.version().await;
    assert_eq!(version_after_ok, version + 2);

    for (i, bad) in [
        json!(0),
        json!(21),
        json!(-1),
        json!(3.5),
        json!("5"),
        json!(5.0),
    ]
    .into_iter()
    .enumerate()
    {
        let s = h.add_pc(c, &format!("Refus {i}"), bad.clone()).await;
        // Already settled: the payload alone shows it.
        let Lookup::Settled { result } = &s.lookup else {
            panic!("{bad} was queued");
        };
        assert_eq!(result.status, State::Rejected, "{bad}");
        assert_eq!(result.violations, ["pc-level-range"], "{bad}");
        assert_eq!(result.data_version, None);
        // The result endpoint returns it too.
        assert_eq!(h.result(s.command_id).await, *result);
    }
    applier.drain().await.unwrap();
    assert_eq!(h.version().await, version_after_ok);
    let refused: i64 = sqlx::query_scalar("SELECT count(*) FROM pj WHERE nom LIKE 'Refus%'")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(refused, 0);
    h.drop_db().await;
}

#[tokio::test]
async fn every_violation_is_reported_in_declaration_order() {
    let h = Harness::new().await;
    let s = h
        .submit(json!({
            "dataCapability": "test.ajouterPj@1",
            "payload": { "campagneId": Uuid::new_v4().to_string(), "class": "Barde", "level": 0 },
            "idempotencyKey": "k1",
        }))
        .await;
    assert_eq!(
        s.lookup.result().unwrap().violations,
        ["pc-name-required", "pc-level-range"]
    );
    let s = h
        .submit(json!({
            "dataCapability": "test.ajouterPj@1",
            "payload": { "campagneId": Uuid::new_v4().to_string(), "name": "Y", "class": " ", "level": 2 },
            "idempotencyKey": "k2",
        }))
        .await;
    assert_eq!(s.lookup.result().unwrap().violations, ["pc-class-required"]);
    h.drop_db().await;
}

#[tokio::test]
async fn campaign_name_required_and_campaign_name_length_at_both_checks() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    for (name, id) in [
        (json!(""), "campaign-name-required"),
        (json!("\u{00A0} "), "campaign-name-required"),
        (json!("x".repeat(101)), "campaign-name-length"),
    ] {
        let s = h
            .submit(json!({
                "dataCapability": CREER_CAMPAGNE,
                "payload": { "name": name },
                "idempotencyKey": Uuid::new_v4().to_string(),
            }))
            .await;
        assert_eq!(s.lookup.result().unwrap().violations, [id]);
    }
    let c = h.campaign(&mut applier, &"é".repeat(100)).await;
    let v = h.version().await;
    let rename = h
        .submit(json!({
            "dataCapability": RENOMMER_CAMPAGNE,
            "target": { "id": c.to_string() },
            "payload": { "name": "   " },
            "basedOn": { "version": v },
        }))
        .await;
    assert_eq!(
        rename.lookup.result().unwrap().violations,
        ["campaign-name-required"]
    );
    let campaigns: i64 = sqlx::query_scalar("SELECT count(*) FROM campagne")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(campaigns, 1);
    h.drop_db().await;
}

#[tokio::test]
async fn a_rename_of_an_archived_campaign_is_rejected_campaign_active() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    h.archive_campaign(c).await;
    applier.drain().await.unwrap();
    let v = h.version().await;
    let rename = h
        .submit(json!({
            "dataCapability": RENOMMER_CAMPAGNE,
            "target": { "id": c.to_string() },
            "payload": { "name": "Autre" },
            "basedOn": { "version": v },
        }))
        .await;
    applier.drain().await.unwrap();
    assert_eq!(
        h.result(rename.command_id).await.violations,
        ["campaign-active"]
    );
    h.drop_db().await;
}

#[tokio::test]
async fn an_edit_of_an_unknown_pc_is_rejected_pc_active_and_changes_no_row() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let before = h.rows().await;
    let version = h.version().await;

    let unknown = h.set_level(Uuid::new_v4(), 4).await;
    let unknown_archive = h.archive_pc(Uuid::new_v4()).await;
    // A rename onto a taken name, on a real row, is refused whole.
    let other = h.pc(&mut applier, c, "Brume", 3).await;
    let before_rename = h.rows().await;
    let taken = h
        .edit_pc(
            other,
            json!({ "name": "ysolde", "level": 9 }),
            h.version().await,
        )
        .await;
    applier.drain().await.unwrap();
    for (s, id) in [
        (&unknown, "pc-active"),
        (&unknown_archive, "pc-active"),
        (&taken, "pc-name-unique-in-campaign"),
    ] {
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Rejected);
        assert_eq!(r.violations, [id]);
    }
    assert_eq!(h.rows().await, before_rename);
    assert_ne!(before, before_rename);
    assert_eq!(h.version().await, version + 1);
    assert_eq!(id_of(&taken), other);
    assert_eq!(h.pc_row(pc).await.0, "Ysolde");
    h.drop_db().await;
}
