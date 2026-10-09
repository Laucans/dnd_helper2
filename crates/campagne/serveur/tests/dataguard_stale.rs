//! The `confirm_on_stale` engine, cancel and holds, on a manual clock: no test
//! waits in real time.

mod common;

use std::time::Duration;

use common::test_commands::{Harness, RENOMMER_CAMPAGNE};
use dataguard::{Clock, GM_IDENTITY, HoldError, Lookup, OperationError, State};
use serde_json::json;
use uuid::Uuid;

const DAY: Duration = Duration::from_secs(24 * 3600);

#[tokio::test]
async fn a_second_edit_on_the_same_base_awaits_confirmation_with_your_value() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;

    let first = h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let second = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    assert_eq!(first.lookup.state(), State::Queued);
    let Lookup::Pending { entry, messages } = &second.lookup else {
        panic!("settled");
    };
    assert_eq!(entry.state, State::AwaitingConfirmation);
    assert_eq!(entry.your_value, Some(json!({ "level": 5 })));
    let projection = entry.projection.as_ref().unwrap();
    assert_eq!(
        projection["confirmed"],
        json!({ "name": "Ysolde", "class": "Barde", "level": 3 })
    );
    assert_eq!(
        projection["pendingAhead"][0]["command"],
        json!(first.command_id.to_string())
    );
    assert_eq!(
        projection["pendingAhead"][0]["value"],
        json!({ "level": 4 })
    );
    let declared = messages
        .iter()
        .find(|m| m["message"] == "ValueDeclaredAhead")
        .unwrap();
    assert_eq!(declared["to"], json!([GM_IDENTITY]));
    assert_eq!(
        declared["actions"],
        json!(["confirm_overwrite", "cancel", "edit"])
    );
    assert_eq!(declared["declaredAhead"]["value"], json!({ "level": 4 }));

    // The message names the field the command ahead touches.
    assert_eq!(declared["field"], "PJ.name");
    // Staleness is by declared touches, not payload: a name edit on the same
    // base is stale against a level edit too.
    let third = h.edit_pc(pc, json!({ "name": "Ysolde II" }), v).await;
    assert_eq!(third.lookup.state(), State::AwaitingConfirmation);
    h.drop_db().await;
}

#[tokio::test]
async fn the_message_names_the_field_that_conflicts() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    // Ahead: a level-only command. Ours touches name, class and level.
    h.set_level(pc, 9).await;
    let edit = h.edit_pc(pc, json!({ "name": "Ysolde II" }), v).await;
    let Lookup::Pending { messages, .. } = &edit.lookup else {
        panic!("settled");
    };
    let declared = messages
        .iter()
        .find(|m| m["message"] == "ValueDeclaredAhead")
        .unwrap();
    assert_eq!(declared["field"], "PJ.level");
    h.drop_db().await;
}

#[tokio::test]
async fn confirmed_before_the_head_it_applies_and_overwrites() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let second = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    let confirmed = h
        .engine
        .confirm(second.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(confirmed.state(), State::Confirmed);
    assert_eq!(
        confirmed.entry().unwrap().confirmation,
        Some(json!({ "by": GM_IDENTITY }))
    );
    // It keeps its position.
    assert_eq!(
        confirmed.entry().unwrap().position,
        second.lookup.entry().unwrap().position
    );
    applier.drain().await.unwrap();
    assert_eq!(h.result(second.command_id).await.status, State::Applied);
    assert_eq!(h.pc_row(pc).await.1, 5);
    h.drop_db().await;
}

#[tokio::test]
async fn unconfirmed_at_the_head_it_parks_then_expires_after_a_day() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let second = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    applier.drain().await.unwrap();

    let parked = h.lookup(second.command_id).await;
    let entry = parked.entry().unwrap();
    assert_eq!(entry.state, State::Parked);
    assert_eq!(
        entry.parked,
        Some(json!({ "ttl": "24h", "onExpire": "drop" }))
    );
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

    // A confirm after expiry returns the result; it does not revive it.
    let late = h
        .engine
        .confirm(second.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(late.result(), Some(&r));
    h.drop_db().await;
}

#[tokio::test]
async fn at_exactly_the_ttl_expiry_wins_over_a_confirm() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let second = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    applier.drain().await.unwrap();
    assert_eq!(h.state(second.command_id).await, State::Parked);

    h.clock.advance(DAY);
    let out = h
        .engine
        .confirm(second.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(out.state(), State::Expired);
    assert_eq!(h.pc_row(pc).await.1, 4);
    h.drop_db().await;
}

