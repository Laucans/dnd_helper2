//! The three mutating DataCapabilities the server ships — `modifierPJ`,
//! `archiverPJ`, `archiverCampagne` — driven through the engine with their
//! real manifests, on a manual clock. Campaigns and PCs are seeded with the
//! test-only inserts of `common::test_commands`; the data is invented.

mod common;

use std::time::Duration;

use campagne_serveur::startup;
use chrono::{DateTime, Utc};
use common::contracts::{J, assert_valid};
use common::test_commands::{self, Harness};
use dataguard::{Aggregates, Clock, GM_IDENTITY, Lookup, State, SubmitError, Submitted};
use serde_json::{Value, json};
use uuid::Uuid;

const DAY: Duration = Duration::from_secs(24 * 3600);

/// The test inserts, for seeding, and the three real mutations.
async fn harness() -> Harness {
    let aggregates = Aggregates::embedded().unwrap();
    let mut r = test_commands::registry();
    for cap in [
        modifier_pj::capability(),
        archiver_pj::capability(),
        archiver_campagne::capability(),
    ] {
        r.register(&aggregates, cap.unwrap()).unwrap();
    }
    Harness::with_registry(r).await
}

fn key() -> String {
    Uuid::new_v4().to_string()
}

async fn try_submit(h: &Harness, s: Value) -> Result<Submitted, SubmitError> {
    h.engine.submit(test_commands::sub(s)).await
}

/// A `modifierPJ` built on `based_on`.
async fn edit(h: &Harness, pc: Uuid, payload: Value, based_on: i64) -> Submitted {
    h.submit(json!({
        "dataCapability": modifier_pj::KEY,
        "target": { "id": pc },
        "payload": payload,
        "basedOn": { "version": based_on },
        "idempotencyKey": key(),
    }))
    .await
}

/// A `modifierPJ` built on the current version: never stale.
async fn edit_now(h: &Harness, pc: Uuid, payload: Value) -> Submitted {
    let v = h.version().await;
    edit(h, pc, payload, v).await
}

async fn archiver_pj(h: &Harness, pc: Uuid) -> Submitted {
    h.submit(json!({
        "dataCapability": archiver_pj::KEY,
        "target": { "id": pc },
        "idempotencyKey": key(),
    }))
    .await
}

async fn archiver_campagne(h: &Harness, campaign: Uuid) -> Submitted {
    h.submit(json!({
        "dataCapability": archiver_campagne::KEY,
        "target": { "id": campaign },
        "payload": {},
        "idempotencyKey": key(),
    }))
    .await
}

/// `campagneId`, `nom`, `classe`, `niveau`, `archiveLe`.
async fn pc_full(h: &Harness, pc: Uuid) -> (Uuid, String, String, i32, Option<DateTime<Utc>>) {
    sqlx::query_as(
        "SELECT \"campagneId\", nom, classe, niveau, \"archiveLe\" FROM pj WHERE id = $1",
    )
    .bind(pc)
    .fetch_one(&h.db.pool)
    .await
    .unwrap()
}

async fn counts(h: &Harness) -> (i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM campagne), (SELECT count(*) FROM pj)")
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

fn ysolde(level: Value) -> Value {
    json!({ "nom": "Ysolde", "classe": "Barde", "niveau": level })
}

/// The registry the server ships. `Registry` has no length, so "only these"
/// is checked on the commands that must never ship: the test ones.
#[test]
fn the_server_registers_the_three_mutations_and_no_test_command() {
    let r = startup::registry(&Aggregates::embedded().unwrap()).unwrap();
    for key in [modifier_pj::KEY, archiver_pj::KEY, archiver_campagne::KEY] {
        assert!(r.get(key).is_some(), "{key}");
    }
    for key in [
        test_commands::CREER_CAMPAGNE,
        test_commands::RENOMMER_CAMPAGNE,
        test_commands::ARCHIVER_CAMPAGNE,
        test_commands::AJOUTER_PJ,
        test_commands::MODIFIER_PJ,
        test_commands::REGLER_NIVEAU,
        test_commands::ARCHIVER_PJ,
    ] {
        assert!(r.get(key).is_none(), "{key}");
    }
}

