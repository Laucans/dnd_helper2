//! `campagne.importerPj@1`: the GM adds to a campaign a PC that was read
//! elsewhere (D&D Beyond): campaign, name, class, level and the id of the
//! character there, all five required. Mode `relative`: the fast path, no
//! review and no confirmation.
//!
//! # What it writes
//!
//! One new `PJ` row, and nothing else. `origin` is `dndbeyond` and comes from
//! this capability, never from the payload: a payload key `origin` is an
//! unknown field here and on `campagne.ajouterPj`. A row written by
//! `ajouterPj` is `manual` with no external id; a row written here always has
//! both `dndbeyond` and its id, and no other pairing exists. Both fields are
//! written once, at insert: no other command can touch them.
//!
//! This crate declares the manifest, maps the payload to the `PJ` fields and
//! checks the shape of `externalId`, nothing else. It does not fetch or parse
//! anything, and no link or URL reaches it: a payload key other than the
//! five is `Malformed("unknown-field")` and nothing is queued. Name, class
//! and level are untrusted whatever their source: trimming, NFC, the length
//! and level ranges and the campaign's activity are the DataGuard's, with the
//! invariants `ajouterPj` already uses. This unit adds no bound of its own and
//! computes no party level.
//!
//! # The shape of `externalId`
//!
//! A canonical decimal id: digits only, first digit non-zero, one to 18
//! digits (it fits an `i64`), compared as exact strings, with no trimming and
//! no normalisation. An absent or `null` id is `Malformed("external-id-required")`,
//! never turned into a null, because a `dndbeyond` row without an id must not
//! exist. Anything else that is not canonical, a JSON number included, is
//! `Malformed("external-id-format")`. These two reasons stay on the server:
//! HTTP renders both as `malformed-submission`.
//!
//! # Already imported
//!
//! The DataGuard result channel carries invariant ids only. "Already
//! imported" is therefore the violation id `pc-external-id-unique-in-campaign`
//! (declared on `PJ`): another active PC of the same campaign holds the same
//! external id. A tombstoned PC does not count, the same id in another
//! campaign is accepted, and a hand-entered PC (no id) never collides. The
//! French label belongs to the layers that show it, not to this unit.
//!
//! `campaign-active` is reported on this command without being listed: it is
//! declared on `Campagne`, and the registry refuses a manifest that lists an
//! invariant its target aggregate (`PJ`) does not declare.
//!
//! # Open conflict, for human review
//!
//! The milestone asks that two PCs with the same name and different external
//! ids both be accepted (its rule 21). That does **not** hold: the
//! DataGuard enforces `pc-name-unique-in-campaign` on every `PJ` insert,
//! whatever a manifest lists, so an import whose name matches an active PC of
//! the campaign, whatever that PC's id or origin, is refused with that id.
//! The owner decided to keep the invariant as written; lifting or carving it
//! out would also change manual entry, and is its own task. A test in the
//! server's `dc_importer_pj` pins the refusal, so a future relaxation is a
//! visible, deliberate change.

use std::sync::Arc;

use dataguard::{Change, DataCapability, DataCapabilityManifest, Malformed, Write};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Contract F.
pub const MANIFEST: &str = include_str!("../data-capability.json");

/// What a submission names it by.
pub const KEY: &str = "campagne.importerPj@1";

/// Payload key → the `PJ` field it writes.
const FIELDS: [(&str, &str); 5] = [
    ("campagneId", "campagneId"),
    ("nom", "name"),
    ("classe", "class"),
    ("niveau", "level"),
    ("externalId", "externalId"),
];

/// The origin this capability writes.
const ORIGIN: &str = "dndbeyond";

/// An id that fits an `i64` in 18 digits.
const EXTERNAL_ID_MAX_DIGITS: usize = 18;

pub struct ImporterPj {
    manifest: DataCapabilityManifest,
}

pub fn capability() -> Result<Arc<dyn DataCapability>, serde_json::Error> {
    Ok(Arc::new(ImporterPj {
        manifest: serde_json::from_str(MANIFEST)?,
    }))
}

/// Digits only, first digit non-zero, one to 18 digits. No trimming.
fn is_canonical_external_id(s: &str) -> bool {
    (1..=EXTERNAL_ID_MAX_DIGITS).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_digit())
        && !s.starts_with('0')
}

impl DataCapability for ImporterPj {
    fn manifest(&self) -> &DataCapabilityManifest {
        &self.manifest
    }

    /// The five fields and the origin. An absent `nom`, `classe` or `niveau`
    /// is written as `null` and refused by the engine with its own invariant
    /// id, never defaulted; values pass untouched. An absent or non-canonical
    /// `externalId` is refused here, before anything is queued. The engine
    /// refuses a target on an insert before this runs, and mints the row's
    /// id itself.
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
        match payload.get("externalId") {
            None | Some(Value::Null) => return Err(Malformed("external-id-required")),
            Some(Value::String(id)) if is_canonical_external_id(id) => {}
            Some(_) => return Err(Malformed("external-id-format")),
        }
        let mut fields: Map<String, Value> = FIELDS
            .iter()
            .map(|(key, field)| {
                let value = payload.get(*key).cloned().unwrap_or(Value::Null);
                (field.to_string(), value)
            })
            .collect();
        fields.insert("origin".into(), Value::String(ORIGIN.into()));
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

