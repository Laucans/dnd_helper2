//! The eight invariants of Campagne and PJ, as pure functions of a write and
//! a state.
//!
//! [`payload_violations`] needs the write alone: they reject at enqueue.
//! [`state_violations`] need the rows: at enqueue they run on the projected
//! state and only warn; at application they run on the real state and
//! decide. Invariants are evaluated on the resulting row, so a command never
//! redeclares them.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::manifest::{Aggregates, TOMBSTONE};
use crate::model::{OpKind, Operation};
use crate::text;

pub const CAMPAIGN_NAME_REQUIRED: &str = "campaign-name-required";
pub const CAMPAIGN_NAME_LENGTH: &str = "campaign-name-length";
pub const CAMPAIGN_ACTIVE: &str = "campaign-active";
pub const PC_NAME_REQUIRED: &str = "pc-name-required";
pub const PC_CLASS_REQUIRED: &str = "pc-class-required";
pub const PC_LEVEL_RANGE: &str = "pc-level-range";
pub const PC_NAME_UNIQUE_IN_CAMPAIGN: &str = "pc-name-unique-in-campaign";
pub const PC_ACTIVE: &str = "pc-active";

pub const CAMPAGNE: &str = "Campagne";
pub const PJ: &str = "PJ";

const NAME_MAX: usize = 100;
const CLASS_MAX: usize = 50;
const LEVEL_MIN: i64 = 1;
const LEVEL_MAX: i64 = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct CampaignRow {
    pub id: Uuid,
    pub name: String,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PcRow {
    pub id: Uuid,
    pub campaign: Uuid,
    pub name: String,
    pub class: String,
    pub level: i64,
    pub archived_at: Option<DateTime<Utc>>,
}

/// The rows an invariant can see: one campaign and its PCs, read from the
/// database, or projected by overlaying the commands queued ahead.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub campaigns: HashMap<Uuid, CampaignRow>,
    pub pcs: HashMap<Uuid, PcRow>,
}

impl Snapshot {
    /// Projects `op` as if applied: what a later command will meet if every
    /// command ahead of it applies.
    pub fn apply(&mut self, op: &Operation, at: DateTime<Utc>) {
        let text_of = |fields: &Map<String, Value>, key: &str| {
            fields.get(key).and_then(Value::as_str).map(text::normalize)
        };
        match (op.aggregate.as_str(), op.kind) {
            (CAMPAGNE, OpKind::Insert) => {
                self.campaigns.insert(
                    op.id,
                    CampaignRow {
                        id: op.id,
                        name: text_of(&op.fields, "name").unwrap_or_default(),
                        archived_at: None,
                    },
                );
            }
            (CAMPAGNE, OpKind::Update) => {
                if let (Some(c), Some(name)) =
                    (self.campaigns.get_mut(&op.id), text_of(&op.fields, "name"))
                {
                    c.name = name;
                }
            }
            (CAMPAGNE, OpKind::Archive) => {
                if let Some(c) = self.campaigns.get_mut(&op.id) {
                    c.archived_at.get_or_insert(at);
                }
                for pc in self.pcs.values_mut().filter(|pc| pc.campaign == op.id) {
                    pc.archived_at.get_or_insert(at);
                }
            }
            (PJ, OpKind::Insert) => {
                let campaign = campaign_of(&op.fields).unwrap_or(Uuid::nil());
                self.pcs.insert(
                    op.id,
                    PcRow {
                        id: op.id,
                        campaign,
                        name: text_of(&op.fields, "name").unwrap_or_default(),
                        class: text_of(&op.fields, "class").unwrap_or_default(),
                        level: op.fields.get("level").and_then(Value::as_i64).unwrap_or(0),
                        archived_at: None,
                    },
                );
            }
            (PJ, OpKind::Update) => {
                if let Some(pc) = self.pcs.get_mut(&op.id) {
                    if let Some(name) = text_of(&op.fields, "name") {
                        pc.name = name;
                    }
                    if let Some(class) = text_of(&op.fields, "class") {
                        pc.class = class;
                    }
                    if let Some(level) = op.fields.get("level").and_then(Value::as_i64) {
                        pc.level = level;
                    }
                }
            }
            (PJ, OpKind::Archive) => {
                if let Some(pc) = self.pcs.get_mut(&op.id) {
                    pc.archived_at.get_or_insert(at);
                }
            }
            _ => {}
        }
    }

