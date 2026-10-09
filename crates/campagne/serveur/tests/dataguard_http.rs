//! The engine's three routes: submission answers at once, the result route
//! shows the entry then the result, and `/data-version` pushes each applied
//! version — not a rejection, not a no-op.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use campagne_serveur::http::{self, AppState};
use common::contracts::assert_lookup_valid;
use common::test_commands::{AJOUTER_PJ, ARCHIVER_PJ, CREER_CAMPAGNE, Harness};
use dataguard::{GM_IDENTITY, Registry, VersionFeed};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::sync::watch;
use tower::ServiceExt;
use uuid::Uuid;

async fn app(h: &Harness) -> (Router, watch::Receiver<i64>) {
    let versions = VersionFeed::spawn(h.db.pool.clone(), std::future::pending())
        .await
        .unwrap();
    let app = http::app(AppState {
        engine: Arc::new(h.engine.clone()),
        versions: versions.clone(),
    });
    (app, versions)
}

/// Status and body, and the body checked for the connection string.
async fn call(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    let body = match body {
        Some(b) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    for secret in [common::database_url(), common::readonly_url()] {
        assert!(!text.contains(&secret), "a connection string leaked");
    }
    (status, serde_json::from_str(&text).unwrap())
}

#[tokio::test]
async fn the_shipped_registry_knows_no_command() {
    let h = Harness::with_registry(Registry::empty()).await;
    let (app, versions) = app(&h).await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/commands",
        Some(json!({ "dataCapability": CREER_CAMPAGNE, "payload": { "name": "X" }, "idempotencyKey": "k" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({ "error": "unknown-capability" }));
    assert_eq!(h.queue_rows().await, 0);
    assert_eq!(h.version().await, 0);
    drop((app, versions));
    h.drop_db().await;
}

#[tokio::test]
async fn submission_answers_at_once_and_the_result_route_follows_the_command() {
    let h = Harness::new().await;
    let (app, versions) = app(&h).await;

    // No applier runs: the answer never waits for one.
    let (status, submitted) = tokio::time::timeout(
        Duration::from_secs(2),
        call(
            &app,
            Method::POST,
            "/commands",
            Some(json!({
                "dataCapability": CREER_CAMPAGNE,
                "payload": { "name": "Les Brumes" },
                "idempotencyKey": "k1",
                "by": "intrus",
            })),
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(submitted["replayed"], false);
    assert_eq!(submitted["warnings"], json!([]));
    assert_eq!(submitted["entry"]["state"], "queued");
    assert_eq!(
        submitted["entry"]["by"], GM_IDENTITY,
        "the caller's by is ignored"
    );
    assert!(
        submitted["partition"]
            .as_str()
            .unwrap()
            .starts_with("Campagne/")
    );
    let id = submitted["commandId"].as_str().unwrap().to_owned();

    let (status, pending) = call(&app, Method::GET, &format!("/commands/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(pending.get("result").is_none());
    assert_eq!(pending["entry"]["command"], id);
    assert_lookup_valid(&pending);

    h.applier().await.drain().await.unwrap();
    let (status, settled) = call(&app, Method::GET, &format!("/commands/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(settled.get("entry").is_none());
    assert_eq!(
        settled["result"],
        json!({ "commandId": id, "status": "applied", "dataVersion": 1, "violations": [], "reviewId": null })
    );
    assert_lookup_valid(&settled);

    for path in [
        format!("/commands/{}", Uuid::new_v4()),
        "/commands/pas-un-id".into(),
    ] {
        let (status, body) = call(&app, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "not-found" }));
    }
    drop((app, versions));
    h.drop_db().await;
}

#[tokio::test]
async fn refusals_answer_a_stable_id_and_never_a_server_message() {
    let h = Harness::new().await;
    let (app, versions) = app(&h).await;
    for (body, id) in [
        (json!("not an object"), "malformed-submission"),
        (json!({ "payload": {} }), "malformed-submission"),
        (
            json!({ "dataCapability": CREER_CAMPAGNE, "payload": { "name": "X" } }),
            "idempotency-key-required",
        ),
        (
            json!({ "dataCapability": "test.reglerNiveau@1", "target": { "id": Uuid::new_v4() },
                    "payload": { "level": 3 }, "basedOn": { "version": 7 } }),
            "based-on-ahead",
        ),
    ] {
        let (status, answer) = call(&app, Method::POST, "/commands", Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer, json!({ "error": id }));
    }

    // A database error that quotes the database answers `internal`, nothing else.
    sqlx::raw_sql(
        "CREATE FUNCTION panne() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN RAISE EXCEPTION 'panne sur %', current_database(); END $$;
         CREATE TRIGGER panne BEFORE INSERT ON dataguard_queue
           FOR EACH ROW EXECUTE FUNCTION panne();",
    )
    .execute(&h.db.pool)
    .await
    .unwrap();
    let (status, answer) = call(
        &app,
        Method::POST,
        "/commands",
        Some(json!({ "dataCapability": CREER_CAMPAGNE, "payload": { "name": "X" }, "idempotencyKey": "k" })),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(answer, json!({ "error": "internal" }));
    assert!(!answer.to_string().contains(&h.db.name));
    drop((app, versions));
    h.drop_db().await;
}

/// The next `dataVersion` event of an SSE body, if one comes within `wait`.
async fn next_version(body: &mut Body, wait: Duration) -> Option<i64> {
    let deadline = tokio::time::Instant::now() + wait;
    let mut text = String::new();
    loop {
        let frame = tokio::time::timeout_at(deadline, body.frame())
            .await
            .ok()??;
        let Ok(data) = frame.unwrap().into_data() else {
            continue;
        };
        text.push_str(std::str::from_utf8(&data).unwrap());
        if let Some(end) = text.find("\n\n") {
            let event: String = text.drain(..end + 2).collect();
            assert!(event.contains("event: dataVersion"), "{event}");
            let data = event
                .lines()
                .find_map(|l| l.strip_prefix("data: "))
                .unwrap();
            let v: Value = serde_json::from_str(data).unwrap();
            assert_eq!(v.as_object().unwrap().len(), 1, "no row data: {v}");
            return Some(v["dataVersion"].as_i64().unwrap());
        }
    }
}

#[tokio::test]
async fn the_page_is_told_of_each_applied_version_and_of_nothing_else() {
    let h = Harness::new().await;
    let mut applier = h.applier().await;
    let c = h.campaign(&mut applier, "Les Brumes").await;
    let (app, versions) = app(&h).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/data-version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let mut body = response.into_body();
    // First the current version.
    assert_eq!(
        next_version(&mut body, Duration::from_secs(1)).await,
        Some(1)
    );

    // An applied command: within 1 s.
    let (status, add) = call(
        &app,
        Method::POST,
        "/commands",
        Some(json!({
            "dataCapability": AJOUTER_PJ,
            "payload": { "campagneId": c.to_string(), "name": "Ysolde", "class": "Barde", "level": 3 },
            "idempotencyKey": "k-add",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    applier.drain().await.unwrap();
    assert_eq!(
        next_version(&mut body, Duration::from_secs(1)).await,
        Some(2)
    );
    let pc = add["partition"]
        .as_str()
        .unwrap()
        .strip_prefix("PJ/")
        .unwrap()
        .to_owned();

    // Rejected at enqueue, rejected at application, and a no-op: silence.
    call(
        &app,
        Method::POST,
        "/commands",
        Some(json!({
            "dataCapability": AJOUTER_PJ,
            "payload": { "campagneId": c.to_string(), "name": "Brume", "class": "Barde", "level": 0 },
            "idempotencyKey": "k-bad",
        })),
    )
    .await;
    call(
        &app,
        Method::POST,
        "/commands",
        Some(json!({
            "dataCapability": AJOUTER_PJ,
            "payload": { "campagneId": c.to_string(), "name": "ysolde", "class": "Barde", "level": 2 },
            "idempotencyKey": "k-dup",
        })),
    )
    .await;
    let archive = json!({ "dataCapability": ARCHIVER_PJ, "target": { "id": pc } });
    call(&app, Method::POST, "/commands", Some(archive.clone())).await;
    applier.drain().await.unwrap();
    assert_eq!(
        next_version(&mut body, Duration::from_secs(1)).await,
        Some(3)
    );
    call(&app, Method::POST, "/commands", Some(archive)).await;
    applier.drain().await.unwrap();
    assert_eq!(h.version().await, 3);
    assert_eq!(
        next_version(&mut body, Duration::from_millis(300)).await,
        None
    );

    // A second subscriber starts at the latest version.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/data-version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut late = response.into_body();
    assert_eq!(
        next_version(&mut late, Duration::from_secs(1)).await,
        Some(3)
    );
    drop((body, late, app, versions));
    h.drop_db().await;
}
