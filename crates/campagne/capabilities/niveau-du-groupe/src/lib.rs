//! `campagne.niveauDuGroupe`: the party level of a campaign, `NiveauDuGroupe@1`.
//!
//! It reads through one persisted query, `NiveauxPjsActifs`, and computes over
//! exactly the levels that query returns: archived and foreign PCs are the
//! views' to exclude, never this crate's. It connects with the read-only
//! variable `DATABASE_URL_READONLY` only, stores nothing and caches nothing:
//! every call reads, so a read after a command never returns the old level.

mod compute;

pub use campagne_persisted_query::{QueryError, StartError};
pub use compute::{ComputeError, Computed, LEVEL_MAX, LEVEL_MIN, compute};

use campagne_persisted_query::{Executor, registry_dir};
use serde_json::{Map, Value};

/// The identifier a Micro-UI calls this Capability by (`capability.json`).
pub const IDENTIFIER: &str = "campagne.niveauDuGroupe";
/// The persisted query it reads through, and the only one (`registry.json`).
pub const QUERY: &str = "NiveauxPjsActifs";
/// `"v"` followed by the Concept's version.
pub const MODEL: &str = "v1";

/// The output of `NiveauDuGroupe@1`: these four fields and no others.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NiveauDuGroupe {
    /// Serialised as `null` for an empty party, never left out.
    pub level: Option<u8>,
    pub pc_count: u64,
    pub model: &'static str,
    /// The data version the levels were read at.
    pub as_of: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A missing, empty or blank campaign id: refused before any read.
    #[error("campagneId is required")]
    MissingCampaignId,
    /// An unknown, archived, foreign or malformed id: one answer, no payload.
    #[error("not found")]
    NotFound,
    /// A hash mismatch, an unavailable file or a database failure.
    #[error(transparent)]
    Read(QueryError),
    /// The answer lacks the shape the query promises.
    #[error("the read broke its contract: {0}")]
    ContractBreach(&'static str),
    #[error(transparent)]
    Compute(#[from] ComputeError),
}

pub struct Capability {
    executor: Executor,
}

impl Capability {
    /// Connects with `DATABASE_URL_READONLY` and nothing else. If it is
    /// missing the Capability does not start.
    pub async fn from_env() -> Result<Self, StartError> {
        Ok(Self {
            executor: Executor::from_env(&registry_dir()).await?,
        })
    }

    /// The same, from any lookup: tests use it instead of the process
    /// environment.
    pub async fn from_variables(vars: impl Fn(&str) -> Option<String>) -> Result<Self, StartError> {
        Ok(Self {
            executor: Executor::from_variables(&registry_dir(), vars).await?,
        })
    }

    /// The party level of one campaign, read now.
    pub async fn run(&self, campagne_id: &str) -> Result<NiveauDuGroupe, Error> {
        if campagne_id.trim().is_empty() {
            return Err(Error::MissingCampaignId);
        }
        // The id goes out untouched: a malformed one is the executor's
        // not-found, indistinguishable from an unknown one.
        let mut variables = Map::new();
        variables.insert(
            "campagneId".to_owned(),
            Value::String(campagne_id.to_owned()),
        );
        let answer = match self.executor.run(QUERY, &variables).await {
            Ok(answer) => answer,
            Err(QueryError::NotFound) => return Err(Error::NotFound),
            Err(other) => return Err(Error::Read(other)),
        };
        from_answer(&answer)
    }
}

/// The answer of `NiveauxPjsActifs` turned into the output. Apart from `run`
/// so it is tested without a database.
pub fn from_answer(answer: &Value) -> Result<NiveauDuGroupe, Error> {
    let object = answer
        .as_object()
        .ok_or(Error::ContractBreach("the answer is not an object"))?;
    if object.len() != 2 || !object.contains_key("levels") || !object.contains_key("dataVersion") {
        return Err(Error::ContractBreach(
            "the answer is not exactly levels and dataVersion",
        ));
    }
    let levels = object["levels"]
        .as_array()
        .ok_or(Error::ContractBreach("levels is not an array"))?
        .iter()
        .map(|level| {
            level
                .as_i64()
                .ok_or(Error::ContractBreach("a level is not an integer"))
        })
        .collect::<Result<Vec<i64>, Error>>()?;
    // Never defaulted: a version that was not read is an error.
    let as_of = object["dataVersion"].as_u64().ok_or(Error::ContractBreach(
        "dataVersion is not a non-negative integer",
    ))?;
    let Computed { level, pc_count } = compute(&levels)?;
    Ok(NiveauDuGroupe {
        level,
        pc_count,
        model: MODEL,
        as_of,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_two_pc_answer_becomes_the_output() {
        let out = from_answer(&json!({"levels": [3, 4], "dataVersion": 7})).unwrap();
        assert_eq!(
            out,
            NiveauDuGroupe {
                level: Some(4),
                pc_count: 2,
                model: "v1",
                as_of: 7
            }
        );
    }

    #[test]
    fn an_empty_party_is_a_success_with_the_version_it_was_read_at() {
        let out = from_answer(&json!({"levels": [], "dataVersion": 0})).unwrap();
        assert_eq!((out.level, out.pc_count, out.as_of), (None, 0, 0));
    }

    #[test]
    fn a_big_version_is_kept() {
        let out = from_answer(&json!({"levels": [1], "dataVersion": i64::MAX})).unwrap();
        assert_eq!(out.as_of, u64::try_from(i64::MAX).unwrap());
    }

    #[test]
    fn an_answer_off_contract_is_a_breach() {
        let answers = [
            json!({"levels": [3]}),
            json!({"levels": [3], "dataVersion": null}),
            json!({"levels": [3], "dataVersion": -1}),
            json!({"levels": [3], "dataVersion": 1.5}),
            json!({"levels": [3], "dataVersion": "7"}),
            json!({"levels": [5.0], "dataVersion": 1}),
            json!({"levels": ["5"], "dataVersion": 1}),
            json!({"levels": [null], "dataVersion": 1}),
            json!({"levels": null, "dataVersion": 1}),
            json!({"dataVersion": 1}),
            json!({"levels": [3], "dataVersion": 1, "extra": true}),
            json!([3, 4]),
            json!(null),
        ];
        for answer in answers {
            assert!(
                matches!(from_answer(&answer), Err(Error::ContractBreach(_))),
                "{answer}"
            );
        }
    }

    #[test]
    fn a_level_out_of_range_is_a_typed_error_and_no_output() {
        for levels in [json!([21]), json!([0]), json!([-3]), json!([4, 4, 25])] {
            let answer = json!({"levels": levels, "dataVersion": 1});
            assert!(matches!(
                from_answer(&answer),
                Err(Error::Compute(ComputeError::LevelOutOfRange { .. }))
            ));
        }
    }

    #[test]
    fn the_output_has_exactly_four_fields_and_a_null_level_stays_present() {
        let out = from_answer(&json!({"levels": [], "dataVersion": 3})).unwrap();
        let value = serde_json::to_value(out).unwrap();
        assert_eq!(
            value,
            json!({"level": null, "pcCount": 0, "model": "v1", "asOf": 3})
        );
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 4);
        assert!(object["level"].is_null());
    }

    #[test]
    fn no_error_message_quotes_a_level() {
        let error = from_answer(&json!({"levels": [987654], "dataVersion": 1})).unwrap_err();
        assert!(!error.to_string().contains("987654"));
        assert_eq!(Error::NotFound.to_string(), "not found");
    }
}
