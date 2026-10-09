//! The manifest against contract B, and against the registry and the query
//! file it names. No database.

use std::path::{Path, PathBuf};

use campagne_lister_pjs::{CAPABILITY, QUERY};
use campagne_persisted_query::registry_dir;
use serde_json::Value;

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn manifest() -> Value {
    serde_json::from_str(&read(&crate_dir().join("capability.json"))).unwrap()
}

fn registry_entry() -> Value {
    let registry: Value =
        serde_json::from_str(&read(&registry_dir().join("registry.json"))).unwrap();
    let entries: Vec<&Value> = registry["queries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["query"] == QUERY)
        .collect();
    assert_eq!(entries.len(), 1, "exactly one registry entry for {QUERY}");
    entries[0].clone()
}

#[test]
fn the_manifest_validates_against_contract_b() {
    let schema: Value = serde_json::from_str(&read(
        &crate_dir().join("../../../../contracts/b-capability-manifest.schema.json"),
    ))
    .unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&manifest())
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_manifest_declares_reads_and_nothing_that_writes_calls_or_computes() {
    let manifest = manifest();
    let object = manifest.as_object().unwrap();
    for forbidden in [
        "implements",
        "cache",
        "invokes",
        "needs",
        "writes",
        "actions",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "{forbidden} must be absent"
        );
    }
    assert_eq!(object["system"], "campagne");
    assert_eq!(object["reads"].as_array().unwrap().len(), 1);
}

#[test]
fn the_identifier_is_the_same_in_the_manifest_the_registry_and_the_query() {
    assert_eq!(manifest()["capability"], CAPABILITY);
    let entry = registry_entry();
    assert_eq!(entry["capability"], CAPABILITY);
    assert_eq!(
        entry["file"],
        manifest()["reads"][0],
        "reads is the registry file, byte for byte"
    );

    let graphql = read(&registry_dir().join(entry["file"].as_str().unwrap()));
    assert!(
        graphql.contains(&format!("@capability(id: \"{CAPABILITY}\")")),
        "{graphql}"
    );
}

#[test]
fn the_query_takes_the_campaign_id_as_its_only_variable() {
    let graphql = read(&registry_dir().join("liste-pjs.graphql"));
    let header = graphql.split('{').next().unwrap();
    let declared = header
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(vars, _)| vars)
        .unwrap();
    let variables: Vec<&str> = declared.split(',').map(str::trim).collect();
    assert_eq!(variables, ["$campagneId: ID!"]);
}