/// AC "Edit PC, happy path".
#[tokio::test]
async fn an_edit_applies_trimmed_nfc_values_under_one_new_version() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;

    // NFD in, NFC out: `e` + U+0301 is stored as `é`.
    let s = edit(
        &h,
        pc,
        json!({ "nom": "  Ysolde\u{301}e ", "classe": " Barde ", "niveau": 20 }),
        v,
    )
    .await;
    assert_eq!(applier.drain().await.unwrap(), 1);
    let r = h.result(s.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert!(r.violations.is_empty());
    assert_eq!(r.data_version, Some(v + 1));
    assert_eq!(h.version().await, v + 1);
    assert_eq!(
        pc_full(&h, pc).await,
        (c, "Ysold\u{e9}e".into(), "Barde".into(), 20, None)
    );

    // The bounds: level 1, a 100-character name and a 50-character class.
    let name = "N".repeat(100);
    let class = "C".repeat(50);
    let s = edit_now(&h, pc, json!({ "nom": name, "classe": class, "niveau": 1 })).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(s.command_id).await.status, State::Applied);
    assert_eq!(pc_full(&h, pc).await, (c, name, class, 1, None));

    // Re-sending the current values is valid.
    let s = edit_now(
        &h,
        pc,
        json!({ "nom": "Ysolde", "classe": "Barde", "niveau": 7 }),
    )
    .await;
    applier.drain().await.unwrap();
    let s2 = edit_now(
        &h,
        pc,
        json!({ "nom": "Ysolde", "classe": "Barde", "niveau": 7 }),
    )
    .await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(s.command_id).await.status, State::Applied);
    assert_eq!(h.result(s2.command_id).await.status, State::Applied);
    h.drop_db().await;
}

/// AC "Edit PC, rejections": what the payload alone shows, refused at once.
#[tokio::test]
async fn payload_violations_reject_at_once_with_their_ids() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let rows = h.rows().await;
    let version = h.version().await;

    let with = |key: &str, value: Value| {
        let mut p = ysolde(json!(4));
        p[key] = value;
        p
    };
    let without = |key: &str| {
        let mut p = ysolde(json!(4));
        p.as_object_mut().unwrap().remove(key);
        p
    };
    let cases: Vec<(Value, Vec<&str>)> = vec![
        (with("nom", json!("")), vec!["pc-name-required"]),
        (with("nom", json!("   ")), vec!["pc-name-required"]),
        (
            with("nom", json!("N".repeat(101))),
            vec!["pc-name-required"],
        ),
        (with("nom", json!(null)), vec!["pc-name-required"]),
        (without("nom"), vec!["pc-name-required"]),
        (with("classe", json!("")), vec!["pc-class-required"]),
        (with("classe", json!(" \t ")), vec!["pc-class-required"]),
        (
            with("classe", json!("C".repeat(51))),
            vec!["pc-class-required"],
        ),
        (without("classe"), vec!["pc-class-required"]),
        (with("niveau", json!(0)), vec!["pc-level-range"]),
        (with("niveau", json!(21)), vec!["pc-level-range"]),
        (with("niveau", json!(-1)), vec!["pc-level-range"]),
        (with("niveau", json!(5.0)), vec!["pc-level-range"]),
        (with("niveau", json!(4.5)), vec!["pc-level-range"]),
        (with("niveau", json!("5")), vec!["pc-level-range"]),
        (with("niveau", json!(null)), vec!["pc-level-range"]),
        (without("niveau"), vec!["pc-level-range"]),
        // Several at once, in the aggregate's order; never only the first.
        (
            json!({ "nom": "", "classe": "Barde", "niveau": 0 }),
            vec!["pc-name-required", "pc-level-range"],
        ),
        (
            json!({}),
            vec!["pc-name-required", "pc-class-required", "pc-level-range"],
        ),
    ];
    for (payload, violations) in cases {
        let s = edit_now(&h, pc, payload.clone()).await;
        let r = s.lookup.result().expect("refused at once").clone();
        assert_eq!(r.status, State::Rejected, "{payload}");
        assert_eq!(r.violations, violations, "{payload}");
        assert_eq!(r.data_version, None, "{payload}");
        applier.drain().await.unwrap();
        assert_eq!(h.rows().await, rows, "{payload}");
        assert_eq!(h.version().await, version, "{payload}");
    }
    h.drop_db().await;
}

