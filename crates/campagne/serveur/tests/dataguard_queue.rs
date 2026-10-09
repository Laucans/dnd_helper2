//! The DataQueue: FIFO per partition, never a concurrency refusal, one
//! applier, one gapless global `dataVersion`, idempotent replays, and a
//! rollback that leaves a failing command in place.

mod common;

use std::time::Duration;

use common::contracts::{G, H, L, assert_lookup_valid, assert_valid};
use common::test_commands::{AJOUTER_PJ, CREER_CAMPAGNE, Harness, sub};
use dataguard::{CommandId, GM_IDENTITY, Lookup, State, SubmitError};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::PgConnection;
use uuid::Uuid;

#[tokio::test]
async fn the_version_starts_at_zero_and_applied_versions_have_no_gap() {
    let h = Harness::new().await;
    assert_eq!(h.version().await, 0);
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    assert_eq!(h.version().await, 1);

    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    // Rejected at enqueue, rejected at application, cancelled, expired:
    // none consumes a version.
    h.add_pc(c, "Bad", json!(0)).await;
    h.add_pc(c, "ysolde", json!(2)).await;
    let cancelled = h.set_level(pc, 9).await;
    h.engine
        .cancel(cancelled.command_id, GM_IDENTITY)
        .await
        .unwrap();
    let v = h.version().await;
    h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let parked = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    applier.drain().await.unwrap();
    h.clock.advance(Duration::from_secs(24 * 3600));
    applier.drain().await.unwrap();
    assert_eq!(h.result(parked.command_id).await.status, State::Expired);
    h.set_level(pc, 7).await;
    applier.drain().await.unwrap();

    let applied: Vec<i64> = sqlx::query_scalar(
        "SELECT data_version FROM dataguard_queue WHERE state = 'applied' ORDER BY data_version",
    )
    .fetch_all(&h.db.pool)
    .await
    .unwrap();
    assert_eq!(applied, (1..=applied.len() as i64).collect::<Vec<_>>());
    assert_eq!(applied.len(), 4);
    assert_eq!(h.version().await, 4);
    let unversioned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dataguard_queue WHERE state <> 'applied' AND data_version IS NOT NULL",
    )
    .fetch_one(&h.db.pool)
    .await
    .unwrap();
    assert_eq!(unversioned, 0);
    h.drop_db().await;
}

#[tokio::test]
async fn commands_apply_in_position_order_and_across_partitions_in_enqueue_order() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let a = h.campaign(&mut applier, "A").await;
    let b = h.campaign(&mut applier, "B").await;
    let pa = h.pc(&mut applier, a, "Ysolde", 1).await;

    let mut subs = Vec::new();
    for level in [5, 6, 7] {
        subs.push(h.set_level(pa, level).await);
        subs.push(h.add_pc(b, &format!("Pj {level}"), json!(level)).await);
    }
    let positions: Vec<i64> = subs
        .iter()
        .step_by(2)
        .map(|s| s.lookup.entry().unwrap().position)
        .collect();
    assert_eq!(positions, [1, 2, 3]);
    applier.drain().await.unwrap();

    let mut versions = Vec::new();
    for s in &subs {
        let r = h.result(s.command_id).await;
        assert_eq!(r.status, State::Applied);
        versions.push(r.data_version.unwrap());
    }
    let mut sorted = versions.clone();
    sorted.sort();
    assert_eq!(versions, sorted, "applied in submission order");
    assert_eq!(h.pc_row(pa).await.1, 7);
    h.drop_db().await;
}

