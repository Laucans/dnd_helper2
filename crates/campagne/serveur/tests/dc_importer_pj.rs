//! `campagne.importerPj@1`, the fast-path insert of a PC read elsewhere,
//! driven through the engine with its real manifest, on a manual clock.
//! Campaigns and the PCs that seed a scenario come from the test-only inserts
//! of `common::test_commands` or from `campagne.ajouterPj`; every name,
//! campaign and external id here is invented.

mod common;

use campagne_serveur::startup;
use common::test_commands::{self, Harness};
use dataguard::{Aggregates, State, SubmitError, Submitted};
use serde_json::{Value, json};
use uuid::Uuid;

/// The test inserts, for seeding, the real add and the real import.
async fn harness() -> Harness {
    let aggregates = Aggregates::embedded().unwrap();
    let mut r = test_commands::registry();
    r.register(&aggregates, ajouter_pj::capability().unwrap())
        .unwrap();
    r.register(&aggregates, importer_pj::capability().unwrap())
        .unwrap();
    Harness::with_registry(r).await
}

fn key() -> String {
    Uuid::new_v4().to_string()
}

/// A payload the DataGuard accepts, for `campaign`, holding `external_id`.
fn valid(campaign: Uuid, nom: &str, external_id: &str) -> Value {
    json!({
        "campagneId": campaign.to_string(),
        "nom": nom,
        "classe": "Barde",
        "niveau": 5,
        "externalId": external_id,
    })
}

/// `payload` with `field` set to `value`, or removed when `value` is `None`.
fn with(mut payload: Value, field: &str, value: Option<Value>) -> Value {
    let object = payload.as_object_mut().unwrap();
    match value {
        Some(v) => object.insert(field.into(), v),
        None => object.remove(field),
    };
    payload
}

/// An import with the key given; the error is what is refused before the
/// queue.
async fn try_import(
    h: &Harness,
    payload: Value,
    key: Option<&str>,
) -> Result<Submitted, SubmitError> {
    let mut s = json!({ "dataCapability": importer_pj::KEY, "payload": payload });
    if let Some(k) = key {
        s["idempotencyKey"] = json!(k);
    }
    h.engine.submit(test_commands::sub(s)).await
}

/// An import under a fresh key.
async fn import(h: &Harness, campaign: Uuid, nom: &str, external_id: &str) -> Submitted {
    try_import(h, valid(campaign, nom, external_id), Some(&key()))
        .await
        .unwrap()
}

/// A hand-entered add under a fresh key.
async fn add(h: &Harness, campaign: Uuid, nom: &str) -> Submitted {
    h.engine
        .submit(test_commands::sub(json!({
            "dataCapability": ajouter_pj::KEY,
            "payload": {
                "campagneId": campaign.to_string(),
                "nom": nom,
                "classe": "Barde",
                "niveau": 5,
            },
            "idempotencyKey": key(),
        })))
        .await
        .unwrap()
}

/// Rows of `pj` in `campaign`, archived ones included.
async fn pcs_of(h: &Harness, campaign: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pj WHERE \"campagneId\" = $1")
        .bind(campaign)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

