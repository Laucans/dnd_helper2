//! `campagne.ajouterPj@1`, the fast-path add of a PC, driven through the
//! engine with its real manifest, on a manual clock. Campaigns and the PCs
//! that seed a scenario come from the test-only inserts of
//! `common::test_commands`; every name and campaign here is invented.

mod common;

use campagne_serveur::startup;
use chrono::{DateTime, Utc};
use common::test_commands::{self, Harness};
use dataguard::{Aggregates, State, SubmitError, Submitted};
use serde_json::{Value, json};
use uuid::Uuid;

/// The test inserts, for seeding, and the real add.
async fn harness() -> Harness {
    let aggregates = Aggregates::embedded().unwrap();
    let mut r = test_commands::registry();
    r.register(&aggregates, ajouter_pj::capability().unwrap())
        .unwrap();
    Harness::with_registry(r).await
}

fn key() -> String {
    Uuid::new_v4().to_string()
}

/// A payload the DataGuard accepts, for `campaign`.
fn valid(campaign: Uuid) -> Value {
    json!({
        "campagneId": campaign.to_string(),
        "nom": "Ysolde",
        "classe": "Barde",
        "niveau": 5,
    })
}

/// `valid` with `field` set to `value`, or removed when `value` is `None`.
fn with(campaign: Uuid, field: &str, value: Option<Value>) -> Value {
    let mut p = valid(campaign);
    let object = p.as_object_mut().unwrap();
    match value {
        Some(v) => object.insert(field.into(), v),
        None => object.remove(field),
    };
    p
}

/// An `ajouterPj` with a fresh key; the error is what is refused before the
/// queue.
async fn try_add(h: &Harness, payload: Value, key: Option<&str>) -> Result<Submitted, SubmitError> {
    let mut s = json!({ "dataCapability": ajouter_pj::KEY, "payload": payload });
    if let Some(k) = key {
        s["idempotencyKey"] = json!(k);
    }
    h.engine.submit(test_commands::sub(s)).await
}

async fn add_payload(h: &Harness, payload: Value) -> Submitted {
    try_add(h, payload, Some(&key())).await.unwrap()
}

async fn add(h: &Harness, campaign: Uuid, nom: &str, classe: &str, niveau: Value) -> Submitted {
    add_payload(
        h,
        json!({
            "campagneId": campaign.to_string(),
            "nom": nom,
            "classe": classe,
            "niveau": niveau,
        }),
    )
    .await
}

/// Rows of `pj` in `campaign`, archived ones included.
async fn pcs_of(h: &Harness, campaign: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pj WHERE \"campagneId\" = $1")
        .bind(campaign)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
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