    fn payload_with(campaign: &str, external_id: Value) -> Value {
        json!({
            "campagneId": campaign,
            "nom": "Ysolde",
            "classe": "Barde",
            "niveau": 5,
            "externalId": external_id
        })
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
        assert_eq!(m["dataCapability"], "campagne.importerPj");
        assert_eq!(m["owner"], "dataguard");
        assert_eq!(m["version"], 1);
        assert_eq!(m["effect"], "insert");
        assert_eq!(m["target"], json!({ "aggregate": "PJ" }));
        assert_eq!(m["touches"], json!([]));
        assert_eq!(m["mode"], "relative");
        assert_eq!(m["idempotencyKey"], "required");
        assert_eq!(m["callableBy"], json!(["campagne"]));
        assert_eq!(m["permissions"], json!([]));
        let mut keys: Vec<&str> = m["payload"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["campagneId", "classe", "externalId", "niveau", "nom"]
        );
        // The PJ aggregate's own order, which is the order of `violations`.
        assert_eq!(
            m["invariants"],
            json!([
                "pc-name-required",
                "pc-class-required",
                "pc-level-range",
                "pc-name-unique-in-campaign",
                "pc-external-id-unique-in-campaign",
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

    /// The campaign's activity is reported on an import, but it belongs to
    /// `Campagne`: listing it here is a registration error, so it stays out
    /// of the manifest.
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
        // The new invariant is declared on `PJ`, which is what lets it be listed.
        assert!(
            pj.invariants
                .iter()
                .any(|i| i.id == "pc-external-id-unique-in-campaign")
        );

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
    fn maps_french_keys_and_sets_origin_dndbeyond() {
        let c = Uuid::new_v4().to_string();
        assert_eq!(
            write(json!({
                "campagneId": c, "nom": "  Ysolde ", "classe": "Barde", "niveau": 5,
                "externalId": "4242"
            })),
            Ok(insert(json!({
                "campagneId": c, "name": "  Ysolde ", "class": "Barde", "level": 5,
                "externalId": "4242", "origin": "dndbeyond"
            })))
        );
        for level in [json!("5"), json!(5.0)] {
            let mut payload = payload_with(&c, json!("4242"));
            payload["niveau"] = level.clone();
            assert_eq!(
                write(payload),
                Ok(insert(json!({
                    "campagneId": c, "name": "Ysolde", "class": "Barde", "level": level,
                    "externalId": "4242", "origin": "dndbeyond"
                })))
            );
        }
    }

    #[test]
    fn a_missing_value_is_null_never_defaulted() {
        assert_eq!(
            write(json!({ "externalId": "7" })),
            Ok(insert(json!({
                "campagneId": null, "name": null, "class": null, "level": null,
                "externalId": "7", "origin": "dndbeyond"
            })))
        );
    }

    #[test]
    fn a_missing_external_id_is_malformed_never_null() {
        let c = Uuid::new_v4().to_string();
        let mut absent = payload_with(&c, json!("1"));
        absent.as_object_mut().unwrap().remove("externalId");
        assert_eq!(write(absent), Err(Malformed("external-id-required")));
        assert_eq!(
            write(payload_with(&c, json!(null))),
            Err(Malformed("external-id-required"))
        );
        assert_eq!(write(json!({})), Err(Malformed("external-id-required")));
    }

    #[test]
    fn a_non_canonical_external_id_is_malformed() {
        let c = Uuid::new_v4().to_string();
        let nineteen = "1".repeat(19);
        for bad in [
            json!(""),
            json!("0"),
            json!("007"),
            json!(" 12"),
            json!("12 "),
            json!("12.0"),
            json!("-5"),
            json!("+5"),
            json!("abc"),
            json!("1e3"),
            json!("١٢"),
            json!(nineteen),
            json!(12),
            json!(12.0),
            json!(true),
            json!(["12"]),
        ] {
            assert_eq!(
                write(payload_with(&c, bad.clone())),
                Err(Malformed("external-id-format")),
                "{bad}"
            );
        }
        for good in ["1", "12", "4242", &"9".repeat(18)] {
            assert!(write(payload_with(&c, json!(good))).is_ok(), "{good}");
        }
    }

    #[test]
    fn an_unknown_key_is_malformed() {
        let c = Uuid::new_v4().to_string();
        for key in [
            "origin",
            "origine",
            "url",
            "lien",
            "link",
            "character",
            "pjId",
            "id",
            "archivedAt",
            "archiveLe",
            "name",
            "level",
            "by",
        ] {
            let mut payload = payload_with(&c, json!("4242"));
            payload[key] = json!("x");
            assert_eq!(write(payload), Err(Malformed("unknown-field")), "{key}");
        }
        // An unknown key wins over a bad id: nothing about the payload is trusted.
        let mut payload = payload_with(&c, json!("0"));
        payload["origin"] = json!("manual");
        assert_eq!(write(payload), Err(Malformed("unknown-field")));
    }

    #[test]
    fn the_canonical_check_bounds() {
        assert!(is_canonical_external_id("1"));
        assert!(is_canonical_external_id(&"9".repeat(18)));
        assert!(!is_canonical_external_id(&"9".repeat(19)));
        assert!(!is_canonical_external_id(""));
        assert!(!is_canonical_external_id("0"));
        assert!(!is_canonical_external_id("01"));
    }
}