/// `origine` and `idExterne` of one PC.
async fn origin_of(h: &Harness, pc: Uuid) -> (String, Option<String>) {
    sqlx::query_as("SELECT origine, \"idExterne\" FROM pj WHERE id = $1")
        .bind(pc)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

/// The whole stored row of a PC, as text: what "untouched" compares.
async fn pc_text(h: &Harness, pc: Uuid) -> String {
    sqlx::query_scalar("SELECT p::text FROM pj p WHERE id = $1")
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

/// The registry the server ships has the import.
#[test]
fn the_server_registers_importer_pj() {
    let r = startup::registry(&Aggregates::embedded().unwrap()).unwrap();
    assert!(r.get(importer_pj::KEY).is_some());
    assert!(r.get(ajouter_pj::KEY).is_some());
}

/// AC "A valid import": one new row, `dndbeyond`, its id, applied.
#[tokio::test]
async fn a_valid_import_applies_with_one_row_origin_dndbeyond_and_its_id() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let other = h.campaign(&mut applier, "Le Val").await;
    let campaign_before = campaign_row(&h, c).await;
    let other_before = campaign_row(&h, other).await;
    let v = h.version().await;

    let s = import(&h, c, " Ysolde\u{00A0}", "4242").await;
    assert_eq!(s.partition, format!("PJ/{}", test_commands::id_of(&s)));
    assert!(!s.replayed);
    assert!(s.warnings.is_empty());
    assert_eq!(applier.drain().await.unwrap(), 1);
    let r = h.result(s.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert!(r.violations.is_empty());
    assert_eq!(r.data_version, Some(v + 1));
    assert_eq!(h.version().await, v + 1);

    let pc = test_commands::id_of(&s);
    assert_eq!(
        origin_of(&h, pc).await,
        ("dndbeyond".into(), Some("4242".into()))
    );
    // The other columns are what a hand-entered add would store.
    let (campaign, nom, classe, niveau, archived): (Uuid, String, String, i32, Option<Value>) =
        sqlx::query_as(
            "SELECT \"campagneId\", nom, classe, niveau, to_jsonb(\"archiveLe\") FROM pj
             WHERE id = $1",
        )
        .bind(pc)
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(
        (campaign, nom.as_str(), classe.as_str(), niveau, archived),
        (c, "Ysolde", "Barde", 5, None)
    );

    // No other row of any campaign moved.
    assert_eq!(pcs_of(&h, c).await, 1);
    assert_eq!(pcs_of(&h, other).await, 0);
    assert_eq!(campaign_row(&h, c).await, campaign_before);
    assert_eq!(campaign_row(&h, other).await, other_before);
    h.drop_db().await;
}

/// AC "pc-external-id-unique-in-campaign refuses an insert whose externalId
/// matches an active PC": that id is the violation, nothing is written, and
/// the first row is untouched.
#[tokio::test]
async fn pc_external_id_unique_in_campaign_refuses_a_second_import_and_leaves_the_first_row_untouched()
 {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    let first = import(&h, c, "Ysolde", "4242").await;
    applier.drain().await.unwrap();
    let pc = test_commands::id_of(&first);
    let row = pc_text(&h, pc).await;
    let (_, version, pcs) = footprint(&h).await;

    // A different name, so that only the id can be the reason.
    let second = import(&h, c, "Brune", "4242").await;
    applier.drain().await.unwrap();
    assert_rejected(&h, &second, &["pc-external-id-unique-in-campaign"]).await;
    assert_eq!(pc_text(&h, pc).await, row);
    assert_eq!(pcs_of(&h, c).await, 1);
    let after = footprint(&h).await;
    assert_eq!((after.1, after.2), (version, pcs));
    h.drop_db().await;
}

/// AC "The same externalId is accepted in another campaign, and again in the
/// same campaign once the earlier PC is archived."
#[tokio::test]
async fn the_same_external_id_is_accepted_in_another_campaign_and_after_an_archive() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let a = h.campaign(&mut applier, "Les Brumes").await;
    let b = h.campaign(&mut applier, "Le Val").await;

    let in_a = import(&h, a, "Ysolde", "4242").await;
    applier.drain().await.unwrap();
    let in_b = import(&h, b, "Ysolde", "4242").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(in_a.command_id).await.status, State::Applied);
    assert_eq!(h.result(in_b.command_id).await.status, State::Applied);
    assert_eq!(
        origin_of(&h, test_commands::id_of(&in_b)).await,
        ("dndbeyond".into(), Some("4242".into()))
    );

    // Once archived, the PC holds neither its id nor its name.
    let holder = test_commands::id_of(&in_a);
    h.archive_pc(holder).await;
    applier.drain().await.unwrap();
    let tombstone = pc_text(&h, holder).await;
    let again = import(&h, a, "Ysolde", "4242").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(again.command_id).await.status, State::Applied);
    assert_eq!(
        origin_of(&h, test_commands::id_of(&again)).await,
        ("dndbeyond".into(), Some("4242".into()))
    );
    // The tombstone keeps its id and is not rewritten.
    assert_eq!(pc_text(&h, holder).await, tombstone);
    assert_eq!(pcs_of(&h, a).await, 2);
    assert_eq!(pcs_of(&h, b).await, 1);
    h.drop_db().await;
}

