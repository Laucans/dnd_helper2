//! `campagne.ajouterPj@1`: the GM adds a PC to a campaign, by hand: campaign,
//! name, class and level, all four required. Mode `relative`: the fast path,
//! no review and no confirmation.
//!
//! This crate declares the manifest and maps the payload to the `PJ` fields,
//! nothing else. Trimming, NFC, the length and level ranges, name uniqueness
//! and the campaign's activity are the DataGuard's, checked at enqueue and
//! again when the command applies. The campaign's activity is the one
//! invariant this command reports without listing: `campaign-active` is
//! declared on `Campagne`, and the registry refuses a manifest that lists an
//! invariant its target aggregate (`PJ`) does not declare.

use std::sync::Arc;

use dataguard::{Change, DataCapability, DataCapabilityManifest, Malformed, Write};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Contract F.
pub const MANIFEST: &str = include_str!("../data-capability.json");

/// What a submission names it by.
pub const KEY: &str = "campagne.ajouterPj@1";

/// Payload key → the `PJ` field it writes.
const FIELDS: [(&str, &str); 4] = [
    ("campagneId", "campagneId"),
    ("nom", "name"),
    ("classe", "class"),
    ("niveau", "level"),
];

pub struct AjouterPj {
    manifest: DataCapabilityManifest,
}

pub fn capability() -> Result<Arc<dyn DataCapability>, serde_json::Error> {
    Ok(Arc::new(AjouterPj {
        manifest: serde_json::from_str(MANIFEST)?,
    }))
}

impl DataCapability for AjouterPj {
    fn manifest(&self) -> &DataCapabilityManifest {
        &self.manifest
    }

    /// Always the four fields. An absent key is written as `null` and refused
    /// by the engine with its own invariant id, never defaulted. Values pass
    /// untouched: a `"5"` or a `5.0` reaches the DataGuard as sent. The
    /// engine refuses a target on an insert before this runs, and mints the
    /// row's id itself.
    fn write(
        &self,
        _target: Option<Uuid>,
        payload: &Map<String, Value>,
    ) -> Result<Write, Malformed> {
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
            change: Change::Insert(fields),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataguard::{Aggregates, RegistrationError, Registry};
    use serde_json::json;

    fn write(payload: Value) -> Result<Write, Malformed> {
        let Value::Object(payload) = payload else {
            panic!("not an object");
        };
        capability().unwrap().write(None, &payload)
    }

    fn insert(fields: Value) -> Write {
        let Value::Object(fields) = fields else {
            panic!("not an object");
        };
        Write {
            aggregate: "PJ".into(),
            change: Change::Insert(fields),
        }
    }

    /// A manifest handed to the registry as is, to see what it refuses.
    struct Listed(DataCapabilityManifest);

    impl DataCapability for Listed {
        fn manifest(&self) -> &DataCapabilityManifest {
            &self.0
        }

        fn write(&self, _: Option<Uuid>, _: &Map<String, Value>) -> Result<Write, Malformed> {
            Err(Malformed("unused"))
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
        assert_eq!(m["dataCapability"], "campagne.ajouterPj");
        assert_eq!(m["owner"], "dataguard");
        assert_eq!(m["version"], 1);
        assert_eq!(m["effect"], "insert");
        assert_eq!(m["target"], json!({ "aggregate": "PJ" }));
        assert_eq!(m["touches"], json!([]));
        assert_eq!(m["mode"], "relative");
        assert_eq!(m["idempotencyKey"], "required");
        assert_eq!(m["callableBy"], json!(["campagne"]));
        let mut keys: Vec<&str> = m["payload"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["campagneId", "classe", "niveau", "nom"]);
        // The PJ aggregate's own order, which is the order of `violations`.
        assert_eq!(
            m["invariants"],
            json!([
                "pc-name-required",
                "pc-class-required",
                "pc-level-range",
                "pc-name-unique-in-campaign",
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

    /// The fifth invariant of an add is `campaign-active`. The engine
    /// enforces and reports it on this command, but it belongs to `Campagne`:
    /// listing it here is a registration error, so it stays out of the
    /// manifest.
    #[test]
    fn campaign_active_is_declared_on_campagne_so_not_listed_here() {
        let aggregates = Aggregates::embedded().unwrap();
        let campagne = aggregates.get("Campagne").unwrap();
        assert!(
            campagne
                .invariants
                .iter()
                .any(|i| i.id == "campaign-active")
        );
        let pj = aggregates.get("PJ").unwrap();
        assert!(!pj.invariants.iter().any(|i| i.id == "campaign-active"));

        let mut m: Value = serde_json::from_str(MANIFEST).unwrap();
        m["invariants"]
            .as_array_mut()
            .unwrap()
            .push(json!("campaign-active"));
        let listed = Listed(serde_json::from_value(m).unwrap());
        assert_eq!(
            Registry::empty().register(&aggregates, Arc::new(listed)),
            Err(RegistrationError::UnknownInvariant(
                "campaign-active".into()
            ))
        );
    }

    #[test]
    fn maps_french_keys_to_aggregate_fields_untouched() {
        let c = Uuid::new_v4().to_string();
        assert_eq!(
            write(json!({ "campagneId": c, "nom": "  Ysolde ", "classe": "Barde", "niveau": 5 })),
            Ok(insert(
                json!({ "campagneId": c, "name": "  Ysolde ", "class": "Barde", "level": 5 })
            ))
        );
        for level in [json!("5"), json!(5.0)] {
            assert_eq!(
                write(
                    json!({ "campagneId": c, "nom": "Ysolde", "classe": "Barde", "niveau": level })
                ),
                Ok(insert(
                    json!({ "campagneId": c, "name": "Ysolde", "class": "Barde", "level": level })
                ))
            );
        }
    }

    #[test]
    fn a_missing_value_is_null_never_defaulted() {
        assert_eq!(
            write(json!({})),
            Ok(insert(
                json!({ "campagneId": null, "name": null, "class": null, "level": null })
            ))
        );
        assert_eq!(
            write(json!({ "niveau": 4 })),
            Ok(insert(
                json!({ "campagneId": null, "name": null, "class": null, "level": 4 })
            ))
        );
    }

    #[test]
    fn an_unknown_key_is_malformed() {
        let c = Uuid::new_v4().to_string();
        for key in [
            "pjId",
            "id",
            "archivedAt",
            "archiveLe",
            "name",
            "level",
            "by",
        ] {
            let mut payload =
                json!({ "campagneId": c, "nom": "Ysolde", "classe": "Barde", "niveau": 5 });
            payload[key] = json!("x");
            assert_eq!(write(payload), Err(Malformed("unknown-field")), "{key}");
        }
    }
}
