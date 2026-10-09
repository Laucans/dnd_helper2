//! `campagne.modifierPJ@1`: the GM edits a PC's name, class and level, all
//! three at once. Mode `confirm_on_stale`: an edit built on a version another
//! edit of the same PC has since overtaken waits for its author's confirm.
//!
//! This crate declares the manifest and maps the payload to the `PJ` fields,
//! nothing else. Trimming, NFC, the level range, name uniqueness and the
//! PC's activity are the DataGuard's, checked at enqueue and again when the
//! command applies.

use std::sync::Arc;

use dataguard::{Change, DataCapability, DataCapabilityManifest, Malformed, Write};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Contract F.
pub const MANIFEST: &str = include_str!("../data-capability.json");

/// What a submission names it by.
pub const KEY: &str = "campagne.modifierPJ@1";

/// Payload key → the `PJ` field it writes.
const FIELDS: [(&str, &str); 3] = [("nom", "name"), ("classe", "class"), ("niveau", "level")];

pub struct ModifierPj {
    manifest: DataCapabilityManifest,
}

pub fn capability() -> Result<Arc<dyn DataCapability>, serde_json::Error> {
    Ok(Arc::new(ModifierPj {
        manifest: serde_json::from_str(MANIFEST)?,
    }))
}

impl DataCapability for ModifierPj {
    fn manifest(&self) -> &DataCapabilityManifest {
        &self.manifest
    }

    /// Always the three fields. The engine reads an absent field as
    /// unchanged, so a missing value is written as `null` and refused there
    /// with its own invariant id, never defaulted. Values pass untouched: a
    /// `"5"` or a `5.0` reaches the DataGuard as sent.
    fn write(
        &self,
        target: Option<Uuid>,
        payload: &Map<String, Value>,
    ) -> Result<Write, Malformed> {
        let id = target.ok_or(Malformed("target-required"))?;
        if payload
            .keys()
            .any(|k| !FIELDS.iter().any(|(key, _)| key == k))
        {
            return Err(Malformed("unknown-field"));
        }
        let fields = FIELDS
            .iter()
            .map(|(key, field)| {
                let value = payload.get(*key).cloned().unwrap_or(Value::Null);
                (field.to_string(), value)
            })
            .collect();
        Ok(Write {
            aggregate: self.manifest.target.aggregate.clone(),
            change: Change::Update { id, fields },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataguard::{Aggregates, Registry};
    use serde_json::json;

    fn write(target: Option<Uuid>, payload: Value) -> Result<Write, Malformed> {
        let Value::Object(payload) = payload else {
            panic!("not an object");
        };
        capability().unwrap().write(target, &payload)
    }

    fn update(id: Uuid, fields: Value) -> Write {
        let Value::Object(fields) = fields else {
            panic!("not an object");
        };
        Write {
            aggregate: "PJ".into(),
            change: Change::Update { id, fields },
        }
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
        assert_eq!(m["effect"], "update");
        assert_eq!(m["target"], json!({ "aggregate": "PJ", "id": "$pjId" }));
        assert_eq!(m["touches"], json!(["PJ.name", "PJ.class", "PJ.level"]));
        assert_eq!(m["mode"], "confirm_on_stale");
        assert_eq!(m["idempotencyKey"], "required");
        assert_eq!(m["callableBy"], json!(["campagne"]));
        // The PJ aggregate's own order, which is the order of `violations`.
        assert_eq!(
            m["invariants"],
            json!([
                "pc-name-required",
                "pc-class-required",
                "pc-level-range",
                "pc-name-unique-in-campaign",
                "pc-active",
            ])
        );
        assert_eq!(capability().unwrap().manifest().key(), KEY);
    }

    #[test]
    fn registers_against_the_embedded_aggregates() {
        let mut r = Registry::empty();
        r.register(&Aggregates::embedded().unwrap(), capability().unwrap())
            .unwrap();
        assert!(r.get(KEY).is_some());
    }

    #[test]
    fn maps_french_keys_to_aggregate_fields_untouched() {
        let id = Uuid::new_v4();
        assert_eq!(
            write(
                Some(id),
                json!({ "nom": "  Ysolde ", "classe": "Barde", "niveau": 5 })
            ),
            Ok(update(
                id,
                json!({ "name": "  Ysolde ", "class": "Barde", "level": 5 })
            ))
        );
        for level in [json!("5"), json!(5.0)] {
            assert_eq!(
                write(
                    Some(id),
                    json!({ "nom": "Ysolde", "classe": "Barde", "niveau": level })
                ),
                Ok(update(
                    id,
                    json!({ "name": "Ysolde", "class": "Barde", "level": level })
                ))
            );
        }
    }

    #[test]
    fn a_missing_value_is_null_never_unchanged() {
        let id = Uuid::new_v4();
        assert_eq!(
            write(Some(id), json!({})),
            Ok(update(
                id,
                json!({ "name": null, "class": null, "level": null })
            ))
        );
        assert_eq!(
            write(Some(id), json!({ "niveau": 4 })),
            Ok(update(
                id,
                json!({ "name": null, "class": null, "level": 4 })
            ))
        );
    }

    #[test]
    fn an_unknown_key_or_no_target_is_malformed() {
        let id = Uuid::new_v4();
        for key in ["pjId", "archivedAt", "campagneId", "name"] {
            let mut payload = json!({ "nom": "Ysolde", "classe": "Barde", "niveau": 5 });
            payload[key] = json!("x");
            assert_eq!(
                write(Some(id), payload),
                Err(Malformed("unknown-field")),
                "{key}"
            );
        }
        assert_eq!(
            write(
                None,
                json!({ "nom": "Ysolde", "classe": "Barde", "niveau": 5 })
            ),
            Err(Malformed("target-required"))
        );
    }
}