#[tokio::test]
async fn submissions_during_an_open_apply_are_queued_never_refused() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 1).await;

    // Hold what an applying transaction holds: the head entry, the rows and
    // the version counter.
    let head = h.set_level(pc, 2).await;
    let mut held = PgConnection::connect(&h.db.url).await.unwrap();
    let mut tx = held.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM dataguard_queue WHERE command = $1 FOR UPDATE")
        .bind(head.command_id.0)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM pj WHERE id = $1 FOR UPDATE")
        .bind(pc)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM campagne WHERE id = $1 FOR UPDATE")
        .bind(c)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT version FROM dataguard_version FOR UPDATE")
        .execute(&mut *tx)
        .await
        .unwrap();

    let mut tasks = Vec::new();
    for level in 3..23 {
        let engine = h.engine.clone();
        tasks.push(tokio::spawn(async move {
            engine
                .submit(sub(json!({
                    "dataCapability": "test.reglerNiveau@1",
                    "target": { "id": pc.to_string() },
                    "payload": { "level": level.min(20) },
                })))
                .await
        }));
    }
    let mut positions = Vec::new();
    for t in tasks {
        let s = tokio::time::timeout(Duration::from_secs(10), t)
            .await
            .expect("a submission waited on the open apply")
            .unwrap()
            .expect("a submission was refused");
        assert_eq!(s.lookup.state(), State::Queued);
        positions.push(s.lookup.entry().unwrap().position);
    }
    positions.sort();
    positions.dedup();
    assert_eq!(positions.len(), 20, "positions are unique");
    tx.rollback().await.unwrap();

    assert_eq!(applier.drain().await.unwrap(), 21);
    let applied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM dataguard_queue WHERE state = 'applied' AND partition = $1",
    )
    .bind(format!("PJ/{pc}"))
    .fetch_one(&h.db.pool)
    .await
    .unwrap();
    assert_eq!(applied, 22);
    // The last by position wins.
    let last: String = sqlx::query_scalar(
        "SELECT payload FROM dataguard_queue WHERE partition = $1 ORDER BY position DESC LIMIT 1",
    )
    .bind(format!("PJ/{pc}"))
    .fetch_one(&h.db.pool)
    .await
    .unwrap();
    let last: Value = serde_json::from_str(&last).unwrap();
    assert_eq!(
        i64::from(h.pc_row(pc).await.1),
        last["level"].as_i64().unwrap()
    );
    h.drop_db().await;
}

#[tokio::test]
async fn a_replay_returns_the_original_whatever_its_state() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let create = |key: &str, name: &str| {
        json!({
            "dataCapability": CREER_CAMPAGNE,
            "payload": { "name": name },
            "idempotencyKey": key,
            "by": "intrus",
        })
    };

    // Still queued.
    let first = h.submit(create("k-queued", "Les Brumes")).await;
    let again = h.submit(create("k-queued", "Les Brumes")).await;
    assert!(!first.replayed);
    assert!(again.replayed);
    assert_eq!(again.command_id, first.command_id);
    assert_eq!(again.partition, first.partition);
    assert_eq!(h.queue_rows().await, 1);

    // Applied.
    applier.drain().await.unwrap();
    let version = h.version().await;
    let after = h.submit(create("k-queued", "Les Brumes")).await;
    assert_eq!(after.command_id, first.command_id);
    assert_eq!(after.partition, first.partition);
    assert_eq!(after.lookup.result().unwrap().status, State::Applied);
    applier.drain().await.unwrap();
    assert_eq!(h.version().await, version);
    let campaigns: i64 = sqlx::query_scalar("SELECT count(*) FROM campagne")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(campaigns, 1);

    // Rejected: never retried.
    let rejected = h.submit(create("k-rejected", "")).await;
    let replay = h.submit(create("k-rejected", "")).await;
    assert_eq!(replay.command_id, rejected.command_id);
    assert_eq!(replay.lookup.result().unwrap().status, State::Rejected);
    assert_eq!(h.queue_rows().await, 2);

    // Same key, other payload: refused, the original untouched.
    let conflict = h
        .engine
        .submit(sub(create("k-queued", "Autre nom")))
        .await
        .unwrap_err();
    assert_eq!(conflict, SubmitError::IdempotencyKeyConflict);
    assert_eq!(h.queue_rows().await, 2);

    // Required and missing.
    let missing = h
        .engine
        .submit(sub(json!({
            "dataCapability": CREER_CAMPAGNE,
            "payload": { "name": "Sans clé" },
        })))
        .await
        .unwrap_err();
    assert_eq!(missing, SubmitError::IdempotencyKeyRequired);
    assert_eq!(h.queue_rows().await, 2);
    h.drop_db().await;
}