/// AC "Two commands with the same externalId, back to back": the first
/// applies, the second is warned at enqueue and refused at apply. Neither is
/// refused for the other being in flight.
#[tokio::test]
async fn two_back_to_back_imports_of_one_id_first_applies_second_warned_then_rejected() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let v = h.version().await;

    let first = import(&h, c, "Ysolde", "4242").await;
    let second = import(&h, c, "Brune", "4242").await;
    assert!(first.warnings.is_empty());
    assert_eq!(second.warnings, ["pc-external-id-unique-in-campaign"]);
    assert_eq!(first.lookup.state(), State::Queued);
    assert_eq!(second.lookup.state(), State::Queued);
    assert_eq!(applier.drain().await.unwrap(), 2);

    let r = h.result(first.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert_eq!(r.data_version, Some(v + 1));
    assert_rejected(&h, &second, &["pc-external-id-unique-in-campaign"]).await;
    assert_eq!(pcs_of(&h, c).await, 1);
    assert_eq!(h.version().await, v + 1);

    // Submitted together: the one lower in the queue applies.
    let (a, b) = tokio::join!(
        import(&h, c, "Aldric", "9001"),
        import(&h, c, "Sorcha", "9001"),
    );
    assert_eq!(a.lookup.state(), State::Queued);
    assert_eq!(b.lookup.state(), State::Queued);
    let (winner, loser) = if enqueue_seq(&h, &a).await < enqueue_seq(&h, &b).await {
        (&a, &b)
    } else {
        (&b, &a)
    };
    assert_eq!(applier.drain().await.unwrap(), 2);
    assert_eq!(h.result(winner.command_id).await.status, State::Applied);
    assert_rejected(&h, loser, &["pc-external-id-unique-in-campaign"]).await;
    assert_eq!(pcs_of(&h, c).await, 2);
    assert_eq!(h.version().await, v + 2);
    h.drop_db().await;
}

/// AC "Replaying a command with the same idempotency key": the first result,
/// no second row, no data version, whether the original applied or was
/// refused as a duplicate.
#[tokio::test]
async fn replay_returns_the_first_result_and_writes_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    // An original that applied.
    let k = key();
    let payload = valid(c, "Ysolde", "4242");
    let first = try_import(&h, payload.clone(), Some(&k)).await.unwrap();
    applier.drain().await.unwrap();
    let original = h.result(first.command_id).await;
    assert_eq!(original.status, State::Applied);
    let (_, version, pcs) = footprint(&h).await;

    let replay = try_import(&h, payload, Some(&k)).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.command_id, first.command_id);
    assert_eq!(replay.partition, first.partition);
    assert_eq!(h.result(replay.command_id).await, original);
    assert_eq!(applier.drain().await.unwrap(), 0);
    assert_eq!(footprint(&h).await, (h.queue_rows().await, version, pcs));
    assert_eq!(pcs_of(&h, c).await, 1);

    // An original refused as a duplicate stays refused, even once the id is
    // free again: a replay re-evaluates nothing.
    let k2 = key();
    let dup_payload = valid(c, "Brune", "4242");
    let dup = try_import(&h, dup_payload.clone(), Some(&k2))
        .await
        .unwrap();
    applier.drain().await.unwrap();
    assert_rejected(&h, &dup, &["pc-external-id-unique-in-campaign"]).await;
    h.archive_pc(test_commands::id_of(&first)).await;
    applier.drain().await.unwrap();
    let version = h.version().await;
    let pcs = pcs_of(&h, c).await;

    let replay = try_import(&h, dup_payload, Some(&k2)).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.command_id, dup.command_id);
    assert_rejected(&h, &replay, &["pc-external-id-unique-in-campaign"]).await;
    assert_eq!(applier.drain().await.unwrap(), 0);
    assert_eq!(h.version().await, version);
    assert_eq!(pcs_of(&h, c).await, pcs);

    // A new key with the same id is no replay: it is applied on its merits.
    let fresh = import(&h, c, "Brune", "4242").await;
    assert!(!fresh.replayed);
    applier.drain().await.unwrap();
    assert_eq!(h.result(fresh.command_id).await.status, State::Applied);
    h.drop_db().await;
}