async fn campaign_row(h: &Harness, campaign: Uuid) -> String {
    sqlx::query_scalar("SELECT c::text FROM campagne c WHERE id = $1")
        .bind(campaign)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

async fn enqueue_seq(h: &Harness, s: &Submitted) -> i64 {
    sqlx::query_scalar("SELECT enqueue_seq FROM dataguard_queue WHERE command = $1")
        .bind(s.command_id.0)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

/// Queue entries, `dataVersion`, PC rows: what a refusal must leave alone.
async fn footprint(h: &Harness) -> (i64, i64, i64) {
    let pcs = sqlx::query_scalar("SELECT count(*) FROM pj")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    (h.queue_rows().await, h.version().await, pcs)
}

/// `s` ended `rejected` with exactly `violations` and no version.
async fn assert_rejected(h: &Harness, s: &Submitted, violations: &[&str]) {
    let r = h.result(s.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert_eq!(r.violations, violations);
    assert_eq!(r.data_version, None);
}

/// The registry the server ships has the add.
#[test]
fn the_server_registers_ajouter_pj() {
    let r = startup::registry(&Aggregates::embedded().unwrap()).unwrap();
    assert!(r.get(ajouter_pj::KEY).is_some());
}

/// AC "A valid add".
#[tokio::test]
async fn a_valid_add_applies_with_a_version_and_one_active_row() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let other = h.campaign(&mut applier, "Le Val").await;
    let campaign_before = campaign_row(&h, c).await;
    let other_before = campaign_row(&h, other).await;
    let v = h.version().await;

    // Trimmed (a no-break space included) and NFC, with the case kept.
    let s = add(&h, c, " Ysolde\u{00A0}", " Barde ", json!(5)).await;
    assert_eq!(s.partition, format!("PJ/{}", test_commands::id_of(&s)));
    assert!(!s.replayed);
    assert_eq!(applier.drain().await.unwrap(), 1);
    let r = h.result(s.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert!(r.violations.is_empty());
    assert_eq!(r.data_version, Some(v + 1));
    assert_eq!(h.version().await, v + 1);
    assert_eq!(
        pc_full(&h, test_commands::id_of(&s)).await,
        (c, "Ysolde".into(), "Barde".into(), 5, None)
    );

    let s = add(&h, c, "E\u{301}lan", "Magicien (Évocation)", json!(1)).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(s.command_id).await.status, State::Applied);
    assert_eq!(
        pc_full(&h, test_commands::id_of(&s)).await,
        (
            c,
            "\u{c9}lan".into(),
            "Magicien (Évocation)".into(),
            1,
            None
        )
    );

    // Neither campaign was touched, and nothing landed in the other one.
    assert_eq!(campaign_row(&h, c).await, campaign_before);
    assert_eq!(campaign_row(&h, other).await, other_before);
    assert_eq!(pcs_of(&h, c).await, 2);
    assert_eq!(pcs_of(&h, other).await, 0);
    h.drop_db().await;
}

/// Business rule 35: the bounds are accepted, in characters and not bytes.
#[tokio::test]
async fn boundaries_are_accepted() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    let cases = [
        // The first PC of an empty campaign, at level 1.
        ("A".to_string(), "B".to_string(), 1),
        ("é".repeat(100), "é".repeat(50), 20),
        ("Brune".to_string(), "Barbare".to_string(), 10),
        ("Aldric".to_string(), "Magicien (Évocation)".to_string(), 10),
        ("Zéphyr".to_string(), "Mot inconnu".to_string(), 10),
    ];
    for (nom, classe, niveau) in &cases {
        let s = add(&h, c, nom, classe, json!(niveau)).await;
        applier.drain().await.unwrap();
        assert_eq!(
            h.result(s.command_id).await.status,
            State::Applied,
            "{nom} / {classe} / {niveau}"
        );
        assert_eq!(
            pc_full(&h, test_commands::id_of(&s)).await,
            (c, nom.clone(), classe.clone(), *niveau, None)
        );
    }
    assert_eq!(pcs_of(&h, c).await, cases.len() as i64);
    h.drop_db().await;
}

/// AC "Each invalid field": the name.
#[tokio::test]
async fn pc_name_required_refuses_and_saves_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let before = footprint(&h).await;

    let long = "é".repeat(101);
    let cases = [
        None,
        Some(json!(null)),
        Some(json!(42)),
        Some(json!(["Ysolde"])),
        Some(json!("")),
        Some(json!("   ")),
        Some(json!("\u{00A0}\t")),
        Some(json!(long)),
    ];
    for nom in cases {
        let s = add_payload(&h, with(c, "nom", nom.clone())).await;
        applier.drain().await.unwrap();
        assert_rejected(&h, &s, &["pc-name-required"]).await;
        assert_eq!(h.version().await, before.1, "{nom:?}");
    }
    assert_eq!(pcs_of(&h, c).await, 0);
    assert_eq!(h.version().await, before.1);
    h.drop_db().await;
}

/// AC "Each invalid field": the class.
#[tokio::test]
async fn pc_class_required_refuses_and_saves_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let version = h.version().await;

    let cases = [
        None,
        Some(json!(null)),
        Some(json!(7)),
        Some(json!("")),
        Some(json!("\t")),
        Some(json!("é".repeat(51))),
    ];
    for classe in cases {
        let s = add_payload(&h, with(c, "classe", classe.clone())).await;
        applier.drain().await.unwrap();
        assert_rejected(&h, &s, &["pc-class-required"]).await;
        assert_eq!(h.version().await, version, "{classe:?}");
    }
    assert_eq!(pcs_of(&h, c).await, 0);
    h.drop_db().await;
}

/// AC "Each invalid field": the level. A refused level never panics.
#[tokio::test]
async fn pc_level_range_refuses_and_saves_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let version = h.version().await;

    let cases = [
        None,
        Some(json!(null)),
        Some(json!(0)),
        Some(json!(21)),
        Some(json!(-1)),
        Some(json!(1.5)),
        Some(json!(5.0)),
        Some(json!(1e1)),
        Some(json!("5")),
        Some(json!(true)),
        Some(json!([5])),
        Some(json!(u64::MAX)),
        Some(json!(i64::MIN)),
        Some(json!(i64::from(i32::MAX) + 1)),
    ];
    for niveau in cases {
        let s = add_payload(&h, with(c, "niveau", niveau.clone())).await;
        applier.drain().await.unwrap();
        assert_rejected(&h, &s, &["pc-level-range"]).await;
        assert_eq!(h.version().await, version, "{niveau:?}");
    }
    assert_eq!(pcs_of(&h, c).await, 0);
    h.drop_db().await;
}