    /// The row an archive targets exists and already carries its tombstone.
    pub fn is_archived(&self, op: &Operation) -> bool {
        match op.aggregate.as_str() {
            CAMPAGNE => self
                .campaigns
                .get(&op.id)
                .is_some_and(|c| c.archived_at.is_some()),
            PJ => self
                .pcs
                .get(&op.id)
                .is_some_and(|p| p.archived_at.is_some()),
            _ => false,
        }
    }
}

/// `campagneId` of a PJ write, when it is a uuid.
pub fn campaign_of(fields: &Map<String, Value>) -> Option<Uuid> {
    fields
        .get("campagneId")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
}

/// The stored form of a text field, when it is a string.
fn stored_text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(text::normalize)
}

fn text_ok(v: Option<&Value>, max: usize) -> bool {
    stored_text(v).is_some_and(|s| !s.is_empty() && text::len(&s) <= max)
}

/// A JSON integer literal in 1..=20. `5.0`, `1e1`, `"5"`, booleans, null and
/// integers beyond `i64` are not.
pub fn level_ok(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Number(n)) if n.as_i64().is_some_and(|l| (LEVEL_MIN..=LEVEL_MAX).contains(&l)))
}

/// Violations the write alone shows. On an update, an absent field is
/// unchanged and not checked.
pub fn payload_violations(op: &Operation) -> Vec<&'static str> {
    let mut out = Vec::new();
    let f = &op.fields;
    let check = |key: &str| op.kind == OpKind::Insert || f.contains_key(key);
    match (op.aggregate.as_str(), op.kind) {
        (_, OpKind::Archive) => {}
        (CAMPAGNE, _) => {
            if check("name") {
                match stored_text(f.get("name")) {
                    Some(s) if s.is_empty() => out.push(CAMPAIGN_NAME_REQUIRED),
                    None => out.push(CAMPAIGN_NAME_REQUIRED),
                    Some(s) if text::len(&s) > NAME_MAX => out.push(CAMPAIGN_NAME_LENGTH),
                    Some(_) => {}
                }
            }
        }
        (PJ, _) => {
            if check("name") && !text_ok(f.get("name"), NAME_MAX) {
                out.push(PC_NAME_REQUIRED);
            }
            if check("class") && !text_ok(f.get("class"), CLASS_MAX) {
                out.push(PC_CLASS_REQUIRED);
            }
            if check("level") && !level_ok(f.get("level")) {
                out.push(PC_LEVEL_RANGE);
            }
        }
        _ => {}
    }
    out
}

/// Violations that depend on the rows: activity of the campaign and the PC,
/// and name uniqueness among the campaign's active PCs.
pub fn state_violations(op: &Operation, view: &Snapshot) -> Vec<&'static str> {
    let mut out = Vec::new();
    let active_campaign = |id: Uuid| {
        view.campaigns
            .get(&id)
            .is_some_and(|c| c.archived_at.is_none())
    };
    let name_taken = |campaign: Uuid, except: Option<Uuid>| {
        stored_text(op.fields.get("name")).is_some_and(|name| {
            let key = text::name_key(&name);
            view.pcs.values().any(|pc| {
                pc.campaign == campaign
                    && pc.archived_at.is_none()
                    && Some(pc.id) != except
                    && text::name_key(&pc.name) == key
            })
        })
    };
    match (op.aggregate.as_str(), op.kind) {
        (CAMPAGNE, OpKind::Update) if !active_campaign(op.id) => out.push(CAMPAIGN_ACTIVE),
        (CAMPAGNE, OpKind::Archive) if !view.campaigns.contains_key(&op.id) => {
            out.push(CAMPAIGN_ACTIVE)
        }
        (PJ, OpKind::Insert) => match campaign_of(&op.fields) {
            Some(c) if active_campaign(c) => {
                if name_taken(c, None) {
                    out.push(PC_NAME_UNIQUE_IN_CAMPAIGN);
                }
            }
            _ => out.push(CAMPAIGN_ACTIVE),
        },
        (PJ, OpKind::Update) => match view.pcs.get(&op.id) {
            None => out.push(PC_ACTIVE),
            Some(pc) => {
                if !active_campaign(pc.campaign) {
                    out.push(CAMPAIGN_ACTIVE);
                }
                if pc.archived_at.is_some() {
                    out.push(PC_ACTIVE);
                }
                if op.fields.contains_key("name") && name_taken(pc.campaign, Some(pc.id)) {
                    out.push(PC_NAME_UNIQUE_IN_CAMPAIGN);
                }
            }
        },
        (PJ, OpKind::Archive) if !view.pcs.contains_key(&op.id) => out.push(PC_ACTIVE),
        _ => {}
    }
    out
}

