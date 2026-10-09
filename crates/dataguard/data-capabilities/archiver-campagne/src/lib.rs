//! `campagne.archiverCampagne@1`: the GM archives a campaign. A soft delete:
//! the row stays, its tombstone is set to the instant the command applies.
//!
//! It touches `Campagne.archivedAt` only. Its active PCs are archived in the
//! same transaction by the DataGuard, from the `PJ.campagneId` relation
//! (`onDelete: archive`), not by this crate. Mode `overwrite`: a stale
//! archive never waits for a confirm, and it lists no invariant, since
//! archiving an archived campaign is a no-op and not a violation.

use std::sync::Arc;

use dataguard::{Change, DataCapability, DataCapabilityManifest, Malformed, Write};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Contract F.
pub const MANIFEST: &str = include_str!("../data-capability.json");

/// What a submission names it by.
pub const KEY: &str = "campagne.archiverCampagne@1";

pub struct ArchiverCampagne {
    manifest: DataCapabilityManifest,
}

pub fn capability() -> Result<Arc<dyn DataCapability>, serde_json::Error> {
    Ok(Arc::new(ArchiverCampagne {
        manifest: serde_json::from_str(MANIFEST)?,
    }))
}

impl DataCapability for ArchiverCampagne {
    fn manifest(&self) -> &DataCapabilityManifest {
        &self.manifest
    }

    /// The payload is empty: no command carries `archivedAt`, which the
    /// applier sets from its clock and nothing ever sets back to null.
    fn write(
        &self,
        target: Option<Uuid>,
        payload: &Map<String, Value>,
    ) -> Result<Write, Malformed> {
        let id = target.ok_or(Malformed("target-required"))?;
        if !payload.is_empty() {
            return Err(Malformed("unknown-field"));
        }
        Ok(Write {
            aggregate: self.manifest.target.aggregate.clone(),
            change: Change::Archive { id },
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
        assert_eq!(
            m["target"],
            json!({ "aggregate": "Campagne", "id": "$campagneId" })
        );
        // Nothing of PJ: the cascade is the relation's, not the command's.
        assert_eq!(m["touches"], json!(["Campagne.archivedAt"]));
        assert_eq!(m["payload"], json!({}));
        assert_eq!(m["mode"], "overwrite");
        assert_eq!(m["invariants"], json!([]));
        assert_eq!(m["idempotencyKey"], "required");
        assert_eq!(m["callableBy"], json!(["campagne"]));
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
    fn writes_an_archive_of_its_target() {
        let id = Uuid::new_v4();
        assert_eq!(
            write(Some(id), json!({})),
            Ok(Write {
                aggregate: "Campagne".into(),
                change: Change::Archive { id },
            })
        );
    }

    #[test]
    fn any_payload_key_or_no_target_is_malformed() {
        let id = Uuid::new_v4();
        for payload in [
            json!({ "archivedAt": "2026-03-01T20:00:00Z" }),
            json!({ "archivedAt": null }),
            json!({ "x": 1 }),
        ] {
            assert_eq!(
                write(Some(id), payload.clone()),
                Err(Malformed("unknown-field")),
                "{payload}"
            );
        }
        assert_eq!(write(None, json!({})), Err(Malformed("target-required")));
    }
}
