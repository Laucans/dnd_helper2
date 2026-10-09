//! The Capability against the read view, on invented rows: the view hides the
//! archived campaigns and the query orders the rest; the Capability only maps.

mod common;

use common::{TestDb, campagne, id};
use lister_campagnes::{Campagne, lister_campagnes};
use serde_json::json;

fn entry(id: uuid::Uuid, name: &str) -> Campagne {
    Campagne {
        id: id.to_string(),
        name: name.to_owned(),
    }
}

#[tokio::test]
async fn an_archived_campaign_is_absent() {
    let db = TestDb::create().await;
    let a = campagne(&db.pool, id(1), "Alpha", "2026-01-01T00:00:00Z", false).await;
    campagne(&db.pool, id(2), "Archivée", "2026-01-02T00:00:00Z", true).await;
    let c = campagne(&db.pool, id(3), "Gamma", "2026-01-03T00:00:00Z", false).await;

    let listed = lister_campagnes(&db.executor().await).await.unwrap();

    assert_eq!(listed, [entry(c, "Gamma"), entry(a, "Alpha")]);
    db.drop_db().await;
}

#[tokio::test]
async fn the_newest_comes_first() {
    let db = TestDb::create().await;
    let a = campagne(&db.pool, id(1), "Alpha", "2026-01-01T00:00:00Z", false).await;
    let b = campagne(&db.pool, id(2), "Bêta", "2026-01-02T00:00:00Z", false).await;
    let c = campagne(&db.pool, id(3), "Alpha", "2026-01-03T00:00:00Z", false).await;

    let listed = lister_campagnes(&db.executor().await).await.unwrap();

    assert_eq!(
        listed,
        [entry(c, "Alpha"), entry(b, "Bêta"), entry(a, "Alpha")]
    );
    db.drop_db().await;
}

#[tokio::test]
async fn the_order_is_by_creation_never_by_name() {
    let db = TestDb::create().await;
    let zed = campagne(&db.pool, id(1), "Zèbre", "2026-01-01T00:00:00Z", false).await;
    let abe = campagne(&db.pool, id(2), "Abeille", "2026-01-02T00:00:00Z", false).await;

    let listed = lister_campagnes(&db.executor().await).await.unwrap();

    assert_eq!(listed, [entry(abe, "Abeille"), entry(zed, "Zèbre")]);
    db.drop_db().await;
}

#[tokio::test]
async fn equal_creation_instants_are_ordered_by_ascending_id_on_every_call() {
    let db = TestDb::create().await;
    // Seeded in reverse of the expected order.
    let high = campagne(&db.pool, id(2), "Deux", "2026-01-01T00:00:00Z", false).await;
    let low = campagne(&db.pool, id(1), "Un", "2026-01-01T00:00:00Z", false).await;
    let executor = db.executor().await;

    let first = lister_campagnes(&executor).await.unwrap();
    let second = lister_campagnes(&executor).await.unwrap();

    assert_eq!(first, [entry(low, "Un"), entry(high, "Deux")]);
    assert_eq!(second, first);
    db.drop_db().await;
}

#[tokio::test]
async fn two_campaigns_with_one_name_both_appear_with_their_own_id() {
    let db = TestDb::create().await;
    let first = campagne(&db.pool, id(1), "Brumes", "2026-01-01T00:00:00Z", false).await;
    let second = campagne(&db.pool, id(2), "Brumes", "2026-01-02T00:00:00Z", false).await;

    let listed = lister_campagnes(&db.executor().await).await.unwrap();

    assert_eq!(listed, [entry(second, "Brumes"), entry(first, "Brumes")]);
    assert_ne!(listed[0].id, listed[1].id);
    db.drop_db().await;
}

#[tokio::test]
async fn an_empty_database_gives_an_empty_list_not_an_error_nor_null() {
    let db = TestDb::create().await;

    let listed = lister_campagnes(&db.executor().await).await.unwrap();

    assert!(listed.is_empty());
    assert_eq!(serde_json::to_value(&listed).unwrap(), json!([]));
    db.drop_db().await;
}

#[tokio::test]
async fn only_archived_campaigns_give_an_empty_list() {
    let db = TestDb::create().await;
    for n in 1..=3 {
        campagne(&db.pool, id(n), "Archivée", "2026-01-01T00:00:00Z", true).await;
    }

    let listed = lister_campagnes(&db.executor().await).await.unwrap();

    assert_eq!(serde_json::to_value(&listed).unwrap(), json!([]));
    db.drop_db().await;
}

#[tokio::test]
async fn each_entry_has_exactly_the_fields_id_and_name_and_the_name_as_stored() {
    let db = TestDb::create().await;
    // Already NFC, with a composed accent: it must come back byte for byte.
    let stored = "L\u{e9}gende d\u{2019}\u{c9}lorn";
    let a = campagne(&db.pool, id(1), stored, "2026-01-01T00:00:00Z", false).await;

    let listed = lister_campagnes(&db.executor().await).await.unwrap();

    let value = serde_json::to_value(&listed).unwrap();
    assert_eq!(value, json!([{ "id": a.to_string(), "name": stored }]));
    let keys: Vec<&String> = value[0].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "name"]);
    assert_eq!(listed[0].name.as_bytes(), stored.as_bytes());
    db.drop_db().await;
}

#[tokio::test]
async fn an_archived_campaign_is_gone_from_the_next_call() {
    let db = TestDb::create().await;
    let a = campagne(&db.pool, id(1), "Alpha", "2026-01-01T00:00:00Z", false).await;
    let b = campagne(&db.pool, id(2), "Bêta", "2026-01-02T00:00:00Z", false).await;
    let executor = db.executor().await;
    assert_eq!(
        lister_campagnes(&executor).await.unwrap(),
        [entry(b, "Bêta"), entry(a, "Alpha")]
    );

    // The test seeds as the migrating role; the Capability never writes.
    sqlx::query(r#"UPDATE campagne SET "archiveLe" = now() WHERE id = $1"#)
        .bind(b)
        .execute(&db.pool)
        .await
        .unwrap();

    assert_eq!(
        lister_campagnes(&executor).await.unwrap(),
        [entry(a, "Alpha")]
    );
    db.drop_db().await;
}

#[tokio::test]
async fn a_campaign_created_after_a_call_is_at_the_top_of_the_next() {
    let db = TestDb::create().await;
    let a = campagne(&db.pool, id(1), "Alpha", "2026-01-01T00:00:00Z", false).await;
    let executor = db.executor().await;
    assert_eq!(
        lister_campagnes(&executor).await.unwrap(),
        [entry(a, "Alpha")]
    );

    let b = campagne(&db.pool, id(2), "Bêta", "2026-01-02T00:00:00Z", false).await;

    assert_eq!(
        lister_campagnes(&executor).await.unwrap(),
        [entry(b, "Bêta"), entry(a, "Alpha")]
    );
    db.drop_db().await;
}
