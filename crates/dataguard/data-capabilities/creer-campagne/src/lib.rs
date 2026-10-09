//! `campagne.creerCampagne@1`: the GM creates a campaign, from its name alone.
//!
//! A fast-path insert: it touches no existing field, it is `relative`, and it
//! needs no review. The DataGuard owns the rest, not this crate: it trims and
//! NFC-normalises the name, counts its length in characters, refuses an empty
//! or missing name with `campaign-name-required` and a long one with
//! `campaign-name-length`, mints the campaign's id and replays an
//! `idempotencyKey` it has already seen. A new campaign has no PC.
//!
//! Only `name` leaves this crate: any other payload key (an id, a tombstone)
//! is dropped, and a missing name travels as `null`, never as a default.

use std::sync::Arc;

use dataguard::{Change, DataCapability, DataCapabilityManifest, Malformed, Write};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Contract F.
pub const MANIFEST: &str = include_str!("../data-capability.json");

/// What a submission names it by.
pub const KEY: &str = "campagne.creerCampagne@1";

pub struct CreerCampagne {
    manifest: DataCapabilityManifest,
}

pub fn capability() -> Result<Arc<dyn DataCapability>, serde_json::Error> {
    Ok(Arc::new(CreerCampagne {
        manifest: serde_json::from_str(MANIFEST)?,
    }))
}

impl DataCapability for CreerCampagne {
    fn manifest(&self) -> &DataCapabilityManifest {
        &self.manifest
    }

    /// An insert has no target: its id is the engine's. The name goes through
    /// untouched, so the DataGuard decides on exactly what it will store.
    fn write(
        &self,
        target: Option<Uuid>,
        payload: &Map<String, Value>,
    ) -> Result<Write, Malformed> {
        if target.is_some() {
            return Err(Malformed("target-on-insert"));
        }
        let mut fields = Map::new();
        fields.insert(
            "name".into(),
            payload.get("name").cloned().unwrap_or(Value::Null),
        );
        Ok(Write {
            aggregate: self.manifest.target.aggregate.clone(),
            change: Change::Insert(fields),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataguard::invariants::payload_violations;
    use dataguard::model::{OpKind, Operation};
    use dataguard::{Aggregates, Registry};
    use serde_json::json;

    fn write(target: Option<Uuid>, payload: Value) -> Result<Write, Malformed> {
        let Value::Object(payload) = payload else {
            panic!("not an object");
        };
        capability().unwrap().write(target, &payload)
    }

    /// The violations the DataGuard finds in what `write` hands it.
    fn violations(payload: Value) -> Vec<&'static str> {
        let Ok(Write {
            aggregate,
            change: Change::Insert(fields),
        }) = write(None, payload)
        else {
            panic!("not an insert");
        };
        payload_violations(&Operation {
            aggregate,
            id: Uuid::new_v4(),
            kind: OpKind::Insert,
            fields,
        })
    }

    #[test]
    fn manifest_conforms_to_contract_f() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../../contracts/f-data-capability.schema.json"
        );
        let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let validator = jsonschema::draft202012::new(&schema).unwrap();
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&manifest)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn manifest_is_as_scoped() {
        let m: Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(m["owner"], "dataguard");
        assert_eq!(m["effect"], "insert");
        // No id: the engine mints it.
        assert_eq!(m["target"], json!({ "aggregate": "Campagne" }));
        assert_eq!(m["touches"], json!([]));
        assert_eq!(m["payload"], json!({ "name": "string" }));
        assert_eq!(m["mode"], "relative");
        assert_eq!(
            m["invariants"],
            json!(["campaign-name-required", "campaign-name-length"])
        );
        assert_eq!(m["permissions"], json!(["campagne.write"]));
        assert_eq!(m["idempotencyKey"], "required");
        assert_eq!(m["callableBy"], json!(["campagne"]));
        assert_eq!(capability().unwrap().manifest().key(), KEY);
    }

    /// Registering also proves both invariant ids exist on `Campagne`.
    #[test]
    fn registers_against_the_embedded_aggregates() {
        let mut r = Registry::empty();
        r.register(&Aggregates::embedded().unwrap(), capability().unwrap())
            .unwrap();
        assert!(r.get(KEY).is_some());
    }

    #[test]
    fn writes_an_insert_of_the_name_only() {
        // Untouched: trimming and NFC are the DataGuard's.
        assert_eq!(
            write(None, json!({ "name": "  Les Brumes  " })),
            Ok(Write {
                aggregate: "Campagne".into(),
                change: Change::Insert(
                    json!({ "name": "  Les Brumes  " })
                        .as_object()
                        .unwrap()
                        .clone()
                ),
            })
        );
        // Nothing else leaves: not an id, not a tombstone, not a stray key.
        let w = write(
            None,
            json!({
                "name": "Le Val",
                "id": Uuid::new_v4(),
                "archivedAt": "2026-03-01T20:00:00Z",
                "x": 1,
            }),
        )
        .unwrap();
        assert_eq!(
            w.change,
            Change::Insert(json!({ "name": "Le Val" }).as_object().unwrap().clone())
        );
    }

    #[test]
    fn a_missing_name_is_null_never_defaulted() {
        let w = write(None, json!({})).unwrap();
        assert_eq!(
            w.change,
            Change::Insert(json!({ "name": null }).as_object().unwrap().clone())
        );
    }

    #[test]
    fn a_target_is_malformed() {
        assert_eq!(
            write(Some(Uuid::new_v4()), json!({ "name": "Le Val" })),
            Err(Malformed("target-on-insert"))
        );
    }

    #[test]
    fn campaign_name_required_is_the_only_id_of_an_empty_or_missing_name() {
        for payload in [
            json!({ "name": "" }),
            json!({ "name": "   " }),
            json!({ "name": "\u{00A0}\t" }),
            json!({}),
            json!({ "name": null }),
            json!({ "name": 42 }),
        ] {
            assert_eq!(
                violations(payload.clone()),
                ["campaign-name-required"],
                "{payload}"
            );
        }
    }

    #[test]
    fn campaign_name_length_counts_characters_after_trim() {
        assert_eq!(
            violations(json!({ "name": "é".repeat(101) })),
            ["campaign-name-length"]
        );
        assert_eq!(
            violations(json!({ "name": format!("  {}  ", "a".repeat(101)) })),
            ["campaign-name-length"]
        );
        for name in [
            "é".repeat(100),
            "🐉".repeat(100),
            format!("  {}  ", "a".repeat(100)),
            "Cafe\u{0301}".to_string(),
        ] {
            assert!(violations(json!({ "name": name })).is_empty(), "{name}");
        }
    }
}
