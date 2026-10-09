//! The contracts of `contracts/`, as validators: a stored entry, a result, a
//! message or an impact plan that does not validate fails the test.

use serde_json::Value;

fn schema(file: &str) -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts/");
    serde_json::from_str(&std::fs::read_to_string(format!("{path}{file}")).unwrap()).unwrap()
}

/// Panics, naming the contract and every error, when `value` does not
/// validate against `contracts/<file>`.
pub fn assert_valid(file: &str, value: &Value) {
    let validator = jsonschema::draft202012::new(&schema(file)).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{file}: {errors:?}\n{value:#}");
}

pub const G: &str = "g-queue-entry.schema.json";
pub const H: &str = "h-command-result.schema.json";
pub const J: &str = "j-impact-plan.schema.json";
pub const L: &str = "l-message.schema.json";

/// A lookup as the result endpoint answers it: each part against its contract.
pub fn assert_lookup_valid(lookup: &Value) {
    if let Some(entry) = lookup.get("entry") {
        assert_valid(G, entry);
        for m in lookup["messages"].as_array().unwrap() {
            assert_valid(L, m);
        }
    } else {
        assert_valid(H, &lookup["result"]);
    }
}
