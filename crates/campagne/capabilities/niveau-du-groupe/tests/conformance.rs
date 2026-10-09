//! Rule 28: every formula fixture of the Concept runs against this crate's own
//! computation. The `semantic-conformance` gate is off, so this test is what
//! enforces it. It iterates over the file: a case added later runs unchanged,
//! and a case it cannot read fails the test, it is never skipped.
//!
//! The two `seeded` cases belong to the read views (their test is in the
//! persisted-query crate): they are recognised by their shape and counted.

use campagne_niveau_du_groupe::{MODEL, compute};
use serde_json::Value;

const CONCEPT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../concepts/NiveauDuGroupe/v1"
);

fn read(file: &str) -> Value {
    let path = format!("{CONCEPT_DIR}/{file}");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {path}: {e}"))
}

#[test]
fn every_formula_fixture_matches_the_computation() {
    let fixtures = read("fixtures.json");
    assert_eq!(fixtures["concept"], "NiveauDuGroupe");
    assert_eq!(fixtures["version"], 1);
    let cases = fixtures["cases"].as_array().expect("cases is an array");
    assert!(!cases.is_empty(), "the fixture file has no case");

    let mut failures: Vec<String> = Vec::new();
    let mut formula = 0usize;
    let mut delegated = 0usize;
    let mut seen_levels: Vec<Vec<i64>> = Vec::new();

    for case in cases {
        let name = case["name"]
            .as_str()
            .unwrap_or_else(|| panic!("a case has no name: {case}"));
        match case["kind"].as_str() {
            Some("formula") => {
                let levels: Vec<i64> = case["input"]["levels"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{name}: input.levels is not an array"))
                    .iter()
                    .map(|l| {
                        l.as_i64()
                            .unwrap_or_else(|| panic!("{name}: a level is not an integer"))
                    })
                    .collect();
                let expected = case["expected"]
                    .as_object()
                    .unwrap_or_else(|| panic!("{name}: expected is not an object"));
                let expected_pc_count = expected
                    .get("pcCount")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| panic!("{name}: expected.pcCount is not an integer"));
                let expected_level = match expected.get("level") {
                    Some(Value::Null) => None,
                    Some(v) => Some(
                        v.as_u64()
                            .unwrap_or_else(|| panic!("{name}: expected.level is not an integer")),
                    ),
                    None => panic!("{name}: expected.level is missing"),
                };
                formula += 1;
                match compute(&levels) {
                    Ok(got)
                        if got.level.map(u64::from) == expected_level
                            && got.pc_count == expected_pc_count => {}
                    other => failures.push(format!(
                        "{name}: {levels:?} expected {expected_level:?}/{expected_pc_count}, got {other:?}"
                    )),
                }
                seen_levels.push(levels);
            }
            Some("seeded") => {
                assert!(
                    case["input"]["campaignUnderTest"].is_string()
                        && case["input"]["seed"].is_array(),
                    "{name}: a seeded case needs input.campaignUnderTest and input.seed"
                );
                delegated += 1;
            }
            other => panic!("{name}: unknown fixture kind {other:?}"),
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {formula} formula fixtures differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // The file must keep the cases the milestone asks for (rule 27).
    assert!(seen_levels.iter().any(Vec::is_empty), "no empty-party case");
    assert!(seen_levels.iter().any(|l| l.len() == 1), "no one-PC case");
    assert!(
        seen_levels.iter().any(|l| l.contains(&1)),
        "no level-1 case"
    );
    assert!(
        seen_levels.iter().any(|l| l.contains(&20)),
        "no level-20 case"
    );
    assert_eq!(formula + delegated, cases.len());
    println!("{formula} formula fixtures run, {delegated} delegated to the views");
}

#[test]
fn model_is_the_concept_version() {
    let concept = read("concept.json");
    assert_eq!(concept["concept"], "NiveauDuGroupe");
    let version = concept["version"].as_u64().expect("version is an integer");
    assert_eq!(MODEL, format!("v{version}"));
    assert_eq!(concept["output"]["model"]["const"], MODEL);
}