/// AC "A missing key is refused. The same key with a different payload is
/// refused." The key is scoped by capability.
#[tokio::test]
async fn a_missing_key_is_refused_and_a_reused_key_with_another_payload_conflicts() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let before = footprint(&h).await;

    for k in [None, Some(""), Some("   ")] {
        assert_eq!(
            try_import(&h, valid(c, "Ysolde", "4242"), k)
                .await
                .map(|s| s.command_id),
            Err(SubmitError::IdempotencyKeyRequired),
            "{k:?}"
        );
    }
    assert_eq!(footprint(&h).await, before);

    let k = key();
    let first = try_import(&h, valid(c, "Ysolde", "4242"), Some(&k))
        .await
        .unwrap();
    applier.drain().await.unwrap();
    let rows = h.rows().await;
    for other in [
        valid(c, "Brune", "4242"),
        valid(c, "Ysolde", "4243"),
        with(valid(c, "Ysolde", "4242"), "niveau", Some(json!(6))),
    ] {
        assert_eq!(
            try_import(&h, other, Some(&k)).await.map(|s| s.command_id),
            Err(SubmitError::IdempotencyKeyConflict)
        );
    }
    assert_eq!(h.rows().await, rows);

    // The same key on another capability is another key.
    let hand = h
        .engine
        .submit(test_commands::sub(json!({
            "dataCapability": ajouter_pj::KEY,
            "payload": { "campagneId": c.to_string(), "nom": "Aldric", "classe": "Barde", "niveau": 5 },
            "idempotencyKey": k,
        })))
        .await
        .unwrap();
    assert!(!hand.replayed);
    assert_ne!(hand.command_id, first.command_id);
    h.drop_db().await;
}

/// AC "Absent, non-canonical or unknown": refused as malformed, nothing is
/// queued, no row is written.
#[tokio::test]
async fn malformed_imports_never_reach_the_queue() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let before = footprint(&h).await;
    let pcs = pcs_of(&h, c).await;
    let valid = valid(c, "Ysolde", "4242");

    for absent in [None, Some(json!(null))] {
        assert_eq!(
            try_import(
                &h,
                with(valid.clone(), "externalId", absent.clone()),
                Some(&key())
            )
            .await
            .map(|s| s.command_id),
            Err(SubmitError::Malformed("external-id-required")),
            "{absent:?}"
        );
    }
    let nineteen = "1".repeat(19);
    for bad in [
        json!(""),
        json!("0"),
        json!("007"),
        json!(" 12"),
        json!("12 "),
        json!("12.0"),
        json!("-5"),
        json!("abc"),
        json!(nineteen),
        json!(12),
        json!(["12"]),
    ] {
        assert_eq!(
            try_import(
                &h,
                with(valid.clone(), "externalId", Some(bad.clone())),
                Some(&key())
            )
            .await
            .map(|s| s.command_id),
            Err(SubmitError::Malformed("external-id-format")),
            "{bad}"
        );
    }
    for extra in [
        "origin",
        "origine",
        "idExterne",
        "url",
        "lien",
        "pjId",
        "id",
        "archivedAt",
        "archiveLe",
        "by",
    ] {
        assert_eq!(
            try_import(
                &h,
                with(valid.clone(), extra, Some(json!("x"))),
                Some(&key())
            )
            .await
            .map(|s| s.command_id),
            Err(SubmitError::Malformed("unknown-field")),
            "{extra}"
        );
    }
    // The engine mints the id: a target is refused.
    let with_target = test_commands::sub(json!({
        "dataCapability": importer_pj::KEY,
        "target": { "id": Uuid::new_v4().to_string() },
        "payload": valid,
        "idempotencyKey": key(),
    }));
    assert_eq!(
        h.engine.submit(with_target).await.map(|s| s.command_id),
        Err(SubmitError::Malformed("target-on-insert"))
    );
    assert_eq!(footprint(&h).await, before);
    assert_eq!(pcs_of(&h, c).await, pcs);
    h.drop_db().await;
}

/// AC "A missing or archived campaign": `campaign-active`, nothing written.
#[tokio::test]
async fn campaign_active_refuses_a_missing_or_archived_campaign() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let archived = h.campaign(&mut applier, "Les Brumes").await;
    let kept = import(&h, archived, "Ysolde", "4242").await;
    applier.drain().await.unwrap();
    h.archive_campaign(archived).await;
    applier.drain().await.unwrap();
    assert!(h.campaign_archived_at(archived).await.is_some());
    let campaign_row_before = campaign_row(&h, archived).await;
    let kept_row = pc_text(&h, test_commands::id_of(&kept)).await;
    let unknown = Uuid::new_v4();
    let before = footprint(&h).await;

    let a = import(&h, unknown, "Brune", "5151").await;
    // The archived campaign's PC holds this id, but its tombstone does not
    // count: the campaign alone is the reason.
    let b = import(&h, archived, "Brune", "4242").await;
    assert_eq!(a.warnings, ["campaign-active"]);
    assert_eq!(b.warnings, ["campaign-active"]);
    applier.drain().await.unwrap();
    assert_rejected(&h, &a, &["campaign-active"]).await;
    assert_rejected(&h, &b, &["campaign-active"]).await;
    assert_eq!(pcs_of(&h, unknown).await, 0);
    assert_eq!(pcs_of(&h, archived).await, 1);
    assert_eq!(campaign_row(&h, archived).await, campaign_row_before);
    assert_eq!(pc_text(&h, test_commands::id_of(&kept)).await, kept_row);
    assert_eq!(h.version().await, before.1);
    h.assert_no_orphan().await;
    h.drop_db().await;
}