/// AC "Edit PC, rejections": name uniqueness, decided on the real state.
#[tokio::test]
async fn pc_name_unique_in_campaign_at_application() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let other = h.campaign(&mut applier, "Le Val").await;
    let a = h.pc(&mut applier, c, "Ysolde", 3).await;
    let b = h.pc(&mut applier, c, "Brume", 4).await;
    let gone = h.pc(&mut applier, c, "Disparu", 2).await;
    archiver_pj(&h, gone).await;
    applier.drain().await.unwrap();
    h.pc(&mut applier, other, "Sorcha", 6).await;

    // Another active PC's name, case-insensitively after trim.
    let rows = h.rows().await;
    let version = h.version().await;
    let s = edit_now(
        &h,
        b,
        json!({ "nom": " ysolde ", "classe": "Barde", "niveau": 4 }),
    )
    .await;
    applier.drain().await.unwrap();
    let r = h.result(s.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert_eq!(r.violations, ["pc-name-unique-in-campaign"]);
    assert_eq!(r.data_version, None);
    assert_eq!(h.rows().await, rows);
    assert_eq!(h.version().await, version);

    // Its own name in another case, an archived PC's name, another
    // campaign's PC's name: none is a conflict.
    for (pc, name) in [(a, "YSOLDE"), (b, "Disparu"), (a, "Sorcha")] {
        let s = edit_now(
            &h,
            pc,
            json!({ "nom": name, "classe": "Barde", "niveau": 4 }),
        )
        .await;
        applier.drain().await.unwrap();
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Applied, "{name}: {:?}", r.violations);
        assert_eq!(pc_full(&h, pc).await.1, name);
    }
    h.drop_db().await;
}

/// AC "Edit PC, rejections": an archived, unknown or cascaded PC.
#[tokio::test]
async fn an_edit_of_an_archived_unknown_or_cascaded_pc_is_rejected_pc_active() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let archived = h.pc(&mut applier, c, "Disparu", 2).await;
    archiver_pj(&h, archived).await;
    let closed = h.campaign(&mut applier, "La Fin").await;
    let cascaded = h.pc(&mut applier, closed, "Corvin", 5).await;
    archiver_campagne(&h, closed).await;
    applier.drain().await.unwrap();
    assert!(pc_full(&h, cascaded).await.4.is_some(), "the cascade ran");

    let rows = h.rows().await;
    let version = h.version().await;
    for (pc, exact) in [(archived, true), (Uuid::new_v4(), true), (cascaded, false)] {
        let s = edit_now(&h, pc, ysolde(json!(4))).await;
        applier.drain().await.unwrap();
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Rejected);
        assert_eq!(r.data_version, None);
        if exact {
            assert_eq!(r.violations, ["pc-active"]);
        } else {
            // The engine names the archived campaign too (`campaign-active`).
            assert!(r.violations.iter().any(|v| v == "pc-active"), "{r:?}");
        }
        assert_eq!(h.rows().await, rows);
        assert_eq!(h.version().await, version);
    }
    h.drop_db().await;
}

/// AC "Concurrent edits": the second waits, applies on confirm only.
#[tokio::test]
async fn a_second_edit_on_the_same_version_awaits_then_applies_on_confirm() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    let rows = h.rows().await;

    let first = edit(&h, pc, ysolde(json!(4)), v).await;
    let second = edit(&h, pc, ysolde(json!(5)), v).await;
    assert_eq!(first.lookup.state(), State::Queued);
    let Lookup::Pending { entry, .. } = &second.lookup else {
        panic!("not rejected for concurrency: {:?}", second.lookup);
    };
    assert_eq!(entry.state, State::AwaitingConfirmation);
    // In the aggregate's field names.
    assert_eq!(
        entry.your_value,
        Some(json!({ "name": "Ysolde", "class": "Barde", "level": 5 }))
    );
    assert_eq!(h.rows().await, rows, "nothing changes before the applier");

    applier.drain().await.unwrap();
    let r1 = h.result(first.command_id).await;
    assert_eq!(r1.status, State::Applied);
    assert_eq!(r1.data_version, Some(v + 1));
    // Unconfirmed at the head, it parks; the PC holds the first's values.
    assert_eq!(h.state(second.command_id).await, State::Parked);
    assert_eq!(pc_full(&h, pc).await.3, 4);
    assert_eq!(h.version().await, v + 1);

    // A third on the same base parks too.
    let third = edit(&h, pc, ysolde(json!(6)), v).await;
    assert_eq!(third.lookup.state(), State::AwaitingConfirmation);
    applier.drain().await.unwrap();
    assert_eq!(h.state(third.command_id).await, State::Parked);

    let confirmed = h
        .engine
        .confirm(second.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(confirmed.state(), State::Confirmed);
    applier.drain().await.unwrap();
    let r2 = h.result(second.command_id).await;
    assert_eq!(r2.status, State::Applied);
    assert_eq!(r2.data_version, Some(v + 2));
    assert_eq!(
        pc_full(&h, pc).await.3,
        5,
        "the second overwrites the first"
    );
    // The third needs its own confirm.
    assert_eq!(h.state(third.command_id).await, State::Parked);
    h.drop_db().await;
}

