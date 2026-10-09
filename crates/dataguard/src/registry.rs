//! The DataCapabilities the engine knows. The shipped server registers none
//! ([`Registry::empty`]): every submission to it is an unknown capability.
//! Registration checks a manifest against the aggregates before the engine
//! ever sees a command of it.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::manifest::{Aggregates, DataCapabilityManifest, Effect};
use crate::model::{Malformed, Write};

pub trait DataCapability: Send + Sync + 'static {
    /// Contract F.
    fn manifest(&self) -> &DataCapabilityManifest;

    /// The write a payload asks for, in aggregate field names. `target` is
    /// `None` for an insert, whose id the engine mints.
    fn write(&self, target: Option<Uuid>, payload: &Map<String, Value>)
    -> Result<Write, Malformed>;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistrationError {
    #[error("{0} targets an aggregate no aggregate.json declares")]
    UnknownAggregate(String),
    #[error("touches {0}, which its target aggregate does not declare")]
    UnknownField(String),
    #[error("lists invariant {0}, which its target aggregate does not declare")]
    UnknownInvariant(String),
    #[error("effect delete: no hard delete exists")]
    DeleteEffect,
    #[error("effect {0:?} is not one the engine applies")]
    UnsupportedEffect(Effect),
    #[error("touches {0}, a relation field with onUpdate: restrict")]
    RestrictedField(String),
    #[error("{0} is registered twice")]
    Duplicate(String),
}

#[derive(Default, Clone)]
pub struct Registry {
    by_key: HashMap<String, Arc<dyn DataCapability>>,
}

impl Registry {
    /// What the shipped server uses: no command at all.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        aggregates: &Aggregates,
        cap: Arc<dyn DataCapability>,
    ) -> Result<(), RegistrationError> {
        let m = cap.manifest();
        let target = aggregates
            .get(&m.target.aggregate)
            .ok_or_else(|| RegistrationError::UnknownAggregate(m.target.aggregate.clone()))?;
        match m.effect {
            Effect::Insert | Effect::Update => {}
            Effect::Delete => return Err(RegistrationError::DeleteEffect),
            Effect::Upsert => return Err(RegistrationError::UnsupportedEffect(m.effect)),
        }
        for touched in &m.touches {
            let field = touched
                .split_once('.')
                .filter(|(a, f)| *a == target.aggregate && target.fields.contains_key(*f))
                .map(|(_, f)| f)
                .ok_or_else(|| RegistrationError::UnknownField(touched.clone()))?;
            if aggregates.is_restricted(&target.aggregate, field) {
                return Err(RegistrationError::RestrictedField(touched.clone()));
            }
        }
        for id in &m.invariants {
            if !target.invariants.iter().any(|i| &i.id == id) {
                return Err(RegistrationError::UnknownInvariant(id.clone()));
            }
        }
        let key = m.key();
        if self.by_key.contains_key(&key) {
            return Err(RegistrationError::Duplicate(key));
        }
        self.by_key.insert(key, cap);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&Arc<dyn DataCapability>> {
        self.by_key.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fake(DataCapabilityManifest);

    impl DataCapability for Fake {
        fn manifest(&self) -> &DataCapabilityManifest {
            &self.0
        }
        fn write(&self, _: Option<Uuid>, _: &Map<String, Value>) -> Result<Write, Malformed> {
            Err(Malformed("fake"))
        }
    }

    fn manifest(patch: Value) -> Arc<dyn DataCapability> {
        let mut m = json!({
            "dataCapability": "test.modifierPj",
            "owner": "dataguard",
            "version": 1,
            "effect": "update",
            "target": {"aggregate": "PJ"},
            "touches": ["PJ.level"],
            "payload": {"level": "integer"},
            "mode": "confirm_on_stale",
            "invariants": ["pc-level-range"],
            "permissions": [],
            "callableBy": ["campagne"],
            "idempotencyKey": "optional"
        });
        for (k, v) in patch.as_object().unwrap() {
            m[k] = v.clone();
        }
        Arc::new(Fake(serde_json::from_value(m).unwrap()))
    }

    fn register(patch: Value) -> Result<(), RegistrationError> {
        Registry::empty().register(&Aggregates::embedded().unwrap(), manifest(patch))
    }

    #[test]
    fn a_sound_manifest_registers_once() {
        let a = Aggregates::embedded().unwrap();
        let mut r = Registry::empty();
        assert!(r.is_empty());
        r.register(&a, manifest(json!({}))).unwrap();
        assert!(r.get("test.modifierPj@1").is_some());
        assert!(r.get("test.modifierPj@2").is_none());
        assert_eq!(
            r.register(&a, manifest(json!({}))),
            Err(RegistrationError::Duplicate("test.modifierPj@1".into()))
        );
    }

    #[test]
    fn registration_refuses_what_the_aggregates_do_not_allow() {
        assert_eq!(
            register(json!({"touches": ["PJ.hitPoints"]})),
            Err(RegistrationError::UnknownField("PJ.hitPoints".into()))
        );
        assert_eq!(
            register(json!({"touches": ["Campagne.name"]})),
            Err(RegistrationError::UnknownField("Campagne.name".into()))
        );
        assert_eq!(
            register(json!({"invariants": ["campaign-name-length"]})),
            Err(RegistrationError::UnknownInvariant(
                "campaign-name-length".into()
            ))
        );
        assert_eq!(
            register(json!({"effect": "delete"})),
            Err(RegistrationError::DeleteEffect)
        );
        assert_eq!(
            register(json!({"effect": "upsert"})),
            Err(RegistrationError::UnsupportedEffect(Effect::Upsert))
        );
        assert_eq!(
            register(json!({"touches": ["PJ.level", "PJ.campagneId"]})),
            Err(RegistrationError::RestrictedField("PJ.campagneId".into()))
        );
        assert_eq!(
            register(json!({"target": {"aggregate": "Monstre"}})),
            Err(RegistrationError::UnknownAggregate("Monstre".into()))
        );
    }
}