#[tokio::test]
async fn concurrent_submissions_with_one_key_make_one_entry() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let rows_before = h.queue_rows().await;
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let engine = h.engine.clone();
        tasks.push(tokio::spawn(async move {
            engine
                .submit(sub(json!({
                    "dataCapability": AJOUTER_PJ,
                    "payload": {
                        "campagneId": c.to_string(), "name": "Ysolde", "class": "Barde", "level": 3,
                    },
                    "idempotencyKey": "double-clic",
                })))
                .await
                .unwrap()
        }));
    }
    let mut ids = Vec::new();
    for t in tasks {
        ids.push(t.await.unwrap().command_id);
    }
    ids.dedup();
    assert_eq!(ids.len(), 1);
    assert_eq!(h.queue_rows().await, rows_before + 1);
    applier.drain().await.unwrap();
    let pcs: i64 = sqlx::query_scalar("SELECT count(*) FROM pj")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(pcs, 1);
    h.drop_db().await;
}

#[tokio::test]
async fn refused_submissions_leave_no_entry() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let rows = h.queue_rows().await;
    let v = h.version().await;
    let refusals = [
        (
            json!({ "dataCapability": "campagne.inconnue@1", "payload": {} }),
            SubmitError::UnknownCapability,
        ),
        (
            json!({ "dataCapability": "test.modifierPj@2", "target": { "id": pc.to_string() } }),
            SubmitError::UnknownCapability,
        ),
        (
            json!({
                "dataCapability": "test.modifierPj@1", "target": { "id": pc.to_string() },
                "payload": { "level": 4 }, "basedOn": { "version": v + 1 },
            }),
            SubmitError::BasedOnAhead,
        ),
        (
            json!({
                "dataCapability": "test.modifierPj@1", "target": { "id": pc.to_string() },
                "payload": { "level": 4 },
            }),
            SubmitError::Malformed("based-on-required"),
        ),
        (
            // onUpdate: restrict — a PC never changes campaign.
            json!({
                "dataCapability": "test.reglerNiveau@1", "target": { "id": pc.to_string() },
                "payload": { "campagneId": Uuid::new_v4().to_string() },
            }),
            SubmitError::Malformed("field-not-touched"),
        ),
        (
            json!({
                "dataCapability": "test.modifierPj@1", "target": { "id": pc.to_string() },
                "payload": { "archivedAt": "2026-01-01T00:00:00Z" }, "basedOn": { "version": v },
            }),
            SubmitError::Malformed("field-not-touched"),
        ),
        (
            json!({ "dataCapability": "test.reglerNiveau@1", "payload": { "level": 4 } }),
            SubmitError::Malformed("target-required"),
        ),
        (
            json!({
                "dataCapability": "test.reglerNiveau@1", "target": { "id": pc.to_string() },
                "payload": {},
            }),
            SubmitError::Malformed("no-change"),
        ),
        (
            json!({
                "dataCapability": AJOUTER_PJ, "idempotencyKey": "k",
                "payload": { "campagneId": "pas-un-uuid", "name": "Y", "class": "B", "level": 1 },
            }),
            SubmitError::Malformed("relation-id"),
        ),
    ];
    for (s, expected) in refusals {
        assert_eq!(
            h.engine.submit(sub(s.clone())).await.unwrap_err(),
            expected,
            "{s}"
        );
    }
    assert_eq!(h.queue_rows().await, rows);
    assert_eq!(h.version().await, v);
    h.drop_db().await;
}

