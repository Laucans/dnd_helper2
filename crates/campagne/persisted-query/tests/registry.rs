//! The registry and its files, with no database: contract C, the hashes, the
//! three queries and nothing else, and the executor's source rules.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use campagne_persisted_query::registry_dir;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn registry() -> Value {
    serde_json::from_str(&read(&registry_dir().join("registry.json"))).unwrap()
}

fn entries() -> Vec<Value> {
    registry()["queries"].as_array().unwrap().clone()
}

fn stem(file: &str) -> &str {
    file.strip_suffix(".graphql").expect("a .graphql file")
}

#[test]
fn the_registry_validates_against_contract_c() {
    let schema: Value = serde_json::from_str(&read(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../contracts/c-persisted-query.schema.json"),
    ))
    .unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&registry())
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn the_registry_lists_exactly_the_three_queries() {
    let triples: Vec<(String, String, String)> = entries()
        .iter()
        .map(|e| {
            let get = |k: &str| e[k].as_str().unwrap().to_owned();
            (get("query"), get("capability"), get("file"))
        })
        .collect();
    let expected = [
        (
            "ListeCampagnes",
            "campagne.listerCampagnes",
            "liste-campagnes.graphql",
        ),
        ("ListePjs", "campagne.listerPjs", "liste-pjs.graphql"),
        (
            "NiveauxPjsActifs",
            "campagne.niveauDuGroupe",
            "niveaux-pjs-actifs.graphql",
        ),
    ];
    assert_eq!(
        triples,
        expected.map(|(a, b, c)| (a.to_owned(), b.to_owned(), c.to_owned()))
    );
}

#[test]
fn every_hash_is_the_digest_of_the_graphql_then_the_sql_bytes() {
    for entry in entries() {
        let file = entry["file"].as_str().unwrap();
        let graphql = std::fs::read(registry_dir().join(file)).unwrap();
        let sql = std::fs::read(registry_dir().join(format!("{}.sql", stem(file)))).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&graphql);
        hasher.update(&sql);
        assert_eq!(
            entry["sha256"].as_str().unwrap(),
            hex::encode(hasher.finalize()),
            "{file}"
        );
    }
}

#[test]
fn every_graphql_names_the_operation_and_capability_of_its_entry() {
    for entry in entries() {
        let text = read(&registry_dir().join(entry["file"].as_str().unwrap()));
        let name = entry["query"].as_str().unwrap();
        let capability = entry["capability"].as_str().unwrap();
        let first = text.lines().next().unwrap();
        assert!(
            first.starts_with(&format!("query {name}")),
            "{name}: {first}"
        );
        assert!(
            text.contains(&format!("@capability(id: \"{capability}\")")),
            "{name} does not declare {capability}"
        );
    }
}

#[test]
fn no_query_file_lacks_an_entry_and_no_entry_lacks_its_files() {
    let registered: BTreeSet<String> = entries()
        .iter()
        .map(|e| stem(e["file"].as_str().unwrap()).to_owned())
        .collect();
    let mut on_disk = BTreeSet::new();
    for item in std::fs::read_dir(registry_dir()).unwrap() {
        let path = item.unwrap().path();
        let ext = path.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("graphql" | "sql")) {
            on_disk.insert(path.file_stem().unwrap().to_str().unwrap().to_owned());
        }
    }
    assert_eq!(registered, on_disk);
    for stem in &registered {
        for ext in ["graphql", "sql"] {
            assert!(
                registry_dir().join(format!("{stem}.{ext}")).is_file(),
                "{stem}.{ext}"
            );
        }
    }
}

// --- the executor's source -------------------------------------------------

/// Whether `text` holds `word` as a whole word, ignoring ASCII case.
fn has_word(text: &str, word: &str, ignore_case: bool) -> bool {
    let (haystack, needle) = if ignore_case {
        (text.to_ascii_lowercase(), word.to_ascii_lowercase())
    } else {
        (text.to_owned(), word.to_owned())
    };
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    haystack.match_indices(&needle).any(|(i, m)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + m.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

fn files_in(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            out.extend(files_in(&path, ext));
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
    out
}

#[test]
fn the_source_never_names_the_migrating_variable() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = files_in(&src, "rs");
    assert!(!files.is_empty());
    for file in files {
        assert!(
            !has_word(&read(&file), "DATABASE_URL", false),
            "{} names the migrating variable",
            file.display()
        );
    }
}

#[test]
fn neither_the_source_nor_the_sql_holds_a_write_keyword() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files: Vec<PathBuf> = files_in(&src, "rs")
        .into_iter()
        .chain(files_in(&registry_dir(), "sql"))
        .collect();
    assert!(files.len() >= 7);
    for file in files {
        let text = read(&file);
        for word in [
            "INSERT", "UPDATE", "DELETE", "TRUNCATE", "ALTER", "CREATE", "DROP", "GRANT",
        ] {
            assert!(
                !has_word(&text, word, true),
                "{} holds {word}",
                file.display()
            );
        }
    }
}

#[test]
fn no_query_filters_on_a_tombstone_or_limits() {
    for file in files_in(&registry_dir(), "sql") {
        let text = read(&file);
        assert!(
            !text.contains("archiveLe"),
            "{}: tombstones are the views' job",
            file.display()
        );
        assert!(!has_word(&text, "LIMIT", true), "{}", file.display());
    }
}

#[test]
fn the_crate_depends_on_nothing_of_the_write_side() {
    let manifest = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    for token in ["capabilities/", "dataguard", "dataqueue", "resolver"] {
        assert!(!manifest.contains(token), "Cargo.toml mentions {token}");
    }
    assert!(
        !manifest.contains("path = \".."),
        "no path dependency on another crate"
    );
}
