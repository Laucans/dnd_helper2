//! The Capability's own logic, with no database: it maps what the executor
//! returns to `{ id, name }`, in the order received, and a failed or malformed
//! read is an error, never an empty list.

use std::sync::Mutex;

use lister_campagnes::{Campagne, Lecture, ListerError, QUERY, QueryError, lister_campagnes};
use serde_json::{Map, Value, json};

/// A read that answers what the test says and remembers what it was asked.
struct Fake {
    answer: Box<dyn Fn() -> Result<Value, QueryError> + Send + Sync>,
    calls: Mutex<Vec<(String, Map<String, Value>)>>,
}

impl Fake {
    fn answering(value: Value) -> Self {
        Self::with(move || Ok(value.clone()))
    }

    fn failing(error: fn() -> QueryError) -> Self {
        Self::with(move || Err(error()))
    }

    fn with(answer: impl Fn() -> Result<Value, QueryError> + Send + Sync + 'static) -> Self {
        Self {
            answer: Box::new(answer),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl Lecture for Fake {
    async fn run(&self, query: &str, variables: &Map<String, Value>) -> Result<Value, QueryError> {
        self.calls
            .lock()
            .unwrap()
            .push((query.to_owned(), variables.clone()));
        (self.answer)()
    }
}

fn entry(id: &str, name: &str) -> Campagne {
    Campagne {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

#[tokio::test]
async fn the_order_received_is_the_order_returned_and_each_row_is_id_and_name() {
    // Neither by id nor by name nor by date: the Capability does not re-sort.
    let fake = Fake::answering(json!({ "campagnes": [
        { "id": "z-id", "nom": "Brumes", "creeLe": "2026-01-03T00:00:00Z" },
        { "id": "a-id", "nom": "Alpha", "creeLe": "2026-01-09T00:00:00Z" },
        { "id": "m-id", "nom": "Brumes", "creeLe": "2026-01-01T00:00:00Z" },
    ]}));

    let listed = lister_campagnes(&fake).await.unwrap();

    // Two rows share a name: both stay.
    assert_eq!(
        listed,
        [
            entry("z-id", "Brumes"),
            entry("a-id", "Alpha"),
            entry("m-id", "Brumes")
        ]
    );
    let value = serde_json::to_value(&listed).unwrap();
    for item in value.as_array().unwrap() {
        let keys: Vec<&String> = item.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["id", "name"]);
    }
}

#[tokio::test]
async fn the_name_and_the_id_are_returned_as_they_came() {
    let fake = Fake::answering(json!({ "campagnes": [
        { "id": " 0A ", "nom": "  Épée  Noire ", "creeLe": "x" },
    ]}));

    let listed = lister_campagnes(&fake).await.unwrap();

    assert_eq!(listed, [entry(" 0A ", "  Épée  Noire ")]);
}

#[tokio::test]
async fn an_empty_answer_is_an_empty_list() {
    let fake = Fake::answering(json!({ "campagnes": [] }));

    let listed = lister_campagnes(&fake).await.unwrap();

    assert_eq!(serde_json::to_value(listed).unwrap(), json!([]));
}

#[tokio::test]
async fn it_asks_the_registered_query_and_nothing_else_with_no_variable() {
    let fake = Fake::answering(json!({ "campagnes": [] }));

    lister_campagnes(&fake).await.unwrap();

    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, QUERY);
    assert!(calls[0].1.is_empty());
}

#[tokio::test]
async fn a_malformed_answer_is_a_shape_error_never_an_empty_list() {
    let malformed = [
        json!(null),
        json!({}),
        json!({ "campagnes": null }),
        json!({ "campagnes": {} }),
        json!({ "campagnes": [{ "id": 1, "nom": "Alpha" }] }),
        json!({ "campagnes": [{ "id": "a" }] }),
        json!({ "campagnes": [{ "id": "a", "nom": null }] }),
        json!({ "campagnes": [{ "id": "a", "nom": "A" }, "oops"] }),
    ];
    for answer in malformed {
        let fake = Fake::answering(answer.clone());
        let result = lister_campagnes(&fake).await;
        assert!(
            matches!(result, Err(ListerError::Shape)),
            "{answer} gave {result:?}"
        );
    }
}

#[tokio::test]
async fn a_failed_read_is_an_error_whose_text_has_no_connection_detail() {
    let failures: [fn() -> QueryError; 4] = [
        || QueryError::Unknown,
        || QueryError::Tampered {
            query: QUERY.to_owned(),
        },
        || QueryError::NotFound,
        || QueryError::Unavailable {
            query: QUERY.to_owned(),
            reason: "the query file cannot be read".to_owned(),
        },
    ];
    for failure in failures {
        let result = lister_campagnes(&Fake::failing(failure)).await;
        let Err(error @ ListerError::Query(_)) = result else {
            panic!("a failed read must be an error, got {result:?}");
        };
        let text = error.to_string();
        assert!(
            !text.contains("postgres://") && !text.contains('@'),
            "{text}"
        );
    }
}

#[tokio::test]
async fn a_database_error_is_an_error_not_a_list() {
    let fake = Fake::failing(|| QueryError::Database(sqlx::Error::PoolTimedOut));

    let result = lister_campagnes(&fake).await;

    let Err(error) = result else {
        panic!("a database error must not become a list");
    };
    assert_eq!(error.to_string(), "database error");
}