#[tokio::test]
async fn a_second_applier_applies_nothing() {
    let h = Harness::new().await;
    let first = h.applier().await;
    assert!(h.engine.applier().await.unwrap().is_none());
    drop(first);
    // The lock goes with the connection.
    let mut again = None;
    for _ in 0..50 {
        again = h.engine.applier().await.unwrap();
        if again.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(again.is_some());
    h.drop_db().await;
}

#[tokio::test]
async fn an_infrastructure_failure_rolls_back_and_the_command_waits_its_turn() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    sqlx::raw_sql(
        "CREATE FUNCTION panne() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN RAISE EXCEPTION 'panne'; END $$;
         CREATE TRIGGER panne BEFORE INSERT ON pj FOR EACH ROW EXECUTE FUNCTION panne();",
    )
    .execute(&h.db.pool)
    .await
    .unwrap();
    let version = h.version().await;
    let add = h.add_pc(c, "Ysolde", json!(3)).await;
    let behind = h.add_pc(c, "Brume", json!(3)).await;

    assert!(applier.apply_next().await.is_err());
    assert_eq!(h.state(add.command_id).await, State::Queued);
    assert_eq!(h.state(behind.command_id).await, State::Queued);
    assert_eq!(h.version().await, version);
    let entry = h.lookup(add.command_id).await;
    assert_eq!(entry.entry().unwrap().position, 0);

    sqlx::raw_sql("DROP TRIGGER panne ON pj")
        .execute(&h.db.pool)
        .await
        .unwrap();
    // Once the failure clears and the backoff is over, it applies first, in
    // its turn.
    assert_eq!(applier.drain().await.unwrap(), 0);
    h.clock.advance(Duration::from_secs(1));
    assert_eq!(applier.drain().await.unwrap(), 2);
    assert_eq!(
        h.result(add.command_id).await.data_version,
        Some(version + 1)
    );
    assert_eq!(
        h.result(behind.command_id).await.data_version,
        Some(version + 2)
    );
    h.drop_db().await;
}

/// A failing head stays the head of its scope: nothing of its campaign
/// applies before it, whatever its partition (rules 25 and 28). Two adds with
/// the same name, A then B, apply A then reject B once the failure clears,
/// never the reverse. A command of another campaign is not held back.
#[tokio::test]
async fn a_failing_head_holds_back_its_scope_and_nothing_else() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    sqlx::raw_sql(
        "CREATE FUNCTION panne() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN IF NEW.nom = 'Ysolde' THEN RAISE EXCEPTION 'panne'; END IF; RETURN NEW; END $$;
         CREATE TRIGGER panne BEFORE INSERT ON pj FOR EACH ROW EXECUTE FUNCTION panne();",
    )
    .execute(&h.db.pool)
    .await
    .unwrap();
    let version = h.version().await;
    let first = h.add_pc(c, "Ysolde", json!(3)).await;
    let second = h.add_pc(c, "Ysolde", json!(4)).await;
    let elsewhere = h
        .submit(json!({
            "dataCapability": CREER_CAMPAGNE,
            "payload": { "name": "Ailleurs" },
            "idempotencyKey": Uuid::new_v4().to_string(),
        }))
        .await;
    assert_ne!(first.partition, second.partition);

    // The failing add sets its campaign aside; the other campaign goes on.
    assert!(applier.apply_next().await.is_err());
    assert_eq!(
        applier.apply_next().await.unwrap(),
        Some(elsewhere.command_id)
    );
    assert_eq!(applier.apply_next().await.unwrap(), None);
    assert_eq!(
        h.result(elsewhere.command_id).await.data_version,
        Some(version + 1)
    );

    // Retried after the backoff, it fails again; B never overtakes it.
    h.clock.advance(Duration::from_secs(1));
    assert!(applier.apply_next().await.is_err());
    assert_eq!(applier.apply_next().await.unwrap(), None);
    for s in [&first, &second] {
        assert_eq!(h.state(s.command_id).await, State::Queued);
    }
    assert_eq!(h.version().await, version + 1);

    sqlx::raw_sql("DROP TRIGGER panne ON pj")
        .execute(&h.db.pool)
        .await
        .unwrap();
    h.clock.advance(Duration::from_secs(1));
    assert_eq!(applier.drain().await.unwrap(), 2);
    assert_eq!(
        h.result(first.command_id).await.data_version,
        Some(version + 2)
    );
    let rejected = h.result(second.command_id).await;
    assert_eq!(rejected.status, State::Rejected);
    assert_eq!(rejected.violations, ["pc-name-unique-in-campaign"]);
    h.drop_db().await;
}