/// AC "Concurrent edits": unconfirmed, dropped after 24 h.
#[tokio::test]
async fn an_unconfirmed_edit_expires_after_a_day_unchanged() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    edit(&h, pc, ysolde(json!(4)), v).await;
    let second = edit(&h, pc, ysolde(json!(5)), v).await;
    applier.drain().await.unwrap();
    assert_eq!(h.state(second.command_id).await, State::Parked);
    let rows = h.rows().await;
    let version = h.version().await;

    h.clock.advance(DAY - Duration::from_secs(1));
    applier.drain().await.unwrap();
    assert_eq!(h.state(second.command_id).await, State::Parked);
    h.clock.advance(Duration::from_secs(1));
    applier.drain().await.unwrap();
    let r = h.result(second.command_id).await;
    assert_eq!(r.status, State::Expired);
    assert_eq!(r.data_version, None);
    assert_eq!(h.rows().await, rows);
    assert_eq!(h.version().await, version);

    let late = h
        .engine
        .confirm(second.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(late.result(), Some(&r));
    applier.drain().await.unwrap();
    assert_eq!(h.rows().await, rows);
    h.drop_db().await;
}

/// SPEC rule 22.
#[tokio::test]
async fn a_parked_edit_cancelled_by_its_author_changes_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    edit(&h, pc, ysolde(json!(4)), v).await;
    let second = edit(&h, pc, ysolde(json!(5)), v).await;
    applier.drain().await.unwrap();
    assert_eq!(h.state(second.command_id).await, State::Parked);
    let rows = h.rows().await;
    let version = h.version().await;

    let out = h
        .engine
        .cancel(second.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(out.state(), State::Cancelled);
    applier.drain().await.unwrap();
    let r = h.result(second.command_id).await;
    assert_eq!(r.status, State::Cancelled);
    assert_eq!(r.data_version, None);
    assert_eq!(h.rows().await, rows);
    assert_eq!(h.version().await, version);
    h.drop_db().await;
}

/// AC "Edit queued before an archive": the application-time check decides,
/// across partitions too.
#[tokio::test]
async fn an_edit_queued_before_an_archive_and_applied_after_is_rejected_pc_active() {
    let h = harness().await;
    let mut applier = h.applier().await;

    // Archive of the PC itself.
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    edit(&h, pc, ysolde(json!(4)), v).await;
    let pending = edit(&h, pc, ysolde(json!(5)), v).await;
    applier.drain().await.unwrap();
    assert_eq!(h.state(pending.command_id).await, State::Parked);
    let archive = archiver_pj(&h, pc).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(archive.command_id).await.status, State::Applied);
    // The archive neither cancels nor expires it.
    assert_eq!(h.state(pending.command_id).await, State::Parked);
    let row = pc_full(&h, pc).await;
    let version = h.version().await;
    h.engine
        .confirm(pending.command_id, GM_IDENTITY)
        .await
        .unwrap();
    applier.drain().await.unwrap();
    let r = h.result(pending.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert_eq!(r.violations, ["pc-active"]);
    assert_eq!(r.data_version, None);
    assert_eq!(pc_full(&h, pc).await, row);
    assert_eq!(h.version().await, version);

    // Archive of its campaign, on another partition: the cascade archived it.
    let closed = h.campaign(&mut applier, "La Fin").await;
    let pc = h.pc(&mut applier, closed, "Corvin", 5).await;
    let v = h.version().await;
    edit(
        &h,
        pc,
        json!({ "nom": "Corvin", "classe": "Mage", "niveau": 6 }),
        v,
    )
    .await;
    let pending = edit(
        &h,
        pc,
        json!({ "nom": "Corvin", "classe": "Mage", "niveau": 7 }),
        v,
    )
    .await;
    applier.drain().await.unwrap();
    assert_eq!(h.state(pending.command_id).await, State::Parked);
    let archive = archiver_campagne(&h, closed).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(archive.command_id).await.status, State::Applied);
    assert_eq!(h.state(pending.command_id).await, State::Parked);
    let row = pc_full(&h, pc).await;
    assert!(row.4.is_some());
    h.engine
        .confirm(pending.command_id, GM_IDENTITY)
        .await
        .unwrap();
    applier.drain().await.unwrap();
    let r = h.result(pending.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert!(r.violations.iter().any(|v| v == "pc-active"), "{r:?}");
    assert_eq!(r.data_version, None);
    assert_eq!(pc_full(&h, pc).await, row);
    h.drop_db().await;
}

