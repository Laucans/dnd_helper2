//! The Capability against a scripted read port: input checks, error mapping,
//! and a pass-through of whatever the query returned. No database.

use std::sync::Mutex;

use campagne_lister_pjs::{ListerPjs, ListerPjsError, QueryError, ReadFailure, ReadPort};
use serde_json::{Map, Value, json};

const ID: &str = "11111111-1111-4111-8111-111111111111";

struct FakePort {
    script: Box<dyn Fn() -> Result<Value, QueryError> + Send + Sync>,
    calls: Mutex<Vec<(String, Map<String, Value>)>>,
}

impl FakePort {
    fn answering(answer: Value) -> Self {
        Self::scripted(move || Ok(answer.clone()))
    }

    fn failing(error: impl Fn() -> QueryError + Send + Sync + 'static) -> Self {
        Self::scripted(move || Err(error()))
    }

    fn scripted(script: impl Fn() -> Result<Value, QueryError> + Send + Sync + 'static) -> Self {
        Self {
            script: Box::new(script),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl ReadPort for &FakePort {
    async fn run(&self, query: &str, variables: &Map<String, Value>) -> Result<Value, QueryError> {
        self.calls
            .lock()
            .unwrap()
            .push((query.to_owned(), variables.clone()));
        (self.script)()
    }
}

fn calls(port: &FakePort) -> usize {
    port.calls.lock().unwrap().len()
}

fn row(id: u128, nom: &str, classe: &str, niveau: i32, cree_le: &str) -> Value {
    json!({
        "id": uuid::Uuid::from_u128(id).to_string(),
        "nom": nom, "classe": classe, "niveau": niveau, "creeLe": cree_le,
    })
}

#[tokio::test]
async fn a_malformed_id_is_invalid_input_and_runs_no_query() {
    let port = FakePort::answering(json!({"pjs": []}));
    let lister = ListerPjs::new(&port);
    for bad in [
        "",
        "   ",
        "abc",
        "1234",
        &ID[..ID.len() - 1],
        "11111111-1111-4111-8111-11111111111g",
    ] {
        let err = lister.lister(bad).await.unwrap_err();
        assert!(
            matches!(err, ListerPjsError::InvalidInput),
            "{bad:?}: {err:?}"
        );
    }
    assert_eq!(calls(&port), 0);
}

#[tokio::test]
async fn any_input_but_exactly_a_campaign_id_is_invalid_and_runs_no_query() {
    let port = FakePort::answering(json!({"pjs": []}));
    let lister = ListerPjs::new(&port);
    let inputs = [
        json!({}),
        json!({"campagneId": 42}),
        json!({"campagneId": null}),
        json!({"campagneId": ID, "includeArchived": true}),
        json!({"campagneId": ID, "limit": 5}),
        json!({"campagne": ID}),
        json!([]),
        json!(ID),
        Value::Null,
    ];
    for input in inputs {
        let err = lister.call(&input).await.unwrap_err();
        assert!(
            matches!(err, ListerPjsError::InvalidInput),
            "{input}: {err:?}"
        );
    }
    assert_eq!(calls(&port), 0);
}

#[tokio::test]
async fn a_valid_id_makes_exactly_one_call_with_that_id_alone() {
    let port = FakePort::answering(json!({"pjs": []}));
    ListerPjs::new(&port).lister(ID).await.unwrap();
    let recorded = port.calls.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "ListePjs");
    assert_eq!(
        Value::Object(recorded[0].1.clone()),
        json!({"campagneId": ID})
    );
}

#[tokio::test]
async fn any_accepted_spelling_of_an_id_is_sent_hyphenated() {
    let port = FakePort::answering(json!({"pjs": []}));
    ListerPjs::new(&port)
        .lister("11111111111141118111111111111111")
        .await
        .unwrap();
    assert_eq!(port.calls.lock().unwrap()[0].1["campagneId"], ID);
}

#[tokio::test]
async fn not_found_is_one_answer_with_one_message() {
    let port = FakePort::failing(|| QueryError::NotFound);
    let err = ListerPjs::new(&port).lister(ID).await.unwrap_err();
    assert!(matches!(err, ListerPjsError::NotFound), "{err:?}");
    assert_eq!(err.to_string(), "not found");
}

#[tokio::test]
async fn a_failed_read_is_never_not_found_nor_an_empty_list() {
    let failures: [fn() -> QueryError; 4] = [
        || QueryError::Database(sqlx::Error::PoolTimedOut),
        || QueryError::Tampered {
            query: "ListePjs".into(),
        },
        || QueryError::Unavailable {
            query: "ListePjs".into(),
            reason: "gone".into(),
        },
        || QueryError::Unknown,
    ];
    for failure in failures {
        let port = FakePort::failing(failure);
        let err = ListerPjs::new(&port).lister(ID).await.unwrap_err();
        assert!(
            matches!(err, ListerPjsError::ReadFailed(ReadFailure::Query(_))),
            "{err:?}"
        );
        assert_eq!(err.to_string(), "read failed");
    }
}

#[tokio::test]
async fn an_answer_of_the_wrong_shape_fails_closed() {
    let answers = [
        json!({}),
        json!({"pjs": null}),
        json!({"pjs": [{"id": "x"}]}),
        json!([]),
        json!({"pjs": [{"id": uuid::Uuid::from_u128(1).to_string(), "nom": "A", "classe": "B", "niveau": "3"}]}),
    ];
    for answer in answers {
        let port = FakePort::answering(answer.clone());
        let err = ListerPjs::new(&port).lister(ID).await.unwrap_err();
        assert!(
            matches!(err, ListerPjsError::ReadFailed(ReadFailure::Answer)),
            "{answer}: {err:?}"
        );
    }
}

#[tokio::test]
async fn rows_come_back_in_the_order_given_with_exactly_four_keys() {
    // Deliberately not in creation order: the Capability must not re-sort.
    let port = FakePort::answering(json!({"pjs": [
        row(3, "Cécile", "Magicienne", 5, "2026-03-01T00:00:00Z"),
        row(1, "Aurélien", "Guerrier", 4, "2026-01-01T00:00:00Z"),
        row(2, "Brune", "Rôdeuse", 4, "2026-02-01T00:00:00Z"),
    ]}));
    let lister = ListerPjs::new(&port);

    let out = lister.call(&json!({"campagneId": ID})).await.unwrap();
    let items = out.as_array().unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Cécile", "Aurélien", "Brune"]);
    for item in items {
        let mut keys: Vec<&str> = item
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["class", "id", "level", "name"]);
    }
    assert_eq!(items[0]["class"], "Magicienne");
    assert_eq!(items[0]["level"], 5);
    assert_eq!(items[0]["id"], uuid::Uuid::from_u128(3).to_string());
}

#[tokio::test]
async fn text_is_returned_as_stored() {
    let nfd = "Ele\u{0301}onore";
    let spaced = "  Brune  ";
    let port = FakePort::answering(json!({"pjs": [
        row(1, nfd, "Guerrier", 1, "t"),
        row(2, spaced, " Rôdeuse ", 1, "t"),
    ]}));
    let pjs = ListerPjs::new(&port).lister(ID).await.unwrap();
    assert_eq!(pjs[0].name.as_bytes(), nfd.as_bytes());
    assert_eq!(pjs[1].name, spaced);
    assert_eq!(pjs[1].class, " Rôdeuse ");
}

#[tokio::test]
async fn names_differing_only_by_case_are_not_deduplicated() {
    let port = FakePort::answering(json!({"pjs": [
        row(1, "Brune", "Guerrier", 1, "t"),
        row(2, "brune", "Guerrier", 1, "t"),
    ]}));
    assert_eq!(ListerPjs::new(&port).lister(ID).await.unwrap().len(), 2);
}

#[tokio::test]
async fn no_active_pc_is_an_empty_list() {
    let port = FakePort::answering(json!({"pjs": []}));
    assert!(ListerPjs::new(&port).lister(ID).await.unwrap().is_empty());
    let out = ListerPjs::new(&port)
        .call(&json!({"campagneId": ID}))
        .await
        .unwrap();
    assert_eq!(out, json!([]));
}