/// Every state a command can be seen in, each against its contract.
#[tokio::test]
async fn entries_results_messages_and_plans_conform_to_their_contracts() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;

    let mut seen: Vec<CommandId> = Vec::new();
    let queued = h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let awaiting = h.edit_pc(pc, json!({ "name": "Ysolde la Grise" }), v).await;
    // The pending rename frees "Ysolde" on the projected state; an unknown
    // campaign is a risk whatever is pending.
    let warned = h.add_pc(Uuid::new_v4(), "Brume", json!(2)).await;
    assert_eq!(warned.warnings, ["campaign-active"]);
    let cancelled = h.set_level(pc, 9).await;
    let rejected_now = h.add_pc(c, "", json!(0)).await;
    assert_eq!(queued.lookup.state(), State::Queued);
    assert_eq!(awaiting.lookup.state(), State::AwaitingConfirmation);
    for s in [&queued, &awaiting, &warned, &cancelled, &rejected_now] {
        assert_lookup_valid(&serde_json::to_value(&s.lookup).unwrap());
        seen.push(s.command_id);
    }
    h.engine
        .cancel(cancelled.command_id, GM_IDENTITY)
        .await
        .unwrap();
    applier.drain().await.unwrap();
    // Parked, then requeued.
    let parked = h.lookup(awaiting.command_id).await;
    assert_eq!(parked.state(), State::Parked);
    assert_lookup_valid(&serde_json::to_value(&parked).unwrap());
    let requeued = h
        .engine
        .confirm(awaiting.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(requeued.state(), State::Confirmed);
    assert_lookup_valid(&serde_json::to_value(&requeued).unwrap());
    applier.drain().await.unwrap();

    // An expired one.
    let v = h.version().await;
    h.edit_pc(pc, json!({ "level": 6 }), v).await;
    let expiring = h.edit_pc(pc, json!({ "level": 7 }), v).await;
    applier.drain().await.unwrap();
    h.clock.advance(Duration::from_secs(24 * 3600));
    applier.drain().await.unwrap();
    seen.push(expiring.command_id);

    let mut statuses = Vec::new();
    for id in seen {
        let lookup = h.lookup(id).await;
        let Lookup::Settled { result } = &lookup else {
            panic!("{id} still pending");
        };
        statuses.push(result.status);
        assert_valid(H, &serde_json::to_value(result).unwrap());
    }
    statuses.sort_by_key(|s| s.as_str());
    statuses.dedup();
    assert_eq!(
        statuses,
        [
            State::Applied,
            State::Cancelled,
            State::Expired,
            State::Rejected
        ]
    );

    // Every stored message and impact plan.
    let rows: Vec<(Value, Option<Value>)> =
        sqlx::query_as::<_, (sqlx::types::Json<Value>, Option<sqlx::types::Json<Value>>)>(
            "SELECT messages, impact_plan FROM dataguard_queue",
        )
        .fetch_all(&h.db.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|(m, p)| (m.0, p.map(|p| p.0)))
        .collect();
    let mut kinds = Vec::new();
    for (messages, plan) in rows {
        for m in messages.as_array().unwrap() {
            assert_valid(L, m);
            kinds.push(m["message"].as_str().unwrap().to_owned());
        }
        if let Some(plan) = plan {
            assert_valid(common::contracts::J, &plan);
        }
    }
    kinds.sort();
    kinds.dedup();
    for kind in [
        "CommandApplied",
        "CommandRejected",
        "Expired",
        "InvariantAtRisk",
        "Parked",
        "ValueDeclaredAhead",
    ] {
        assert!(kinds.iter().any(|k| k == kind), "{kind} never stored");
    }
    // The G entry is what the endpoint shows; the stored row is its source.
    let pending = h.set_level(pc, 2).await;
    assert_valid(
        G,
        &serde_json::to_value(pending.lookup.entry().unwrap()).unwrap(),
    );
    h.drop_db().await;
}