/// AC "An empty name, an empty class or a level outside 1-20": the invariants
/// `ajouterPj` already uses, nothing written.
#[tokio::test]
async fn reused_invariants_refuse_and_save_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let version = h.version().await;
    let id = "4242";

    let cases: [(&str, Option<Value>, &[&str]); 7] = [
        ("nom", Some(json!("")), &["pc-name-required"]),
        ("nom", Some(json!("   ")), &["pc-name-required"]),
        ("nom", None, &["pc-name-required"]),
        ("classe", Some(json!("")), &["pc-class-required"]),
        ("classe", None, &["pc-class-required"]),
        ("niveau", Some(json!(0)), &["pc-level-range"]),
        ("niveau", Some(json!(21)), &["pc-level-range"]),
    ];
    for (field, value, expected) in cases {
        let s = try_import(
            &h,
            with(valid(c, "Ysolde", id), field, value.clone()),
            Some(&key()),
        )
        .await
        .unwrap();
        applier.drain().await.unwrap();
        assert_rejected(&h, &s, expected).await;
        assert_eq!(pcs_of(&h, c).await, 0, "{field} {value:?}");
        assert_eq!(h.version().await, version);
    }
    let s = try_import(
        &h,
        json!({ "campagneId": c.to_string(), "externalId": id }),
        Some(&key()),
    )
    .await
    .unwrap();
    applier.drain().await.unwrap();
    assert_rejected(
        &h,
        &s,
        &["pc-name-required", "pc-class-required", "pc-level-range"],
    )
    .await;

    // A refused import holds nothing: the same id then applies.
    let ok = import(&h, c, "Ysolde", id).await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(ok.command_id).await.status, State::Applied);
    h.drop_db().await;
}

/// The conflict with milestone rule 21, for human review: the import keeps
/// `pc-name-unique-in-campaign`, so a same name with another id is refused.
/// This test pins that, so a future relaxation is a deliberate change.
#[tokio::test]
async fn pc_name_unique_in_campaign_still_refuses_a_same_name_import_with_another_id() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let other = h.campaign(&mut applier, "Le Val").await;

    let first = import(&h, c, "Ysolde", "4242").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(first.command_id).await.status, State::Applied);

    let same_name = import(&h, c, " YSOLDE ", "4243").await;
    assert_eq!(same_name.warnings, ["pc-name-unique-in-campaign"]);
    applier.drain().await.unwrap();
    assert_rejected(&h, &same_name, &["pc-name-unique-in-campaign"]).await;
    assert_eq!(pcs_of(&h, c).await, 1);

    // Whatever the origin of the PC that holds the name.
    let hand = add(&h, c, "Aldric").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(hand.command_id).await.status, State::Applied);
    let imported = import(&h, c, "aldric", "4244").await;
    applier.drain().await.unwrap();
    assert_rejected(&h, &imported, &["pc-name-unique-in-campaign"]).await;
    assert_eq!(pcs_of(&h, c).await, 2);

    // Another campaign is another namespace.
    let elsewhere = import(&h, other, "Ysolde", "4243").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(elsewhere.command_id).await.status, State::Applied);
    h.drop_db().await;
}

/// Business rule 17: several broken rules, all reported, in declaration order.
#[tokio::test]
async fn name_and_id_duplicates_are_both_reported() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    import(&h, c, "Ysolde", "4242").await;
    applier.drain().await.unwrap();

    let both = import(&h, c, "ysolde", "4242").await;
    let expected = [
        "pc-name-unique-in-campaign",
        "pc-external-id-unique-in-campaign",
    ];
    assert_eq!(both.warnings, expected);
    applier.drain().await.unwrap();
    assert_rejected(&h, &both, &expected).await;

    // With a bad level too: the engine decides the payload first, as it does
    // for `ajouterPj` (a bad name hides an unknown campaign the same way), so
    // the row-dependent ids are reported once the fields themselves are valid.
    let all = try_import(
        &h,
        with(valid(c, "ysolde", "4242"), "niveau", Some(json!(0))),
        Some(&key()),
    )
    .await
    .unwrap();
    applier.drain().await.unwrap();
    assert_rejected(&h, &all, &["pc-level-range"]).await;
    assert_eq!(pcs_of(&h, c).await, 1);
    h.drop_db().await;
}

