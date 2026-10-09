//! `campagne.creerCampagne@1` driven through the engine with its real
//! manifest, on a manual clock. Every name is invented. The test inserts of
//! `common::test_commands` stay registered beside it, to prove that a key is
//! scoped to its DataCapability.

mod common;

use common::contracts::{H, assert_valid};
use common::test_commands::{self, Harness, id_of, sub};
use dataguard::{Aggregates, Lookup, State, SubmitError, Submitted};
use serde_json::{Value, json};
use uuid::Uuid;

async fn harness() -> Harness {
    let mut r = test_commands::registry();
    r.register(
        &Aggregates::embedded().unwrap(),
        creer_campagne::capability().unwrap(),
    )
    .unwrap();
    Harness::with_registry(r).await
}

fn create(name: Value, key: &str) -> Value {
    json!({
        "dataCapability": creer_campagne::KEY,
        "payload": { "name": name },
        "idempotencyKey": key,
    })
}

fn key() -> String {
    Uuid::new_v4().to_string()
}

async fn try_submit(h: &Harness, s: Value) -> Result<Submitted, SubmitError> {
    h.engine.submit(sub(s)).await
}

/// The stored name of campaign `id`, and whether it is archived.
async fn campaign_row(h: &Harness, id: Uuid) -> (String, bool) {
    sqlx::query_as("SELECT nom, \"archiveLe\" IS NOT NULL FROM campagne WHERE id = $1")
        .bind(id)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

async fn pc_count(h: &Harness, campaign: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pj WHERE \"campagneId\" = $1")
        .bind(campaign)
        .fetch_one(&h.db.pool)
        .await
        .unwrap()
}

/// AC "A valid create command is applied".
#[tokio::test]
async fn a_valid_create_applies_with_the_next_data_version() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let v = h.version().await;

    let s = h.submit(create(json!("Les Brumes"), &key())).await;
    assert!(!s.replayed);
    assert_eq!(applier.drain().await.unwrap(), 1);

    let r = h.result(s.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert_eq!(r.data_version, Some(v + 1));
    assert!(r.violations.is_empty());
    assert_eq!(r.review_id, None);
    assert_valid(H, &serde_json::to_value(&r).unwrap());

    // Exactly one row, active, with no PC.
    let id = id_of(&s);
    assert_eq!(campaign_row(&h, id).await, ("Les Brumes".into(), false));
    assert_eq!(h.rows().await.len(), 1);
    assert_eq!(pc_count(&h, id).await, 0);
    assert_eq!(h.version().await, v + 1);
    h.drop_db().await;
}

/// AC "The stored name is the trimmed, NFC-normalised form".
#[tokio::test]
async fn the_stored_name_is_trimmed_nfc() {
    let h = harness().await;
    let mut applier = h.applier().await;

    // `e` + U+0301 in, U+00E9 out.
    let a = h.submit(create(json!("  Cafe\u{301}  "), &key())).await;
    // Case and inner whitespace are kept.
    let b = h.submit(create(json!("  Le  VAL "), &key())).await;
    assert_eq!(applier.drain().await.unwrap(), 2);

    assert_eq!(campaign_row(&h, id_of(&a)).await.0, "Caf\u{e9}");
    assert_eq!(campaign_row(&h, id_of(&b)).await.0, "Le  VAL");
    h.drop_db().await;
}

/// AC "Name length is counted in Unicode characters, not bytes".
#[tokio::test]
async fn length_counts_chars_not_bytes() {
    let h = harness().await;
    let mut applier = h.applier().await;

    let ok = h.submit(create(json!("🐉".repeat(100)), &key())).await;
    let edge = h
        .submit(create(json!(format!("  {}  ", "é".repeat(100))), &key()))
        .await;
    let too_long = h.submit(create(json!("🐉".repeat(101)), &key())).await;
    applier.drain().await.unwrap();

    for s in [&ok, &edge] {
        assert_eq!(h.result(s.command_id).await.status, State::Applied);
    }
    let r = h.result(too_long.command_id).await;
    assert_eq!(r.status, State::Rejected);
    assert_eq!(r.violations, ["campaign-name-length"]);
    assert_eq!(r.data_version, None);
    assert_eq!(h.rows().await.len(), 2);
    h.drop_db().await;
}

/// AC "An empty name, a whitespace-only name, or a missing name".
#[tokio::test]
async fn campaign_name_required_rejects_with_null_data_version() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let v = h.version().await;

    let missing = json!({
        "dataCapability": creer_campagne::KEY,
        "payload": {},
        "idempotencyKey": key(),
    });
    let mut commands = vec![missing];
    for name in [
        json!(""),
        json!("   "),
        json!("\u{00A0}"),
        json!(null),
        json!(42),
    ] {
        commands.push(create(name, &key()));
    }
    for command in commands {
        let s = h.submit(command.clone()).await;
        applier.drain().await.unwrap();
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Rejected, "{command}");
        assert_eq!(r.violations, ["campaign-name-required"], "{command}");
        assert_eq!(r.data_version, None, "{command}");
        assert_eq!(r.review_id, None, "{command}");
        assert_valid(H, &serde_json::to_value(&r).unwrap());
    }
    assert!(h.rows().await.is_empty());
    assert_eq!(h.version().await, v);
    h.drop_db().await;
}