/// AC "all their ids appear in `violations`", in the manifest's order. The
/// field checks come first and short-circuit the campaign's: they are
/// reported even when the campaign is bad, and it is not looked at then.
#[tokio::test]
async fn several_field_violations_are_all_reported_in_order() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    let s = add_payload(&h, with(c, "nom", None)).await;
    let s0 = {
        let mut p = with(c, "nom", None);
        p["niveau"] = json!(0);
        add_payload(&h, p).await
    };
    let all = add_payload(
        &h,
        json!({ "campagneId": c.to_string(), "nom": " ", "classe": "", "niveau": 21 }),
    )
    .await;
    let nothing = add_payload(&h, json!({ "campagneId": c.to_string() })).await;
    let unknown_campaign = add_payload(&h, with(Uuid::new_v4(), "nom", Some(json!("  ")))).await;
    applier.drain().await.unwrap();

    assert_rejected(&h, &s, &["pc-name-required"]).await;
    assert_rejected(&h, &s0, &["pc-name-required", "pc-level-range"]).await;
    assert_rejected(
        &h,
        &all,
        &["pc-name-required", "pc-class-required", "pc-level-range"],
    )
    .await;
    assert_rejected(
        &h,
        &nothing,
        &["pc-name-required", "pc-class-required", "pc-level-range"],
    )
    .await;
    // The field ids are enough: `campaign-active` is not evaluated beside them.
    assert_rejected(&h, &unknown_campaign, &["pc-name-required"]).await;
    assert_eq!(pcs_of(&h, c).await, 0);
    h.drop_db().await;
}

/// AC "An unknown or archived `campagneId`": the same answer, nothing saved,
/// and nothing that says which of the two it was.
#[tokio::test]
async fn campaign_active_unknown_and_archived_give_the_same_answer() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let archived = h.campaign(&mut applier, "Les Brumes").await;
    h.archive_campaign(archived).await;
    applier.drain().await.unwrap();
    let archived_row = campaign_row(&h, archived).await;
    assert!(h.campaign_archived_at(archived).await.is_some());
    let unknown = Uuid::new_v4();
    let before = footprint(&h).await;

    let a = add(&h, unknown, "Ysolde", "Barde", json!(5)).await;
    let b = add(&h, archived, "Ysolde", "Barde", json!(5)).await;
    assert_eq!(a.warnings, b.warnings);
    assert_eq!(a.warnings, ["campaign-active"]);
    applier.drain().await.unwrap();

    assert_rejected(&h, &a, &["campaign-active"]).await;
    assert_rejected(&h, &b, &["campaign-active"]).await;
    let (ra, rb) = (h.result(a.command_id).await, h.result(b.command_id).await);
    assert_eq!(
        (ra.status, ra.data_version, ra.violations, ra.review_id),
        (rb.status, rb.data_version, rb.violations, rb.review_id)
    );
    assert_eq!(pcs_of(&h, unknown).await, 0);
    assert_eq!(pcs_of(&h, archived).await, 0);
    assert_eq!(campaign_row(&h, archived).await, archived_row);
    assert_eq!(h.version().await, before.1);
    h.drop_db().await;
}

/// Business rule 25: the application-time check decides. An add queued
/// behind the archive of its campaign is refused when it applies.
#[tokio::test]
async fn campaign_active_when_the_campaign_is_archived_between_enqueue_and_apply() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let version = h.version().await;

    let archive = h.archive_campaign(c).await;
    let s = add(&h, c, "Ysolde", "Barde", json!(5)).await;
    assert_eq!(s.lookup.state(), State::Queued);
    applier.drain().await.unwrap();

    assert_eq!(h.result(archive.command_id).await.status, State::Applied);
    assert_rejected(&h, &s, &["campaign-active"]).await;
    assert_eq!(pcs_of(&h, c).await, 0);
    // The archive took the next number; the refused add took none.
    assert_eq!(h.version().await, version + 1);
    h.drop_db().await;
}

/// A campaign id that is no uuid names no campaign: it is a malformed
/// submission, before any queue entry.
#[tokio::test]
async fn a_missing_or_malformed_campagne_id_is_a_malformed_submission() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let before = footprint(&h).await;

    for id in [
        None,
        Some(json!(null)),
        Some(json!("pas-un-uuid")),
        Some(json!(7)),
    ] {
        assert_eq!(
            try_add(&h, with(c, "campagneId", id.clone()), Some(&key()))
                .await
                .map(|s| s.command_id),
            Err(SubmitError::Malformed("relation-id")),
            "{id:?}"
        );
    }
    assert_eq!(footprint(&h).await, before);
    h.drop_db().await;
}