/// Business rule 31: an imported PC reads like a hand-entered one; only `id`
/// and `creeLe` differ, and no view shows the origin or the external id.
#[tokio::test]
async fn an_imported_pc_reads_like_a_hand_entered_one_in_pj_actif() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let a = h.campaign(&mut applier, "Les Brumes").await;
    let b = h.campaign(&mut applier, "Le Val").await;

    let imported = import(&h, a, "Ysolde", "4242").await;
    let hand = add(&h, b, "Ysolde").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(imported.command_id).await.status, State::Applied);
    assert_eq!(h.result(hand.command_id).await.status, State::Applied);

    let view = |pc: Uuid| {
        let pool = h.db.pool.clone();
        async move {
            let mut row: serde_json::Map<String, Value> =
                sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Map<String, Value>>>(
                    "SELECT to_jsonb(v) FROM pj_actif v WHERE id = $1",
                )
                .bind(pc)
                .fetch_one(&pool)
                .await
                .unwrap()
                .0;
            for key in ["id", "creeLe", "campagneId"] {
                assert!(row.remove(key).is_some(), "{key}");
            }
            row
        }
    };
    let (i, m) = (
        view(test_commands::id_of(&imported)).await,
        view(test_commands::id_of(&hand)).await,
    );
    assert_eq!(i, m);
    assert_eq!(
        (
            i["nom"].as_str(),
            i["classe"].as_str(),
            i["niveau"].as_i64()
        ),
        (Some("Ysolde"), Some("Barde"), Some(5))
    );
    assert_eq!(i.len(), 3);
    h.drop_db().await;
}

/// Business rules 3 and 4: `ajouterPj` writes `manual` and no external id,
/// and takes neither field from its payload.
#[tokio::test]
async fn ajouter_pj_still_writes_origin_manual_and_no_external_id() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;

    let s = add(&h, c, "Ysolde").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(s.command_id).await.status, State::Applied);
    assert_eq!(
        origin_of(&h, test_commands::id_of(&s)).await,
        ("manual".into(), None)
    );
    // Any number of hand-entered PCs coexist: null never collides with null.
    let t = add(&h, c, "Aldric").await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(t.command_id).await.status, State::Applied);

    let before = footprint(&h).await;
    for extra in ["origin", "externalId", "idExterne", "origine"] {
        let mut payload = json!({
            "campagneId": c.to_string(), "nom": "Brune", "classe": "Barde", "niveau": 5,
        });
        payload[extra] = json!("dndbeyond");
        let refused = h
            .engine
            .submit(test_commands::sub(json!({
                "dataCapability": ajouter_pj::KEY,
                "payload": payload,
                "idempotencyKey": key(),
            })))
            .await;
        assert_eq!(
            refused.map(|s| s.command_id),
            Err(SubmitError::Malformed("unknown-field")),
            "{extra}"
        );
    }
    assert_eq!(footprint(&h).await, before);
    h.drop_db().await;
}

/// Business rule 4: written once. No update command declares either field,
/// so none can carry it.
#[tokio::test]
async fn origin_and_external_id_cannot_be_changed_after_the_insert() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let s = import(&h, c, "Ysolde", "4242").await;
    applier.drain().await.unwrap();
    let pc = test_commands::id_of(&s);
    let row = pc_text(&h, pc).await;
    let before = footprint(&h).await;

    for (field, value) in [
        ("origin", json!("manual")),
        ("externalId", json!("9")),
        ("origin", json!("dndbeyond")),
    ] {
        for capability in [test_commands::REGLER_NIVEAU, test_commands::MODIFIER_PJ] {
            let mut sub = json!({
                "dataCapability": capability,
                "target": { "id": pc.to_string() },
                "payload": {},
                "basedOn": { "version": h.version().await },
            });
            sub["payload"][field] = value.clone();
            assert_eq!(
                h.engine
                    .submit(test_commands::sub(sub))
                    .await
                    .map(|s| s.command_id),
                Err(SubmitError::Malformed("field-not-touched")),
                "{capability} {field}"
            );
        }
    }
    assert_eq!(footprint(&h).await, before);
    assert_eq!(pc_text(&h, pc).await, row);
    h.drop_db().await;
}