/// AC "A 101-character name (after trim) is rejected".
#[tokio::test]
async fn campaign_name_length_rejects_with_null_data_version() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let v = h.version().await;

    for name in ["N".repeat(101), format!("  {}  ", "N".repeat(101))] {
        let s = h.submit(create(json!(name), &key())).await;
        applier.drain().await.unwrap();
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Rejected);
        assert_eq!(r.violations, ["campaign-name-length"]);
        assert_eq!(r.data_version, None);
        assert_valid(H, &serde_json::to_value(&r).unwrap());
    }
    assert!(h.rows().await.is_empty());
    assert_eq!(h.version().await, v);
    h.drop_db().await;
}

/// AC "A rejected command consumes no dataVersion".
#[tokio::test]
async fn a_rejected_create_consumes_no_version() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let v = h.version().await;

    let first = h.submit(create(json!("Les Brumes"), &key())).await;
    applier.drain().await.unwrap();
    let rejected = h.submit(create(json!("  "), &key())).await;
    applier.drain().await.unwrap();
    let second = h.submit(create(json!("Le Val"), &key())).await;
    applier.drain().await.unwrap();

    assert_eq!(h.result(first.command_id).await.data_version, Some(v + 1));
    assert_eq!(h.result(rejected.command_id).await.data_version, None);
    assert_eq!(h.result(second.command_id).await.data_version, Some(v + 2));
    h.drop_db().await;
}

/// AC "Two commands with the same name but different keys".
#[tokio::test]
async fn same_name_different_keys_make_two_campaigns() {
    let h = harness().await;
    let mut applier = h.applier().await;

    let a = h.submit(create(json!("Les Brumes"), &key())).await;
    let b = h.submit(create(json!("Les Brumes"), &key())).await;
    let c = h.submit(create(json!("les brumes"), &key())).await;
    applier.drain().await.unwrap();

    let ids = [id_of(&a), id_of(&b), id_of(&c)];
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[2]);
    assert_ne!(ids[1], ids[2]);
    for s in [&a, &b, &c] {
        assert_eq!(h.result(s.command_id).await.status, State::Applied);
    }
    assert_eq!(h.rows().await.len(), 3);
    h.drop_db().await;
}

/// AC "Replaying a command with the same idempotencyKey".
#[tokio::test]
async fn replay_returns_the_original_and_writes_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let k = key();

    // Replayed while still queued: the same command, the same campaign id.
    let first = h.submit(create(json!("Les Brumes"), &k)).await;
    let again = h.submit(create(json!("Les Brumes"), &k)).await;
    assert!(again.replayed);
    assert_eq!(again.command_id, first.command_id);
    assert_eq!(again.partition, first.partition);
    assert_eq!(h.queue_rows().await, 1);

    applier.drain().await.unwrap();
    let original = h.result(first.command_id).await;
    assert_eq!(original.status, State::Applied);
    let (v, rows) = (h.version().await, h.rows().await);

    // Replayed once settled: the original result, unchanged.
    let settled = h.submit(create(json!("Les Brumes"), &k)).await;
    assert!(settled.replayed);
    assert_eq!(settled.command_id, first.command_id);
    assert_eq!(settled.lookup.result(), Some(&original));
    assert_eq!(applier.drain().await.unwrap(), 0);
    assert_eq!((h.version().await, h.rows().await), (v, rows));
    assert_eq!(h.queue_rows().await, 1);

    // A key whose first command was rejected replays that rejection.
    let k = key();
    let bad = h.submit(create(json!(""), &k)).await;
    let rejection = h.result(bad.command_id).await;
    assert_eq!(rejection.status, State::Rejected);
    let bad_again = h.submit(create(json!(""), &k)).await;
    assert!(bad_again.replayed);
    assert_eq!(bad_again.command_id, bad.command_id);
    assert_eq!(bad_again.lookup.result(), Some(&rejection));
    assert_eq!(h.version().await, v);
    h.drop_db().await;
}

/// The same key with another name creates nothing: the engine refuses it
/// with `idempotency-key-conflict` instead of answering the first result.
#[tokio::test]
async fn a_replayed_key_with_another_name_writes_nothing() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let k = key();
    let first = h.submit(create(json!("Les Brumes"), &k)).await;
    applier.drain().await.unwrap();
    let (v, rows, queued) = (h.version().await, h.rows().await, h.queue_rows().await);

    let err = try_submit(&h, create(json!("Le Val"), &k))
        .await
        .unwrap_err();
    assert_eq!(err, SubmitError::IdempotencyKeyConflict);

    assert_eq!(applier.drain().await.unwrap(), 0);
    assert_eq!((h.version().await, h.rows().await), (v, rows));
    assert_eq!(h.queue_rows().await, queued);
    assert_eq!(h.result(first.command_id).await.status, State::Applied);
    h.drop_db().await;
}