/// AC "Two PCs in one campaign cannot share a name": the one lower in the
/// queue applies, the other is an invariant rejection, never a concurrency one.
#[tokio::test]
async fn two_concurrent_same_name_adds_first_in_queue_applies_second_is_rejected_pc_name_unique_in_campaign()
 {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let v = h.version().await;

    let (a, b) = tokio::join!(
        add(&h, c, "Gandalf", "Magicien", json!(10)),
        add(&h, c, " GANDALF ", "Barbare", json!(3)),
    );
    // Neither is refused for the other being in flight.
    assert_eq!(a.lookup.state(), State::Queued);
    assert_eq!(b.lookup.state(), State::Queued);
    let (first, second) = if enqueue_seq(&h, &a).await < enqueue_seq(&h, &b).await {
        (&a, &b)
    } else {
        (&b, &a)
    };
    assert_eq!(applier.drain().await.unwrap(), 2);

    let r = h.result(first.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert!(r.violations.is_empty());
    assert_eq!(r.data_version, Some(v + 1));
    assert_rejected(&h, second, &["pc-name-unique-in-campaign"]).await;
    assert_eq!(pcs_of(&h, c).await, 1);
    assert_eq!(h.version().await, v + 1);
    h.drop_db().await;
}

/// Business rule 26: a rejected first reserves nothing.
#[tokio::test]
async fn if_the_first_same_name_add_is_rejected_the_second_applies() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    let first = add(&h, c, "Gandalf", "Magicien", json!(0)).await;
    let second = add(&h, c, "gandalf", "Magicien", json!(10)).await;
    applier.drain().await.unwrap();

    assert_rejected(&h, &first, &["pc-level-range"]).await;
    assert_eq!(h.result(second.command_id).await.status, State::Applied);
    assert_eq!(pcs_of(&h, c).await, 1);
    h.drop_db().await;
}

/// AC "The same name is accepted in another campaign", after an archive, and
/// when it only looks alike.
#[tokio::test]
async fn the_same_name_is_accepted_in_another_campaign_and_after_an_archive() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let a = h.campaign(&mut applier, "Les Brumes").await;
    let b = h.campaign(&mut applier, "Le Val").await;

    // Concurrently, in two campaigns.
    let (in_a, in_b) = tokio::join!(
        add(&h, a, "Sorcha", "Druide", json!(4)),
        add(&h, b, "SORCHA", "Druide", json!(4)),
    );
    applier.drain().await.unwrap();
    assert_eq!(h.result(in_a.command_id).await.status, State::Applied);
    assert_eq!(h.result(in_b.command_id).await.status, State::Applied);

    // An archived PC does not hold its name.
    h.archive_pc(test_commands::id_of(&in_a)).await;
    applier.drain().await.unwrap();
    let again = add(&h, a, "sorcha", "Druide", json!(4)).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(again.command_id).await.status, State::Applied);

    // A diacritic or an inner space makes another name.
    for pair in [["Elan", "Élan"], ["Le Gris", "LeGris"]] {
        for nom in pair {
            let s = add(&h, a, nom, "Voleur", json!(2)).await;
            applier.drain().await.unwrap();
            assert_eq!(h.result(s.command_id).await.status, State::Applied, "{nom}");
        }
    }
    assert_eq!(pcs_of(&h, b).await, 1);
    h.drop_db().await;
}

/// Business rule 24: a rejection consumes no number, so there is no gap.
#[tokio::test]
async fn a_rejection_consumes_no_version_and_the_next_add_takes_the_next() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let v = h.version().await;

    let one = add(&h, c, "Ysolde", "Barde", json!(5)).await;
    let refused = add(&h, c, "Brune", "Barde", json!(21)).await;
    let duplicate = add(&h, c, "ysolde", "Barde", json!(5)).await;
    let two = add(&h, c, "Aldric", "Barde", json!(5)).await;
    applier.drain().await.unwrap();

    assert_eq!(h.result(one.command_id).await.data_version, Some(v + 1));
    assert_rejected(&h, &refused, &["pc-level-range"]).await;
    assert_rejected(&h, &duplicate, &["pc-name-unique-in-campaign"]).await;
    assert_eq!(h.result(two.command_id).await.data_version, Some(v + 2));
    assert_eq!(h.version().await, v + 2);
    h.drop_db().await;
}

