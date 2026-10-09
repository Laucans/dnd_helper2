//! The registry of persisted queries (contract C): loading and the static
//! checks that need no file beside `registry.json` itself.

use std::path::{Component, Path};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::StartError;

/// One registered query. Contract C allows nothing else in an entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub query: String,
    pub capability: String,
    pub file: String,
    pub sha256: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    queries: Vec<Entry>,
}

pub(crate) fn load(dir: &Path) -> Result<Vec<Entry>, StartError> {
    let text = std::fs::read_to_string(dir.join("registry.json"))
        .map_err(|_| invalid("registry.json cannot be read"))?;
    parse(&text)
}

pub(crate) fn parse(text: &str) -> Result<Vec<Entry>, StartError> {
    let file: RegistryFile =
        serde_json::from_str(text).map_err(|e| invalid(&format!("registry.json: {e}")))?;
    let mut names: Vec<&str> = file.queries.iter().map(|e| e.query.as_str()).collect();
    names.sort_unstable();
    if let Some(pair) = names.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(invalid(&format!("duplicate query {}", pair[0])));
    }
    for entry in &file.queries {
        check(entry)?;
    }
    Ok(file.queries)
}

fn check(entry: &Entry) -> Result<(), StartError> {
    let name = &entry.query;
    let hex_ok = entry.sha256.len() == 64
        && entry
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !hex_ok {
        return Err(invalid(&format!(
            "query {name}: sha256 is not 64 lowercase hex"
        )));
    }
    if !capability_ok(&entry.capability) {
        return Err(invalid(&format!(
            "query {name}: capability must be <system>.<name>"
        )));
    }
    let path = Path::new(&entry.file);
    let inside = path.components().all(|c| matches!(c, Component::Normal(_)));
    if entry.file.is_empty() || !inside {
        return Err(invalid(&format!(
            "query {name}: file must be a relative path inside the registry directory"
        )));
    }
    if path.extension().and_then(|e| e.to_str()) != Some("graphql") {
        return Err(invalid(&format!("query {name}: file must end in .graphql")));
    }
    if entry.fields.is_empty() || entry.fields.iter().any(|f| f.contains('*')) {
        return Err(invalid(&format!(
            "query {name}: fields must be explicit, with no wildcard"
        )));
    }
    Ok(())
}

/// Contract C's pattern: `^[a-z][a-z0-9-]*\.[A-Za-z][A-Za-z0-9]*$`.
fn capability_ok(id: &str) -> bool {
    let Some((system, name)) = id.split_once('.') else {
        return false;
    };
    system.starts_with(|c: char| c.is_ascii_lowercase())
        && system
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn invalid(reason: &str) -> StartError {
    StartError::Registry(reason.to_owned())
}

/// The registered digest of a query: the `.graphql` bytes, then the `.sql`
/// bytes, as read, with no normalisation.
pub(crate) fn digest(graphql: &[u8], sql: &[u8]) -> String {
    hex::encode(
        Sha256::new()
            .chain_update(graphql)
            .chain_update(sql)
            .finalize(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn entry(query: &str, file: &str, sha: &str) -> String {
        format!(
            r#"{{"query":"{query}","capability":"campagne.x","file":"{file}","sha256":"{sha}","fields":["v.a"]}}"#
        )
    }

    fn registry(entries: &[String]) -> String {
        format!(r#"{{"queries":[{}]}}"#, entries.join(","))
    }

    fn rejected(text: &str) -> bool {
        matches!(parse(text), Err(StartError::Registry(_)))
    }

    #[test]
    fn accepts_a_well_formed_registry() {
        let text = registry(&[
            entry("A", "a.graphql", HASH),
            entry("B", "sub/b.graphql", HASH),
        ]);
        assert_eq!(parse(&text).unwrap().len(), 2);
    }

    #[test]
    fn refuses_a_duplicate_name() {
        let text = registry(&[entry("A", "a.graphql", HASH), entry("A", "b.graphql", HASH)]);
        assert!(rejected(&text));
    }

    #[test]
    fn refuses_a_file_that_leaves_the_directory() {
        assert!(rejected(&registry(&[entry("A", "../x.graphql", HASH)])));
        assert!(rejected(&registry(&[entry("A", "/abs.graphql", HASH)])));
        assert!(rejected(&registry(&[entry(
            "A",
            "a/../../x.graphql",
            HASH
        )])));
    }

    #[test]
    fn refuses_a_capability_outside_contract_c() {
        for bad in [
            "listerPjs",
            "campagne.lister-pjs",
            "Campagne.listerPjs",
            "campagne.",
        ] {
            let text = registry(&[entry("A", "a.graphql", HASH)]).replace("campagne.x", bad);
            assert!(rejected(&text), "{bad}");
        }
    }

    #[test]
    fn refuses_a_file_that_is_not_graphql() {
        assert!(rejected(&registry(&[entry("A", "a.sql", HASH)])));
    }

    #[test]
    fn refuses_an_unknown_key() {
        let text = r#"{"queries":[{"query":"A","capability":"c.x","file":"a.graphql","sha256":"0000000000000000000000000000000000000000000000000000000000000000","fields":["v.a"],"extra":1}]}"#;
        assert!(rejected(text));
        assert!(rejected(r#"{"queries":[],"extra":1}"#));
    }

    #[test]
    fn refuses_uppercase_or_short_hashes() {
        let upper = HASH.replace('0', "A");
        assert!(rejected(&registry(&[entry("A", "a.graphql", &upper)])));
        assert!(rejected(&registry(&[entry("A", "a.graphql", "abc")])));
    }

    #[test]
    fn refuses_a_wildcard_or_empty_fields() {
        let star = r#"{"queries":[{"query":"A","capability":"c.x","file":"a.graphql","sha256":"0000000000000000000000000000000000000000000000000000000000000000","fields":["v.*"]}]}"#;
        assert!(rejected(star));
        let empty = star.replace(r#"["v.*"]"#, "[]");
        assert!(rejected(&empty));
    }

    #[test]
    fn the_digest_covers_both_files_in_order() {
        assert_ne!(digest(b"a", b"b"), digest(b"b", b"a"));
        assert_eq!(digest(b"ab", b""), digest(b"a", b"b"));
        assert_ne!(digest(b"a\n", b"b"), digest(b"a", b"b"));
    }
}
