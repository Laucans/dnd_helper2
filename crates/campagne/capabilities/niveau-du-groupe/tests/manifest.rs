//! The manifest is contract B, names the one registered query it reads, and
//! the crate depends on nothing of the write side.

use std::path::Path;

use campagne_niveau_du_groupe::{IDENTIFIER, QUERY};
use campagne_persisted_query::registry_dir;
use serde_json::Value;

const CRATE_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn manifest() -> Value {
    json(&Path::new(CRATE_DIR).join("capability.json"))
}

#[test]
fn the_manifest_validates_against_contract_b() {
    let schema =
        json(&Path::new(CRATE_DIR).join("../../../../contracts/b-capability-manifest.schema.json"));
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let manifest = manifest();
    let errors: Vec<String> = validator
        .iter_errors(&manifest)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_manifest_says_what_the_crate_is() {
    let manifest = manifest();
    assert_eq!(manifest["capability"], IDENTIFIER);
    assert_eq!(manifest["system"], "campagne");
    assert_eq!(manifest["implements"], "NiveauDuGroupe@1");
    assert_eq!(
        manifest["reads"],
        serde_json::json!(["queries/niveaux-pjs-actifs.graphql"])
    );
    let object = manifest.as_object().unwrap();
    assert!(!object.contains_key("invokes"));
    // No cache in the code, so none in the manifest.
    assert!(!object.contains_key("cache"));
}

#[test]
fn the_read_is_the_registered_query() {
    let manifest = manifest();
    let reads = manifest["reads"].as_array().unwrap();
    assert_eq!(reads.len(), 1);
    let read = reads[0].as_str().unwrap();
    let file_name = Path::new(read).file_name().unwrap().to_str().unwrap();

    let registry = json(&registry_dir().join("registry.json"));
    let mine: Vec<&Value> = registry["queries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["capability"] == IDENTIFIER)
        .collect();
    assert_eq!(mine.len(), 1, "exactly one registered query per capability");
    assert_eq!(mine[0]["query"], QUERY);
    assert_eq!(mine[0]["file"], file_name);

    // `reads` is relative to the system's directory, as in the architecture.
    let system_dir = Path::new(CRATE_DIR).join("../..");
    assert!(system_dir.join(read).is_file(), "{read} does not exist");
}

#[test]
fn the_crate_depends_on_nothing_of_the_write_side() {
    let cargo = std::fs::read_to_string(Path::new(CRATE_DIR).join("Cargo.toml")).unwrap();
    for line in cargo.lines().filter(|l| !l.trim_start().starts_with('#')) {
        for forbidden in ["capabilities/", "dataguard", "dataqueue", "resolver"] {
            assert!(!line.contains(forbidden), "`{line}` names {forbidden}");
        }
    }
}