/// Every violation of `op` on `view`, in the aggregates' declaration order.
pub fn all_violations(aggregates: &Aggregates, op: &Operation, view: &Snapshot) -> Vec<String> {
    let mut ids = payload_violations(op);
    ids.extend(state_violations(op, view));
    aggregates.sort_violations(&ids)
}

/// Fields an operation may carry: the aggregate's, minus the tombstone (an
/// archive sets it) and minus a relation field on an update (`onUpdate:
/// restrict`).
pub fn writable(aggregates: &Aggregates, aggregate: &str, kind: OpKind, field: &str) -> bool {
    field != TOMBSTONE
        && aggregates
            .get(aggregate)
            .is_some_and(|a| a.fields.contains_key(field))
        && !(kind == OpKind::Update && aggregates.is_restricted(aggregate, field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn aggregates() -> Aggregates {
        Aggregates::embedded().unwrap()
    }

    fn op(aggregate: &str, kind: OpKind, id: Uuid, fields: Value) -> Operation {
        Operation {
            aggregate: aggregate.into(),
            id,
            kind,
            fields: fields.as_object().unwrap().clone(),
        }
    }

    fn add_pc(campaign: Uuid, fields: Value) -> Operation {
        let mut f = fields.as_object().unwrap().clone();
        f.insert("campagneId".into(), json!(campaign.to_string()));
        op(PJ, OpKind::Insert, Uuid::new_v4(), Value::Object(f))
    }

    fn pc_fields(name: Value, class: Value, level: Value) -> Value {
        json!({ "name": name, "class": class, "level": level })
    }

    /// One active campaign `c` with an active PC "Élan", an archived PC
    /// "Brume", and another campaign `other` with an active PC "Sorcha".
    fn world() -> (Snapshot, Uuid, Uuid, Uuid) {
        let mut s = Snapshot::default();
        let (c, other) = (Uuid::new_v4(), Uuid::new_v4());
        for id in [c, other] {
            s.campaigns.insert(
                id,
                CampaignRow {
                    id,
                    name: "Campagne".into(),
                    archived_at: None,
                },
            );
        }
        let elan = Uuid::new_v4();
        let pcs = [
            (elan, c, "Élan", None),
            (Uuid::new_v4(), c, "Brume", Some(Utc::now())),
            (Uuid::new_v4(), other, "Sorcha", None),
        ];
        for (id, campaign, name, archived_at) in pcs {
            s.pcs.insert(
                id,
                PcRow {
                    id,
                    campaign,
                    name: name.into(),
                    class: "Barde".into(),
                    level: 3,
                    archived_at,
                },
            );
        }
        (s, c, other, elan)
    }

    #[test]
    fn pc_level_range_accepts_integer_literals_1_to_20_only() {
        for ok in ["1", "20", "7"] {
            let v: Value = serde_json::from_str(ok).unwrap();
            assert!(
                payload_violations(&add_pc(Uuid::nil(), pc_fields(json!("A"), json!("B"), v)))
                    .is_empty(),
                "{ok}"
            );
        }
        for bad in [
            "0",
            "21",
            "-1",
            "3.5",
            "5.0",
            "1e1",
            "\"5\"",
            "true",
            "null",
            "18446744073709551615",
            "99999999999999999999999",
        ] {
            let v: Value = serde_json::from_str(bad).unwrap();
            assert_eq!(
                payload_violations(&add_pc(Uuid::nil(), pc_fields(json!("A"), json!("B"), v))),
                ["pc-level-range"],
                "{bad}"
            );
        }
        // Missing on create: never defaulted to 1.
        assert_eq!(
            payload_violations(&add_pc(Uuid::nil(), json!({"name": "A", "class": "B"}))),
            ["pc-level-range"]
        );
    }

    #[test]
    fn pc_name_required_and_pc_class_required_bounds() {
        let n100 = "n".repeat(100);
        let n101 = "n".repeat(101);
        let c50 = "é".repeat(50);
        let c51 = "é".repeat(51);
        let ok = |name: &str, class: &str| {
            payload_violations(&add_pc(
                Uuid::nil(),
                pc_fields(json!(name), json!(class), json!(1)),
            ))
        };
        assert!(ok("Y", "B").is_empty());
        assert!(ok(&n100, &c50).is_empty());
        assert!(ok(&format!("  {n100}\u{00A0}"), "B").is_empty());
        assert_eq!(ok(&n101, "B"), ["pc-name-required"]);
        assert_eq!(ok("Y", &c51), ["pc-class-required"]);
        for blank in ["", "   ", "\u{00A0}\u{00A0}", "\t\n"] {
            assert_eq!(ok(blank, "B"), ["pc-name-required"], "{blank:?}");
            assert_eq!(ok("Y", blank), ["pc-class-required"], "{blank:?}");
        }
        for not_text in [json!(5), json!(null), json!(["Y"]), json!(true)] {
            assert_eq!(
                payload_violations(&add_pc(
                    Uuid::nil(),
                    pc_fields(not_text.clone(), not_text.clone(), json!(2))
                )),
                ["pc-name-required", "pc-class-required"]
            );
        }
        assert_eq!(
            payload_violations(&add_pc(Uuid::nil(), json!({"level": 2}))),
            ["pc-name-required", "pc-class-required"]
        );
    }

    #[test]
    fn campaign_name_required_or_campaign_name_length_never_both() {
        let name = |v: Value| {
            payload_violations(&op(
                CAMPAGNE,
                OpKind::Insert,
                Uuid::new_v4(),
                json!({ "name": v }),
            ))
        };
        assert!(name(json!("C")).is_empty());
        assert!(name(json!("é".repeat(100))).is_empty());
        assert_eq!(name(json!("é".repeat(101))), ["campaign-name-length"]);
        for bad in [
            json!(""),
            json!("  "),
            json!("\u{00A0}"),
            json!(3),
            json!(null),
        ] {
            assert_eq!(name(bad.clone()), ["campaign-name-required"], "{bad}");
        }
        assert_eq!(
            payload_violations(&op(CAMPAGNE, OpKind::Insert, Uuid::new_v4(), json!({}))),
            ["campaign-name-required"]
        );
    }

    #[test]
    fn an_update_checks_only_the_fields_it_carries() {
        let id = Uuid::new_v4();
        assert!(payload_violations(&op(PJ, OpKind::Update, id, json!({"level": 4}))).is_empty());
        assert_eq!(
            payload_violations(&op(PJ, OpKind::Update, id, json!({"level": 21}))),
            ["pc-level-range"]
        );
        assert_eq!(
            payload_violations(&op(PJ, OpKind::Update, id, json!({"name": " "}))),
            ["pc-name-required"]
        );
        assert!(payload_violations(&op(CAMPAGNE, OpKind::Update, id, json!({}))).is_empty());
    }

    #[test]
    fn every_violation_is_reported_in_declaration_order() {
        let a = aggregates();
        let (s, c, _, _) = world();
        let pc = add_pc(c, json!({"class": "Barde", "level": 0}));
        assert_eq!(
            all_violations(&a, &pc, &s),
            ["pc-name-required", "pc-level-range"]
        );
        let pc = add_pc(Uuid::new_v4(), json!({"name": "", "class": "", "level": 0}));
        assert_eq!(
            all_violations(&a, &pc, &s),
            [
                "campaign-active",
                "pc-name-required",
                "pc-class-required",
                "pc-level-range"
            ]
        );
    }

    #[test]
    fn pc_name_unique_in_campaign_ignores_case_whitespace_and_form() {
        let (s, c, _, _) = world();
        for same in [
            "Élan",
            "élan",
            "  ÉLAN ",
            "E\u{0301}lan",
            "e\u{0301}lan\u{00A0}",
        ] {
            let pc = add_pc(c, pc_fields(json!(same), json!("Mage"), json!(2)));
            assert_eq!(
                state_violations(&pc, &s),
                ["pc-name-unique-in-campaign"],
                "{same:?}"
            );
        }
        let pc = add_pc(c, pc_fields(json!("Elan"), json!("Mage"), json!(2)));
        assert!(state_violations(&pc, &s).is_empty());
    }

    #[test]
    fn pc_name_unique_in_campaign_skips_archived_and_other_campaigns() {
        let (s, c, other, _) = world();
        // "Brume" is archived in `c`; "Sorcha" lives in `other`.
        for name in ["Brume", "Sorcha"] {
            let pc = add_pc(c, pc_fields(json!(name), json!("Mage"), json!(2)));
            assert!(state_violations(&pc, &s).is_empty(), "{name}");
        }
        let pc = add_pc(other, pc_fields(json!("élan"), json!("Mage"), json!(2)));
        assert!(state_violations(&pc, &s).is_empty());
    }

    #[test]
    fn renaming_a_pc_to_its_own_name_in_another_case_is_valid() {
        let (s, _, _, elan) = world();
        let rename = op(PJ, OpKind::Update, elan, json!({"name": "ÉLAN"}));
        assert!(state_violations(&rename, &s).is_empty());
    }

    #[test]
    fn campaign_active_refuses_unknown_and_archived_campaigns() {
        let (mut s, c, _, elan) = world();
        let unknown = Uuid::new_v4();
        let pc = add_pc(unknown, pc_fields(json!("Y"), json!("B"), json!(1)));
        assert_eq!(state_violations(&pc, &s), ["campaign-active"]);
        let rename = op(CAMPAGNE, OpKind::Update, unknown, json!({"name": "X"}));
        assert_eq!(state_violations(&rename, &s), ["campaign-active"]);
        let archive = op(CAMPAGNE, OpKind::Archive, unknown, json!({}));
        assert_eq!(state_violations(&archive, &s), ["campaign-active"]);

        s.campaigns.get_mut(&c).unwrap().archived_at = Some(Utc::now());
        let pc = add_pc(c, pc_fields(json!("Y"), json!("B"), json!(1)));
        assert_eq!(state_violations(&pc, &s), ["campaign-active"]);
        let rename = op(CAMPAGNE, OpKind::Update, c, json!({"name": "X"}));
        assert_eq!(state_violations(&rename, &s), ["campaign-active"]);
        let edit = op(PJ, OpKind::Update, elan, json!({"level": 4}));
        assert_eq!(state_violations(&edit, &s), ["campaign-active"]);
    }

    #[test]
    fn pc_active_refuses_unknown_and_archived_pcs() {
        let (s, c, _, _) = world();
        let brume = s
            .pcs
            .values()
            .find(|p| p.name == "Brume" && p.campaign == c)
            .unwrap()
            .id;
        let edit = op(PJ, OpKind::Update, brume, json!({"level": 4}));
        assert_eq!(state_violations(&edit, &s), ["pc-active"]);
        let unknown = op(PJ, OpKind::Update, Uuid::new_v4(), json!({"level": 4}));
        assert_eq!(state_violations(&unknown, &s), ["pc-active"]);
        let archive_unknown = op(PJ, OpKind::Archive, Uuid::new_v4(), json!({}));
        assert_eq!(state_violations(&archive_unknown, &s), ["pc-active"]);
        // Archiving an archived PC is the no-op's business, not a violation.
        let archive = op(PJ, OpKind::Archive, brume, json!({}));
        assert!(state_violations(&archive, &s).is_empty());
        assert!(s.is_archived(&archive));
    }

    #[test]
    fn the_projection_applies_the_commands_ahead() {
        let (mut s, c, _, elan) = world();
        let first = add_pc(c, pc_fields(json!("Ysolde"), json!("Barde"), json!(3)));
        s.apply(&first, Utc::now());
        let second = add_pc(c, pc_fields(json!("ysolde"), json!("Mage"), json!(4)));
        assert_eq!(
            state_violations(&second, &s),
            ["pc-name-unique-in-campaign"]
        );

        let at = Utc::now();
        s.apply(&op(CAMPAGNE, OpKind::Archive, c, json!({})), at);
        assert_eq!(s.pcs[&elan].archived_at, Some(at));
        let edit = op(PJ, OpKind::Update, elan, json!({"level": 4}));
        assert_eq!(
            state_violations(&edit, &s),
            ["campaign-active", "pc-active"]
        );
    }

    #[test]
    fn neither_a_tombstone_nor_a_restricted_relation_is_writable() {
        let a = aggregates();
        assert!(writable(&a, PJ, OpKind::Insert, "campagneId"));
        assert!(!writable(&a, PJ, OpKind::Update, "campagneId"));
        assert!(!writable(&a, PJ, OpKind::Update, "archivedAt"));
        assert!(!writable(&a, PJ, OpKind::Update, "id"));
        assert!(writable(&a, PJ, OpKind::Update, "level"));
        assert!(!writable(&a, CAMPAGNE, OpKind::Insert, "level"));
    }
}