#[tokio::test]
async fn a_confirmed_parked_command_is_requeued_at_the_end_under_its_own_id() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let second = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    applier.drain().await.unwrap();
    let old_position = h.lookup(second.command_id).await.entry().unwrap().position;
    // Another command queues on the partition while it is parked.
    let other = h.set_level(pc, 8).await;

    h.clock.advance(Duration::from_secs(3600));
    let requeued = h
        .engine
        .confirm(second.command_id, GM_IDENTITY)
        .await
        .unwrap();
    let entry = requeued.entry().unwrap();
    assert_eq!(entry.state, State::Confirmed);
    assert_eq!(entry.command, second.command_id.to_string());
    assert_eq!(entry.requeued_from, Some(second.command_id.to_string()));
    assert!(entry.position > other.lookup.entry().unwrap().position);
    assert!(entry.position > old_position);
    assert_eq!(entry.based_on.version, h.version().await as u64);
    assert_eq!(entry.parked, None);

    applier.drain().await.unwrap();
    // No second confirmation: applied after `other`.
    let r = h.result(second.command_id).await;
    assert_eq!(r.status, State::Applied);
    assert!(r.data_version > h.result(other.command_id).await.data_version);
    assert_eq!(h.pc_row(pc).await.1, 5);
    h.drop_db().await;
}

#[tokio::test]
async fn when_the_earlier_command_is_cancelled_the_stale_one_applies_unasked() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    let first = h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let second = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    assert_eq!(second.lookup.state(), State::AwaitingConfirmation);
    let cancelled = h
        .engine
        .cancel(first.command_id, GM_IDENTITY)
        .await
        .unwrap();
    let r = cancelled.result().unwrap();
    assert_eq!(r.status, State::Cancelled);
    assert_eq!(r.data_version, None);
    applier.drain().await.unwrap();
    assert_eq!(h.result(second.command_id).await.status, State::Applied);
    assert_eq!(h.pc_row(pc).await.1, 5);
    h.drop_db().await;
}

#[tokio::test]
async fn cancel_settles_pending_commands_and_leaves_terminal_ones_alone() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let v = h.version().await;
    h.edit_pc(pc, json!({ "level": 4 }), v).await;
    let parked = h.edit_pc(pc, json!({ "level": 5 }), v).await;
    applier.drain().await.unwrap();
    assert_eq!(h.state(parked.command_id).await, State::Parked);
    let v = h.version().await;
    let queued = h.edit_pc(pc, json!({ "level": 6 }), v).await;
    let awaiting = h.edit_pc(pc, json!({ "level": 7 }), v).await;
    assert_eq!(awaiting.lookup.state(), State::AwaitingConfirmation);
    let rows = h.rows().await;

    for s in [&parked, &queued, &awaiting] {
        let out = h.engine.cancel(s.command_id, GM_IDENTITY).await.unwrap();
        assert_eq!(out.state(), State::Cancelled);
        assert_eq!(out.result().unwrap().data_version, None);
    }
    applier.drain().await.unwrap();
    assert_eq!(h.rows().await, rows);

    // Terminal: the result stands, cancelled or applied.
    let again = h
        .engine
        .cancel(queued.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(again.state(), State::Cancelled);
    let applied = h.set_level(pc, 2).await;
    applier.drain().await.unwrap();
    let before = h.result(applied.command_id).await;
    let after = h
        .engine
        .cancel(applied.command_id, GM_IDENTITY)
        .await
        .unwrap();
    assert_eq!(after.result(), Some(&before));
    h.drop_db().await;
}

#[tokio::test]
async fn only_the_author_confirms_or_cancels() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let v = h.version().await;
    let rename = |name: &str| {
        json!({
            "dataCapability": RENOMMER_CAMPAGNE,
            "target": { "id": c.to_string() },
            "payload": { "name": name },
            "basedOn": { "version": v },
        })
    };
    h.submit(rename("Brumes I")).await;
    let second = h.submit(rename("Brumes II")).await;
    assert_eq!(second.lookup.state(), State::AwaitingConfirmation);
    assert_eq!(
        h.engine.confirm(second.command_id, "someone-else").await,
        Err(OperationError::NotAuthor)
    );
    assert_eq!(
        h.engine.cancel(second.command_id, "someone-else").await,
        Err(OperationError::NotAuthor)
    );
    assert_eq!(
        h.engine
            .cancel(dataguard::CommandId(Uuid::new_v4()), GM_IDENTITY)
            .await,
        Err(OperationError::NotFound)
    );
    assert_eq!(
        h.state(second.command_id).await,
        State::AwaitingConfirmation
    );
    h.drop_db().await;
}