/// A key belongs to its DataCapability: `test.creerCampagne@1` and
/// `campagne.creerCampagne@1` do not collide on it.
#[tokio::test]
async fn the_key_is_scoped_to_the_capability() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let k = key();

    let test = h
        .submit(json!({
            "dataCapability": test_commands::CREER_CAMPAGNE,
            "payload": { "name": "Les Brumes" },
            "idempotencyKey": k,
        }))
        .await;
    let real = h.submit(create(json!("Les Brumes"), &k)).await;
    assert!(!real.replayed);
    assert_ne!(real.command_id, test.command_id);
    applier.drain().await.unwrap();

    for s in [&test, &real] {
        assert_eq!(h.result(s.command_id).await.status, State::Applied);
    }
    assert_eq!(h.rows().await.len(), 2);
    h.drop_db().await;
}

/// AC "Two commands with the same key arriving at once give one row".
#[tokio::test]
async fn concurrent_same_key_gives_one_row() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let k = key();

    let (a, b) = tokio::join!(
        h.engine.submit(sub(create(json!("Les Brumes"), &k))),
        h.engine.submit(sub(create(json!("Les Brumes"), &k))),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.command_id, b.command_id);
    assert_eq!(a.partition, b.partition);
    assert_eq!([a.replayed, b.replayed].iter().filter(|r| **r).count(), 1);

    applier.drain().await.unwrap();
    assert_eq!(h.result(a.command_id).await.status, State::Applied);
    assert_eq!(h.rows().await.len(), 1);
    assert_eq!(h.queue_rows().await, 1);
    h.drop_db().await;
}

/// AC "Two commands with different keys on the same name, arriving at once".
#[tokio::test]
async fn concurrent_different_keys_both_apply_in_order() {
    let h = harness().await;
    let mut applier = h.applier().await;
    let v = h.version().await;

    let (a, b) = tokio::join!(
        h.engine.submit(sub(create(json!("Les Brumes"), &key()))),
        h.engine.submit(sub(create(json!("Les Brumes"), &key()))),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(id_of(&a), id_of(&b));
    assert_eq!(applier.drain().await.unwrap(), 2);

    let mut versions = Vec::new();
    for s in [&a, &b] {
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Applied);
        versions.push(r.data_version.unwrap());
    }
    versions.sort_unstable();
    assert_eq!(versions, [v + 1, v + 2]);
    assert_eq!(h.rows().await.len(), 2);
    h.drop_db().await;
}

/// AC "A command without an idempotencyKey is refused and creates no row".
#[tokio::test]
async fn a_missing_or_blank_key_is_refused_before_enqueue() {
    let h = harness().await;
    let _applier = h.applier().await;
    let v = h.version().await;

    let none = json!({
        "dataCapability": creer_campagne::KEY,
        "payload": { "name": "Les Brumes" },
    });
    let mut commands = vec![none];
    for k in ["", "  "] {
        commands.push(create(json!("Les Brumes"), k));
    }
    // A blank key is refused before the name is looked at.
    commands.push(create(json!(""), " "));
    for command in commands {
        let err = try_submit(&h, command.clone()).await.unwrap_err();
        assert_eq!(err, SubmitError::IdempotencyKeyRequired, "{command}");
    }
    assert_eq!(h.queue_rows().await, 0);
    assert!(h.rows().await.is_empty());
    assert_eq!(h.version().await, v);
    h.drop_db().await;
}

/// AC "Extra payload keys are ignored" and "no target on a create".
#[tokio::test]
async fn a_target_or_a_tombstone_in_the_payload_never_lands() {
    let h = harness().await;
    let mut applier = h.applier().await;

    let err = try_submit(
        &h,
        json!({
            "dataCapability": creer_campagne::KEY,
            "target": { "id": Uuid::new_v4() },
            "payload": { "name": "Les Brumes" },
            "idempotencyKey": key(),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err, SubmitError::Malformed("target-on-insert"));
    assert_eq!(h.queue_rows().await, 0);

    // A client-chosen id and tombstone are dropped: the id is the minted one.
    let chosen = Uuid::new_v4();
    let s = h
        .submit(json!({
            "dataCapability": creer_campagne::KEY,
            "payload": {
                "name": "Les Brumes",
                "id": chosen,
                "archivedAt": "2026-03-01T20:00:00Z",
                "by": "someone-else",
            },
            "idempotencyKey": key(),
        }))
        .await;
    applier.drain().await.unwrap();
    assert_eq!(h.result(s.command_id).await.status, State::Applied);
    assert_ne!(id_of(&s), chosen);
    assert_eq!(
        campaign_row(&h, id_of(&s)).await,
        ("Les Brumes".into(), false)
    );
    assert_eq!(h.rows().await.len(), 1);
    assert!(matches!(
        h.lookup(s.command_id).await,
        Lookup::Settled { .. }
    ));
    h.drop_db().await;
}