/// AC "Replaying a command with the same `idempotencyKey`".
#[tokio::test]
async fn idempotent_replay_returns_the_original_and_writes_no_second_row() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let v = h.version().await;

    let k = key();
    let first = try_add(&h, valid(c), Some(&k)).await.unwrap();
    applier.drain().await.unwrap();
    let original = h.result(first.command_id).await;
    assert_eq!(original.status, State::Applied);

    // Sent again, before and after the PC is archived: the same answer.
    for archived in [false, true] {
        if archived {
            h.archive_pc(test_commands::id_of(&first)).await;
            applier.drain().await.unwrap();
        }
        let version = h.version().await;
        let replay = try_add(&h, valid(c), Some(&k)).await.unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.command_id, first.command_id);
        assert_eq!(replay.partition, first.partition);
        assert_eq!(h.result(replay.command_id).await, original);
        assert_eq!(applier.drain().await.unwrap(), 0);
        assert_eq!(h.version().await, version);
        assert_eq!(pcs_of(&h, c).await, 1);
    }
    assert_eq!(original.data_version, Some(v + 1));

    // The same key with another payload is refused, and writes nothing.
    let rows = h.rows().await;
    assert_eq!(
        try_add(&h, with(c, "nom", Some(json!("Brune"))), Some(&k))
            .await
            .map(|s| s.command_id),
        Err(SubmitError::IdempotencyKeyConflict)
    );
    assert_eq!(h.rows().await, rows);
    h.drop_db().await;
}

/// Business rule 31: a replay re-evaluates nothing, so a rejection stays one
/// even once the data would allow the add.
#[tokio::test]
async fn a_replayed_rejection_stays_rejected_after_the_data_changes() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let holder = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;

    let k = key();
    let first = try_add(&h, valid(c), Some(&k)).await.unwrap();
    applier.drain().await.unwrap();
    assert_rejected(&h, &first, &["pc-name-unique-in-campaign"]).await;

    h.archive_pc(holder).await;
    applier.drain().await.unwrap();
    let replay = try_add(&h, valid(c), Some(&k)).await.unwrap();
    assert!(replay.replayed);
    assert_rejected(&h, &replay, &["pc-name-unique-in-campaign"]).await;
    assert_eq!(applier.drain().await.unwrap(), 0);
    assert_eq!(pcs_of(&h, c).await, 1);
    assert_eq!(h.version().await, v + 1);

    // A corrected command needs a new key, and applies.
    let retry = add_payload(&h, valid(c)).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(retry.command_id).await.status, State::Applied);
    h.drop_db().await;
}

/// AC "malformed commands": refused before any queue entry exists.
#[tokio::test]
async fn malformed_commands_never_reach_the_queue() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let before = footprint(&h).await;

    for extra in ["pjId", "id", "archiveLe", "archivedAt", "by"] {
        let mut p = valid(c);
        p[extra] = json!(Uuid::new_v4().to_string());
        assert_eq!(
            try_add(&h, p, Some(&key())).await.map(|s| s.command_id),
            Err(SubmitError::Malformed("unknown-field")),
            "{extra}"
        );
    }
    for k in [None, Some(""), Some("   ")] {
        assert_eq!(
            try_add(&h, valid(c), k).await.map(|s| s.command_id),
            Err(SubmitError::IdempotencyKeyRequired),
            "{k:?}"
        );
    }
    // The engine mints the id: a target is refused.
    let with_target = test_commands::sub(json!({
        "dataCapability": ajouter_pj::KEY,
        "target": { "id": Uuid::new_v4().to_string() },
        "payload": valid(c),
        "idempotencyKey": key(),
    }));
    assert_eq!(
        h.engine.submit(with_target).await.map(|s| s.command_id),
        Err(SubmitError::Malformed("target-on-insert"))
    );
    assert_eq!(footprint(&h).await, before);
    h.drop_db().await;
}

/// Business rule 19: no party size cap.
#[tokio::test]
async fn a_hundredth_pc_is_accepted_like_the_first() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    for i in 1..100 {
        h.pc(&mut applier, c, &format!("Aventurier {i}"), 1 + i % 20)
            .await;
    }
    assert_eq!(pcs_of(&h, c).await, 99);

    let s = add(&h, c, "Aventurier 100", "Barde", json!(20)).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(s.command_id).await.status, State::Applied);
    assert_eq!(pcs_of(&h, c).await, 100);
    h.assert_no_orphan().await;
    h.drop_db().await;
}