#[tokio::test]
async fn overwrite_and_relative_commands_are_never_stale() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let a = h.set_level(pc, 4).await;
    let b = h.set_level(pc, 5).await;
    assert_eq!(b.lookup.state(), State::Queued);
    let add = h.add_pc(c, "Brume", json!(2)).await;
    assert_eq!(add.lookup.state(), State::Queued);
    applier.drain().await.unwrap();
    for s in [a, b, add] {
        assert_eq!(h.result(s.command_id).await.status, State::Applied);
    }
    h.drop_db().await;
}

#[tokio::test]
async fn a_hold_lasts_a_minute_at_most_and_is_renewed_from_the_renewal() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let partition = format!("PJ/{pc}");
    let start = h.clock.now();

    assert_eq!(
        h.engine
            .hold(&partition, GM_IDENTITY, Duration::from_secs(61))
            .await,
        Err(HoldError::AboveMaxTtl)
    );
    let hold = h
        .engine
        .hold(&partition, GM_IDENTITY, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(hold.expires_at, start + chrono::Duration::seconds(60));

    // A hold never blocks nor delays a command, and consumes no version.
    let version = h.version().await;
    let edit = h.set_level(pc, 9).await;
    assert_eq!(applier.drain().await.unwrap(), 1);
    assert_eq!(
        h.result(edit.command_id).await.data_version,
        Some(version + 1)
    );

    h.clock.advance(Duration::from_secs(30));
    assert_eq!(
        h.engine
            .renew_hold(hold.id, "someone-else", Duration::from_secs(60))
            .await,
        Err(HoldError::NotAuthor)
    );
    assert_eq!(
        h.engine
            .renew_hold(hold.id, GM_IDENTITY, Duration::from_secs(90))
            .await,
        Err(HoldError::AboveMaxTtl)
    );
    let renewed = h
        .engine
        .renew_hold(hold.id, GM_IDENTITY, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(renewed.expires_at, start + chrono::Duration::seconds(90));

    h.clock.advance(Duration::from_secs(60));
    assert_eq!(
        h.engine
            .renew_hold(hold.id, GM_IDENTITY, Duration::from_secs(60))
            .await,
        Err(HoldError::Expired)
    );
    assert_eq!(
        h.engine
            .renew_hold(Uuid::new_v4(), GM_IDENTITY, Duration::from_secs(1))
            .await,
        Err(HoldError::NotFound)
    );
    assert_eq!(
        h.engine
            .hold("Monstre/1", GM_IDENTITY, Duration::from_secs(1))
            .await,
        Err(HoldError::NotFound)
    );
    assert_eq!(h.version().await, version + 1);
    h.drop_db().await;
}

/// Cancel waits on the applier's row lock: a command already inside its
/// applying transaction ends `applied`.
#[tokio::test]
async fn a_cancel_during_the_apply_finds_the_command_applied() {
    use sqlx::Connection;
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let pc = h.pc(&mut applier, c, "Ysolde", 3).await;
    let edit = h.set_level(pc, 9).await;

    // Stand in for the applying transaction: the entry is locked.
    let mut held = sqlx::PgConnection::connect(&h.db.url).await.unwrap();
    let mut tx = held.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM dataguard_queue WHERE command = $1 FOR UPDATE")
        .bind(edit.command_id.0)
        .execute(&mut *tx)
        .await
        .unwrap();
    let engine = h.engine.clone();
    let id = edit.command_id;
    let cancel = tokio::spawn(async move { engine.cancel(id, GM_IDENTITY).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !cancel.is_finished(),
        "cancel waits for the applying transaction"
    );
    sqlx::query(
        "UPDATE dataguard_queue SET state = 'applied', data_version = 99, settled_at = now()
         WHERE command = $1",
    )
    .bind(edit.command_id.0)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let out = cancel.await.unwrap().unwrap();
    assert_eq!(out.state(), State::Applied);
    assert_eq!(out.result().unwrap().data_version, Some(99));
    h.drop_db().await;
}