/// AC "Archive PC".
#[tokio::test]
async fn archiving_a_pc_keeps_the_row_and_a_second_archive_is_a_no_op() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let a = h.pc(&mut applier, c, "Ysolde", 3).await;
    let b = h.pc(&mut applier, c, "Brume", 4).await;
    let before = counts(&h).await;
    let sibling = pc_full(&h, b).await;
    let version = h.version().await;

    h.clock.advance(Duration::from_secs(3600));
    let at = h.clock.now();
    let s = archiver_pj(&h, a).await;
    assert_eq!(applier.drain().await.unwrap(), 1);
    let r = h.result(s.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert_eq!(r.data_version, Some(version + 1));
    assert_eq!(pc_full(&h, a).await.4, Some(at));
    assert_eq!(counts(&h).await, before, "nothing removed");
    assert_eq!(pc_full(&h, b).await, sibling);
    assert_eq!(h.campaign_archived_at(c).await, None);

    // The last active PC: the campaign stays active.
    archiver_pj(&h, b).await;
    applier.drain().await.unwrap();
    assert!(pc_full(&h, b).await.4.is_some());
    assert_eq!(h.campaign_archived_at(c).await, None);

    // Again, later: applied, nothing written, no new version.
    h.clock.advance(Duration::from_secs(3600));
    let rows = h.rows().await;
    let version = h.version().await;
    let again = archiver_pj(&h, a).await;
    applier.drain().await.unwrap();
    let r = h.result(again.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert!(r.violations.is_empty());
    assert_eq!(r.data_version, Some(version));
    assert_eq!(h.version().await, version);
    assert_eq!(pc_full(&h, a).await.4, Some(at), "tombstone kept");
    assert_eq!(h.rows().await, rows);

    // An unknown PC.
    let unknown = archiver_pj(&h, Uuid::new_v4()).await;
    applier.drain().await.unwrap();
    let r = h.result(unknown.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert_eq!(r.violations, ["pc-active"]);
    assert_eq!(r.data_version, None);
    assert_eq!(h.rows().await, rows);
    h.drop_db().await;
}

/// AC "Archive campaign".
#[tokio::test]
async fn archiving_a_campaign_cascades_to_its_active_pcs_under_one_version() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let other = h.campaign(&mut applier, "Le Val").await;
    let active = [
        h.pc(&mut applier, c, "Ysolde", 3).await,
        h.pc(&mut applier, c, "Brume", 4).await,
        h.pc(&mut applier, c, "Corvin", 5).await,
    ];
    let gone = h.pc(&mut applier, c, "Disparu", 2).await;
    archiver_pj(&h, gone).await;
    applier.drain().await.unwrap();
    let gone_at = pc_full(&h, gone).await.4.unwrap();
    let elsewhere = h.pc(&mut applier, other, "Sorcha", 6).await;
    let before = counts(&h).await;
    let version = h.version().await;

    h.clock.advance(Duration::from_secs(3600));
    let at = h.clock.now();
    let archive = archiver_campagne(&h, c).await;
    assert_eq!(applier.drain().await.unwrap(), 1);
    let r = h.result(archive.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert_eq!(r.data_version, Some(version + 1));
    assert_eq!(h.version().await, version + 1);
    assert_eq!(h.campaign_archived_at(c).await, Some(at));
    for pc in active {
        assert_eq!(pc_full(&h, pc).await.4, Some(at));
    }
    assert_eq!(pc_full(&h, gone).await.4, Some(gone_at), "tombstone kept");
    assert_eq!(pc_full(&h, elsewhere).await.4, None);
    assert_eq!(h.campaign_archived_at(other).await, None);
    assert_eq!(counts(&h).await, before, "nothing removed");
    h.assert_no_orphan().await;

    let plan: sqlx::types::Json<Value> =
        sqlx::query_scalar("SELECT impact_plan FROM dataguard_queue WHERE command = $1")
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

    // Again, later: a no-op that does not re-run the cascade.
    h.clock.advance(Duration::from_secs(3600));
    let rows = h.rows().await;
    let version = h.version().await;
    let again = archiver_campagne(&h, c).await;
    applier.drain().await.unwrap();
    let r = h.result(again.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert!(r.violations.is_empty());
    assert_eq!(r.data_version, Some(version));
    assert_eq!(h.version().await, version);
    assert_eq!(h.campaign_archived_at(c).await, Some(at));
    assert_eq!(h.rows().await, rows);

    // A campaign without a PC.
    let empty = h.campaign(&mut applier, "Vide").await;
    let s = archiver_campagne(&h, empty).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(s.command_id).await.status, State::Applied);
    assert!(h.campaign_archived_at(empty).await.is_some());

    // An unknown campaign.
    let rows = h.rows().await;
    let unknown = archiver_campagne(&h, Uuid::new_v4()).await;
    applier.drain().await.unwrap();
    let r = h.result(unknown.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert_eq!(r.violations, ["campaign-active"]);
    assert_eq!(r.data_version, None);
    assert_eq!(h.rows().await, rows);
    h.assert_no_orphan().await;
    h.drop_db().await;
}

/// AC "Cascade atomicity".
#[tokio::test]
async fn a_failing_cascade_changes_nothing_and_is_retried() {
    let h = harness().await;
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

    let archive = archiver_campagne(&h, c).await;
    assert!(applier.apply_next().await.is_err());
    assert_eq!(h.rows().await, rows, "neither the campaign nor any PC");
    assert_eq!(h.campaign_archived_at(c).await, None);
    assert_eq!(h.state(archive.command_id).await, State::Queued);
    assert_eq!(h.version().await, version);

    sqlx::raw_sql("DROP TRIGGER panne ON pj")
        .execute(&h.db.pool)
        .await
        .unwrap();
    // Set aside until the backoff is over on the engine clock.
    assert_eq!(applier.drain().await.unwrap(), 0);
    assert_eq!(h.state(archive.command_id).await, State::Queued);
    h.clock.advance(Duration::from_secs(2));
    assert_eq!(applier.drain().await.unwrap(), 1);
    let r = h.result(archive.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert_eq!(r.data_version, Some(version + 1));
    let at = h.campaign_archived_at(c).await;
    assert!(at.is_some());
    for pc in pcs {
        assert_eq!(pc_full(&h, pc).await.4, at);
    }
    h.drop_db().await;
}

/// AC "Idempotency", and what is refused before any queue entry exists.
#[tokio::test]
async fn idempotency_on_the_three_commands() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let other_pc = h.pc(&mut applier, c, "Brume", 4).await;
    let other_c = h.campaign(&mut applier, "Le Val").await;
    let v = h.version().await;

    let edit_cmd = |payload: Value, k: &str| {
        json!({
            "dataCapability": modifier_pj::KEY,
            "target": { "id": pc },
            "payload": payload,
            "basedOn": { "version": v },
            "idempotencyKey": k,
        })
    };
    let archive_cmd = |cap: &str, target: Uuid, k: &str| json!({ "dataCapability": cap, "target": { "id": target }, "idempotencyKey": k });
    let k1 = key();
    let k2 = key();
    let k3 = key();
    // The campaign is archived last: its cascade would archive the PC first.
    let commands = [
        (
            edit_cmd(ysolde(json!(4)), &k1),
            edit_cmd(ysolde(json!(5)), &k1),
        ),
        (
            archive_cmd(archiver_pj::KEY, pc, &k2),
            archive_cmd(archiver_pj::KEY, other_pc, &k2),
        ),
        (
            archive_cmd(archiver_campagne::KEY, c, &k3),
            archive_cmd(archiver_campagne::KEY, other_c, &k3),
        ),
    ];
    for (command, conflicting) in commands {
        let queued = h.queue_rows().await;
        let version = h.version().await;
        let first = h.submit(command.clone()).await;
        assert!(!first.replayed);
        let pending = h.submit(command.clone()).await;
        assert!(pending.replayed, "{command}");
        assert_eq!(pending.command_id, first.command_id);
        assert_eq!(h.queue_rows().await, queued + 1);

        applier.drain().await.unwrap();
        let r = h.result(first.command_id).await;
        assert_eq!(r.status, State::Applied, "{command}");
        assert_eq!(h.version().await, version + 1);
        let rows = h.rows().await;

        let settled = h.submit(command.clone()).await;
        assert!(settled.replayed);
        assert_eq!(settled.command_id, first.command_id);
        assert_eq!(settled.lookup.result(), Some(&r));
        applier.drain().await.unwrap();
        assert_eq!(h.version().await, version + 1, "no second version");
        assert_eq!(h.rows().await, rows, "no second change");
        assert_eq!(h.queue_rows().await, queued + 1);

        assert_eq!(
            try_submit(&h, conflicting).await.unwrap_err(),
            SubmitError::IdempotencyKeyConflict
        );
        assert_eq!(h.rows().await, rows);
        assert_eq!(h.queue_rows().await, queued + 1);
    }

    // Refused before enqueue: no entry, no version.
    let target = h.pc(&mut applier, other_c, "Sorcha", 6).await;
    let queued = h.queue_rows().await;
    let rows = h.rows().await;
    let version = h.version().await;
    let mut refusals = Vec::new();
    for cap in [modifier_pj::KEY, archiver_pj::KEY] {
        let payload = if cap == modifier_pj::KEY {
            ysolde(json!(4))
        } else {
            json!({})
        };
        let base = json!({
            "dataCapability": cap,
            "target": { "id": target },
            "payload": payload,
            "basedOn": { "version": version },
        });
        let mut blank = base.clone();
        blank["idempotencyKey"] = json!("  ");
        refusals.push((base, SubmitError::IdempotencyKeyRequired));
        refusals.push((blank, SubmitError::IdempotencyKeyRequired));
    }
    refusals.push((
        json!({
            "dataCapability": archiver_campagne::KEY,
            "target": { "id": other_c },
        }),
        SubmitError::IdempotencyKeyRequired,
    ));
    refusals.push((
        json!({
            "dataCapability": modifier_pj::KEY,
            "target": { "id": target },
            "payload": ysolde(json!(4)),
            "idempotencyKey": key(),
        }),
        SubmitError::Malformed("based-on-required"),
    ));
    refusals.push((
        json!({
            "dataCapability": modifier_pj::KEY,
            "target": { "id": target },
            "payload": { "nom": "Sorcha", "classe": "Mage", "niveau": 6, "pjId": target },
            "basedOn": { "version": version },
            "idempotencyKey": key(),
        }),
        SubmitError::Malformed("unknown-field"),
    ));
    for cap in [archiver_pj::KEY, archiver_campagne::KEY] {
        let id = if cap == archiver_pj::KEY {
            target
        } else {
            other_c
        };
        refusals.push((
            json!({
                "dataCapability": cap,
                "target": { "id": id },
                "payload": { "archivedAt": null },
                "idempotencyKey": key(),
            }),
            SubmitError::Malformed("unknown-field"),
        ));
    }
    for (submission, error) in refusals {
        assert_eq!(
            try_submit(&h, submission.clone()).await.unwrap_err(),
            error,
            "{submission}"
        );
    }
    applier.drain().await.unwrap();
    assert_eq!(h.queue_rows().await, queued);
    assert_eq!(h.rows().await, rows);
    assert_eq!(h.version().await, version);
    h.drop_db().await;
}
