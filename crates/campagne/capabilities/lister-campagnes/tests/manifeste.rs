//! The manifest, the registry entry and the crate's sources against the
//! contracts and the architecture gates, with no database. A local
//! `cargo test` catches what `gates.yml` would refuse.

use std::path::{Path, PathBuf};

use campagne_persisted_query::registry_dir;
use lister_campagnes::{CAPABILITY, QUERY};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_dir().join("../../../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&read(path)).unwrap()
}

fn errors(schema: &str, instance: &Value) -> Vec<String> {
    let schema = json(&repo_root().join("contracts").join(schema));
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    validator
        .iter_errors(instance)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect()
}

fn manifest() -> Value {
    json(&crate_dir().join("capability.json"))
}

fn registry_entry() -> Value {
    json(&registry_dir().join("registry.json"))["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["capability"] == CAPABILITY)
        .expect("the registry has an entry for the capability")
        .clone()
}

#[test]
fn the_manifest_validates_against_contract_b() {
    let errors = errors("b-capability-manifest.schema.json", &manifest());
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_manifest_declares_only_what_this_capability_has() {
    let manifest = manifest();
    let object = manifest.as_object().unwrap();
    for key in [
        "implements",
        "input",
        "cache",
        "needs",
        "writes",
        "actions",
        "invokes",
    ] {
        assert!(!object.contains_key(key), "unexpected key {key}");
    }
    assert!(!read(&crate_dir().join("capability.json")).contains("\"invokes\""));
    assert_eq!(manifest["capability"], CAPABILITY);
    assert_eq!(manifest["system"], "campagne");
    assert!(!manifest["description"].as_str().unwrap().is_empty());
}

#[test]
fn the_registry_entry_names_the_query_and_the_graphql_the_capability() {
    let entry = registry_entry();
    assert_eq!(entry["query"], QUERY);
    let graphql = read(&registry_dir().join(entry["file"].as_str().unwrap()));
    assert!(
        graphql.contains(&format!("@capability(id: \"{CAPABILITY}\")")),
        "{graphql}"
    );
    assert!(graphql.contains(&format!("query {QUERY} ")), "{graphql}");
}

#[test]
fn reads_is_the_one_registered_query_file() {
    let manifest = manifest();
    let reads = manifest["reads"].as_array().unwrap();
    assert_eq!(reads.len(), 1);
    let declared = repo_root().join(reads[0].as_str().unwrap());
    let registered = registry_dir().join(registry_entry()["file"].as_str().unwrap());
    assert_eq!(
        declared.canonicalize().unwrap(),
        registered.canonicalize().unwrap()
    );
}

#[test]
fn the_registry_validates_against_contract_c_and_carries_the_real_hash() {
    let registry = json(&registry_dir().join("registry.json"));
    let errors = errors("c-persisted-query.schema.json", &registry);
    assert!(errors.is_empty(), "{errors:?}");

    let entry = registry_entry();
    let file = entry["file"].as_str().unwrap();
    let graphql = std::fs::read(registry_dir().join(file)).unwrap();
    let sql = std::fs::read(registry_dir().join(file.replace(".graphql", ".sql"))).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(&graphql);
    hasher.update(&sql);
    assert_eq!(entry["sha256"], hex::encode(hasher.finalize()));
}

#[test]
fn the_registry_fields_are_the_view_columns_the_query_reads() {
    let entry = registry_entry();
    let fields: Vec<&str> = entry["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(fields.iter().all(|f| f.starts_with("campagne_active.")));
    for field in ["campagne_active.id", "campagne_active.nom"] {
        assert!(fields.contains(&field), "{fields:?}");
    }
}

#[test]
fn cargo_toml_passes_the_isolation_gate() {
    for (n, line) in read(&crate_dir().join("Cargo.toml")).lines().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        for forbidden in ["capabilities/", "dataguard", "dataqueue", "resolver"] {
            assert!(!line.contains(forbidden), "Cargo.toml:{}: {line}", n + 1);
        }
    }
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for item in std::fs::read_dir(dir).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_sources_pass_the_read_only_gate() {
    let mut files = Vec::new();
    sources(&crate_dir().join("src"), &mut files);
    assert!(!files.is_empty());
    for file in files {
        let text = read(&file);
        let words: Vec<String> = text
            .split_whitespace()
            .map(|w| w.to_ascii_lowercase())
            .collect();
        for pair in words.windows(2) {
            let phrase = format!("{} {}", pair[0], pair[1]);
            for forbidden in [
                "insert into",
                "delete from",
                "alter table",
                "drop table",
                "create table",
            ] {
                assert_ne!(phrase, forbidden, "{}", file.display());
            }
        }
        for window in words.windows(3) {
            assert!(
                !(window[0] == "update" && window[2] == "set"),
                "{}: {window:?}",
                file.display()
            );
        }
        assert!(
            !words.iter().any(|w| w.contains("truncate")),
            "{}",
            file.display()
        );
        for (at, _) in text.match_indices("DATABASE_URL") {
            assert!(
                text[at..].starts_with("DATABASE_URL_READONLY"),
                "{}: a bare DATABASE_URL",
                file.display()
            );
        }
    }
}
