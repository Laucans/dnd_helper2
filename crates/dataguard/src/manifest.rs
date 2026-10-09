//! Contracts I (aggregate) and F (DataCapability) as Rust types, the two
//! embedded aggregates, and the checks the engine runs on them at startup.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The tombstone field: same name on every aggregate, written only by an
/// archive, never cleared.
pub const TOMBSTONE: &str = "archivedAt";

const CAMPAGNE: &str = include_str!("../aggregates/Campagne/aggregate.json");
const PJ: &str = include_str!("../aggregates/PJ/aggregate.json");

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("aggregate manifest {0} does not parse as contract I")]
    Parse(String),
    #[error("aggregate {0} is declared twice")]
    Duplicate(String),
    #[error("relation {0} does not start from a field of its own aggregate")]
    RelationSource(String),
    #[error("relation {0} names no declared aggregate as its target")]
    RelationTarget(String),
    #[error("relation {0}: onDelete must be archive, the only policy the engine applies")]
    OnDeleteNotArchive(String),
    #[error("relation {0} declares no onUpdate")]
    MissingOnUpdate(String),
    #[error("duration {0:?} is not <digits>(ms|s|m|h|d)")]
    Duration(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Derived,
    Authoritative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldSpec {
    pub kind: FieldKind,
    pub criticality: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnViolation {
    Reject,
    HumanReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvariantSpec {
    pub id: String,
    pub rule: String,
    pub on_violation: OnViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnDelete {
    Restrict,
    Cascade,
    Nullify,
    Archive,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnUpdate {
    Propagate,
    Restrict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationSpec {
    pub from: String,
    pub on_delete: OnDelete,
    #[serde(default)]
    pub on_update: Option<OnUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Holds {
    pub max_ttl: String,
    pub renewable: bool,
}

/// Contract I.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AggregateManifest {
    pub aggregate: String,
    pub fields: BTreeMap<String, FieldSpec>,
    pub invariants: Vec<InvariantSpec>,
    pub relations: Vec<RelationSpec>,
    pub holds: Holds,
}

/// A relation resolved against the declared aggregates: rows of `source`
/// point at a row of `target` through `field`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub from: String,
    pub source: String,
    pub field: String,
    pub target: String,
    pub on_delete: OnDelete,
    pub on_update: OnUpdate,
}

/// The aggregates the engine guards, in declaration order: Campagne, then PJ.
#[derive(Debug, Clone)]
pub struct Aggregates {
    list: Vec<AggregateManifest>,
    relations: Vec<Relation>,
}

impl Aggregates {
    /// The two manifests of `aggregates/`, validated.
    pub fn embedded() -> Result<Self, ManifestError> {
        let parse = |name: &str, text: &str| {
            serde_json::from_str::<AggregateManifest>(text)
                .map_err(|_| ManifestError::Parse(name.to_owned()))
        };
        Self::from_manifests(vec![
            parse("Campagne/aggregate.json", CAMPAGNE)?,
            parse("PJ/aggregate.json", PJ)?,
        ])
    }

    /// Refuses a relation it cannot resolve, one that lacks `onUpdate`, and
    /// any `onDelete` but `archive`: a policy is never ignored silently.
    pub fn from_manifests(list: Vec<AggregateManifest>) -> Result<Self, ManifestError> {
        for (i, a) in list.iter().enumerate() {
            if list[..i].iter().any(|b| b.aggregate == a.aggregate) {
                return Err(ManifestError::Duplicate(a.aggregate.clone()));
            }
            parse_duration(&a.holds.max_ttl)?;
        }
        let mut relations = Vec::new();
        for a in &list {
            for r in &a.relations {
                let (source, field) = r
                    .from
                    .split_once('.')
                    .filter(|(s, f)| *s == a.aggregate && a.fields.contains_key(*f))
                    .ok_or_else(|| ManifestError::RelationSource(r.from.clone()))?;
                // Contract I names no target: `<x>Id` points at aggregate `<x>`.
                let target = field
                    .strip_suffix("Id")
                    .and_then(|stem| list.iter().find(|t| t.aggregate.eq_ignore_ascii_case(stem)))
                    .ok_or_else(|| ManifestError::RelationTarget(r.from.clone()))?;
                if r.on_delete != OnDelete::Archive {
                    return Err(ManifestError::OnDeleteNotArchive(r.from.clone()));
                }
                let on_update = r
                    .on_update
                    .ok_or_else(|| ManifestError::MissingOnUpdate(r.from.clone()))?;
                relations.push(Relation {
                    from: r.from.clone(),
                    source: source.to_owned(),
                    field: field.to_owned(),
                    target: target.aggregate.clone(),
                    on_delete: r.on_delete,
                    on_update,
                });
            }
        }
        Ok(Self { list, relations })
    }

    pub fn get(&self, name: &str) -> Option<&AggregateManifest> {
        self.list.iter().find(|a| a.aggregate == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &AggregateManifest> {
        self.list.iter()
    }

    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    /// Every invariant id, Campagne's before PJ's, each in declaration order:
    /// the order violations are reported in.
    pub fn invariant_order(&self) -> Vec<&str> {
        self.list
            .iter()
            .flat_map(|a| a.invariants.iter().map(|i| i.id.as_str()))
            .collect()
    }

    /// `ids` deduplicated and sorted by [`Self::invariant_order`].
    pub fn sort_violations(&self, ids: &[&str]) -> Vec<String> {
        self.invariant_order()
            .into_iter()
            .filter(|id| ids.contains(id))
            .map(str::to_owned)
            .collect()
    }

    /// A field `Aggregate.field` whose relation forbids an update.
    pub fn is_restricted(&self, aggregate: &str, field: &str) -> bool {
        self.relations
            .iter()
            .any(|r| r.source == aggregate && r.field == field && r.on_update == OnUpdate::Restrict)
    }
}

/// `^[0-9]+(ms|s|m|h|d)$`, the contracts' duration.
pub fn parse_duration(s: &str) -> Result<Duration, ManifestError> {
    let bad = || ManifestError::Duration(s.to_owned());
    let split = s.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
    let (digits, unit) = s.split_at(split);
    if digits.is_empty() {
        return Err(bad());
    }
    let n: u64 = digits.parse().map_err(|_| bad())?;
    let ms = match unit {
        "ms" => Some(n),
        "s" => n.checked_mul(1_000),
        "m" => n.checked_mul(60_000),
        "h" => n.checked_mul(3_600_000),
        "d" => n.checked_mul(86_400_000),
        _ => None,
    }
    .ok_or_else(bad)?;
    Ok(Duration::from_millis(ms))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Insert,
    Update,
    Delete,
    Upsert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    ConfirmOnStale,
    Overwrite,
    Relative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyPolicy {
    Required,
    Optional,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityTarget {
    pub aggregate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Contract F.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataCapabilityManifest {
    pub data_capability: String,
    pub owner: String,
    pub version: u32,
    pub effect: Effect,
    pub target: CapabilityTarget,
    pub touches: Vec<String>,
    pub payload: BTreeMap<String, String>,
    pub mode: Mode,
    pub invariants: Vec<String>,
    pub permissions: Vec<String>,
    pub callable_by: Vec<String>,
    pub idempotency_key: KeyPolicy,
}

impl DataCapabilityManifest {
    /// `<system>.<name>@<version>`, the key a submission names it by.
    pub fn key(&self) -> String {
        format!("{}@{}", self.data_capability, self.version)
    }

    /// The fields of its target it touches, without the aggregate prefix.
    pub fn touched_fields(&self) -> Vec<&str> {
        field_names(&self.touches)
    }
}

/// `<Aggregate>.<field>` touches → their field names.
pub fn field_names(touches: &[String]) -> Vec<&str> {
    touches
        .iter()
        .filter_map(|t| t.split_once('.').map(|(_, f)| f))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const INVARIANTS: [&str; 8] = [
        "campaign-name-required",
        "campaign-name-length",
        "campaign-active",
        "pc-name-required",
        "pc-class-required",
        "pc-level-range",
        "pc-name-unique-in-campaign",
        "pc-active",
    ];

    fn crate_dir() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    /// Every `aggregates/*/aggregate.json` on disk, not only the embedded two.
    fn manifests_on_disk() -> Vec<(PathBuf, serde_json::Value)> {
        let mut found: Vec<_> = std::fs::read_dir(crate_dir().join("aggregates"))
            .unwrap()
            .map(|e| e.unwrap().path().join("aggregate.json"))
            .filter(|p| p.is_file())
            .map(|p| {
                let v = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
                (p, v)
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(found.len(), 2, "{found:?}");
        found
    }

    #[test]
    fn every_manifest_validates_against_contract_i() {
        let schema: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(crate_dir().join("../../contracts/i-aggregate.schema.json"))
                .unwrap(),
        )
        .unwrap();
        let validator = jsonschema::draft202012::new(&schema).unwrap();
        for (path, manifest) in manifests_on_disk() {
            let errors: Vec<String> = validator
                .iter_errors(&manifest)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{}: {errors:?}", path.display());
        }
    }

    #[test]
    fn every_field_is_authoritative_and_every_invariant_rejects() {
        for (path, manifest) in manifests_on_disk() {
            let m: AggregateManifest = serde_json::from_value(manifest).unwrap();
            for (name, field) in &m.fields {
                assert_eq!(
                    field.kind,
                    FieldKind::Authoritative,
                    "{}: {name}",
                    path.display()
                );
            }
            for inv in &m.invariants {
                assert_eq!(
                    inv.on_violation,
                    OnViolation::Reject,
                    "{}: {}",
                    path.display(),
                    inv.id
                );
            }
            assert_eq!(m.holds.max_ttl, "1m");
            assert!(m.holds.renewable);
            assert_eq!(
                parse_duration(&m.holds.max_ttl),
                Ok(Duration::from_secs(60))
            );
        }
    }

    #[test]
    fn the_eight_invariants_in_declaration_order() {
        let aggregates = Aggregates::embedded().unwrap();
        assert_eq!(aggregates.invariant_order(), INVARIANTS);
        let campagne = aggregates.get("Campagne").unwrap();
        assert_eq!(
            campagne.fields.keys().collect::<Vec<_>>(),
            ["archivedAt", "name"]
        );
        let pj = aggregates.get("PJ").unwrap();
        assert_eq!(
            pj.fields.keys().collect::<Vec<_>>(),
            ["archivedAt", "campagneId", "class", "level", "name"]
        );
    }

    /// Contract I makes `onUpdate` optional; this test is what requires it.
    #[test]
    fn every_relation_declares_on_delete_and_on_update() {
        let mut seen = 0;
        for (path, manifest) in manifests_on_disk() {
            for relation in manifest["relations"].as_array().unwrap() {
                seen += 1;
                for key in ["onDelete", "onUpdate"] {
                    assert!(
                        relation.get(key).is_some_and(|v| v.is_string()),
                        "{}: {relation} lacks {key}",
                        path.display()
                    );
                }
            }
        }
        assert_eq!(seen, 1);
        let aggregates = Aggregates::embedded().unwrap();
        assert_eq!(
            aggregates.relations(),
            [Relation {
                from: "PJ.campagneId".into(),
                source: "PJ".into(),
                field: "campagneId".into(),
                target: "Campagne".into(),
                on_delete: OnDelete::Archive,
                on_update: OnUpdate::Restrict,
            }]
        );
        assert!(aggregates.is_restricted("PJ", "campagneId"));
        assert!(!aggregates.is_restricted("PJ", "name"));
    }

    fn with_relation(relation: serde_json::Value) -> Result<Aggregates, ManifestError> {
        let mut pj: AggregateManifest = serde_json::from_str(PJ).unwrap();
        pj.relations = vec![serde_json::from_value(relation).unwrap()];
        Aggregates::from_manifests(vec![serde_json::from_str(CAMPAGNE).unwrap(), pj])
    }

    #[test]
    fn startup_refuses_a_policy_it_does_not_apply() {
        for policy in ["cascade", "restrict", "nullify", "review"] {
            assert_eq!(
                with_relation(serde_json::json!(
                    {"from": "PJ.campagneId", "onDelete": policy, "onUpdate": "restrict"}
                ))
                .unwrap_err(),
                ManifestError::OnDeleteNotArchive("PJ.campagneId".into())
            );
        }
        assert_eq!(
            with_relation(serde_json::json!({"from": "PJ.campagneId", "onDelete": "archive"}))
                .unwrap_err(),
            ManifestError::MissingOnUpdate("PJ.campagneId".into())
        );
    }

    #[test]
    fn startup_refuses_a_relation_it_cannot_resolve() {
        assert_eq!(
            with_relation(serde_json::json!(
                {"from": "PJ.level", "onDelete": "archive", "onUpdate": "restrict"}
            ))
            .unwrap_err(),
            ManifestError::RelationTarget("PJ.level".into())
        );
        assert_eq!(
            with_relation(serde_json::json!(
                {"from": "Campagne.name", "onDelete": "archive", "onUpdate": "restrict"}
            ))
            .unwrap_err(),
            ManifestError::RelationSource("Campagne.name".into())
        );
    }

    #[test]
    fn durations_follow_the_contract_pattern() {
        assert_eq!(parse_duration("250ms"), Ok(Duration::from_millis(250)));
        assert_eq!(parse_duration("1m"), Ok(Duration::from_secs(60)));
        assert_eq!(parse_duration("24h"), Ok(Duration::from_secs(86_400)));
        assert_eq!(parse_duration("2d"), Ok(Duration::from_secs(172_800)));
        for bad in ["", "m", "1", "1.5m", "-1m", "1 m", "1M", "1w"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }
}
