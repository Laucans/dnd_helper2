//! The seeded cases of NiveauDuGroupe@1 (`concepts/NiveauDuGroupe/v1`), run
//! against the real views through the executor, on invented rows. Filtering
//! is the views' job: the test asserts what the queries expose, and computes
//! no party level (that is the Capability's).

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{TestDb, campagne, campagne_id, new_id, pj};
use serde_json::{Value, json};
use uuid::Uuid;

fn fixtures() -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../concepts/NiveauDuGroupe/v1/fixtures.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    all["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "seeded")
        .cloned()
        .collect()
}

#[tokio::test]
async fn both_required_seeded_cases_exist() {
    let names: Vec<String> = fixtures()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    for required in ["archived-pc-excluded", "other-campaign-pc-excluded"] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} is not in the fixtures"
        );
    }
}

#[tokio::test]
async fn the_seeded_cases_read_back_through_the_views() {
    for case in fixtures() {
        let name = case["name"].as_str().unwrap();
        let db = TestDb::create().await;
        let exec = db.executor().await;
        let rows = case["input"]["seed"].as_array().unwrap();
        let under_test = case["input"]["campaignUnderTest"].as_str().unwrap();

        // Each symbolic label becomes a real, active campaign.
        let mut campaigns: BTreeMap<String, Uuid> = BTreeMap::new();
        for label in rows
            .iter()
            .map(|r| r["campaignId"].as_str().unwrap())
            .chain([under_test])
        {
            if !campaigns.contains_key(label) {
                let id = campagne(
                    &db.pool,
                    new_id(),
                    &format!("Campagne de test {label}"),
                    "2026-01-01T00:00:00Z",
                    false,
                )
                .await;
                campaigns.insert(label.to_owned(), id);
            }
        }

        // Each row becomes a PC, one second after the previous one.
        let mut active_levels: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        for (i, row) in rows.iter().enumerate() {
            let label = row["campaignId"].as_str().unwrap();
            let level = row["level"].as_i64().unwrap();
            let archived = row["archived"].as_bool().unwrap();
            pj(
                &db.pool,
                new_id(),
                campaigns[label],
                &format!("Joueur {i}"),
                level as i32,
                &format!("2026-02-01T00:00:{i:02}Z"),
                archived,
            )
            .await;
            if !archived {
                active_levels
                    .entry(label.to_owned())
                    .or_default()
                    .push(level);
            }
        }

        // Every campaign sees only its own active PCs.
        for (label, id) in &campaigns {
            let expected = active_levels.get(label).cloned().unwrap_or_default();
            let answer = exec
                .run("NiveauxPjsActifs", &campagne_id(*id))
                .await
                .unwrap();
            let levels: Vec<i64> = answer["levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l.as_i64().unwrap())
                .collect();
            assert_eq!(levels, expected, "{name}: levels of {label}");
            assert_eq!(answer["dataVersion"], json!(0), "{name}");

            let pjs = exec.run("ListePjs", &campagne_id(*id)).await.unwrap();
            let listed: Vec<i64> = pjs["pjs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["niveau"].as_i64().unwrap())
                .collect();
            assert_eq!(listed, expected, "{name}: PCs of {label}");
        }

        // The campaign under test matches the fixture's pcCount, and never
        // carries a level of a PC the fixture marks as excluded.
        let expected_count = case["expected"]["pcCount"].as_u64().unwrap() as usize;
        let answer = exec
            .run("NiveauxPjsActifs", &campagne_id(campaigns[under_test]))
            .await
            .unwrap();
        assert_eq!(
            answer["levels"].as_array().unwrap().len(),
            expected_count,
            "{name}"
        );
        let mut sorted: Vec<i64> = answer["levels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_i64().unwrap())
            .collect();
        sorted.sort_unstable();
        let mut wanted = active_levels.get(under_test).cloned().unwrap_or_default();
        wanted.sort_unstable();
        assert_eq!(sorted, wanted, "{name}");
        for excluded in rows
            .iter()
            .filter(|r| r["archived"] == true || r["campaignId"] != under_test)
        {
            let level = excluded["level"].as_i64().unwrap();
            if !wanted.contains(&level) {
                assert!(
                    !sorted.contains(&level),
                    "{name}: the excluded level {level} leaked"
                );
            }
        }

        db.drop_db().await;
    }
}
