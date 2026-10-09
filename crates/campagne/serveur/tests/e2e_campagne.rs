//! The end-to-end proof of the milestone "the campaign knows its party", on a
//! fresh PostgreSQL database per test and invented data only.
//!
//! The commands go in through the server's real router (`POST /commands`,
//! `GET /commands/{id}`) and are applied by the real engine with the shipped
//! registry. One `GET /data-version` stream stays open for the whole scenario:
//! it is the open page. After each command the test waits for that stream to
//! announce the command's `dataVersion`, and only then reads again through the
//! three Capabilities, in-process, on `DATABASE_URL_READONLY`. Nothing is
//! reloaded or re-navigated; the page learns of each change from the stream.
//! Tombstones and row counts are read straight from the tables with the
//! privileged pool, never through a Capability.
//!
//! The database tests fail, never skip, without `DATABASE_URL` and
//! `DATABASE_URL_READONLY` (the `app_lecture` login).

mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use campagne_lister_pjs::{Executor, ListerPjs, ListerPjsError, Pj, registry_dir};
use campagne_niveau_du_groupe::{Capability, Error as NiveauError, NiveauDuGroupe, StartError};
use campagne_serveur::embedded::EMBEDDED;
use campagne_serveur::http::{self, AppState};
use campagne_serveur::{migrate, startup};
use chrono::{DateTime, Utc};
use common::{TestDb, code, database_url, readonly_options, readonly_url, url_with_database};
use dataguard::{Aggregates, Engine, SystemClock, VersionFeed};
use http_body_util::BodyExt;
use serde_json::{Map, Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use uuid::Uuid;

const READONLY_VARIABLE: &str = "DATABASE_URL_READONLY";
const WAIT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// The commands, as the GM's page submits them.
// ---------------------------------------------------------------------------

fn key() -> String {
    Uuid::new_v4().to_string()
}

fn creer(name: &str, key: &str) -> Value {
    json!({ "dataCapability": creer_campagne::KEY, "payload": { "name": name }, "idempotencyKey": key })
}

/// A PC payload; a `None` field is left out of the submission altogether.
fn pc_payload(
    campagne: Uuid,
    nom: Option<Value>,
    classe: Option<Value>,
    niveau: Option<Value>,
) -> Value {
    let mut payload = Map::new();
    payload.insert("campagneId".into(), json!(campagne.to_string()));
    for (field, value) in [("nom", nom), ("classe", classe), ("niveau", niveau)] {
        if let Some(value) = value {
            payload.insert(field.into(), value);
        }
    }
    Value::Object(payload)
}

fn ajouter_payload(payload: Value, key: &str) -> Value {
    json!({ "dataCapability": ajouter_pj::KEY, "payload": payload, "idempotencyKey": key })
}

fn ajouter(campagne: Uuid, nom: &str, classe: &str, niveau: Value, key: &str) -> Value {
    let payload = pc_payload(
        campagne,
        Some(json!(nom)),
        Some(json!(classe)),
        Some(niveau),
    );
    ajouter_payload(payload, key)
}

fn modifier(pc: Uuid, nom: &str, classe: &str, niveau: Value, based_on: i64) -> Value {
    json!({
        "dataCapability": modifier_pj::KEY,
        "target": { "id": pc },
        "payload": { "nom": nom, "classe": classe, "niveau": niveau },
        "basedOn": { "version": based_on },
        "idempotencyKey": key(),
    })
}

fn archiver_un_pj(pc: Uuid) -> Value {
    json!({ "dataCapability": archiver_pj::KEY, "target": { "id": pc }, "idempotencyKey": key() })
}

fn archiver_une_campagne(campagne: Uuid) -> Value {
    json!({
        "dataCapability": archiver_campagne::KEY,
        "target": { "id": campagne },
        "payload": {},
        "idempotencyKey": key(),
    })
}

// ---------------------------------------------------------------------------
// What a settled command says.
// ---------------------------------------------------------------------------

fn violations(result: &Value) -> Vec<String> {
    result["violations"]
        .as_array()
        .expect("a result lists its violations")
        .iter()
        .map(|v| v.as_str().expect("a violation is an id").to_owned())
        .collect()
}

/// Asserts the command was applied; returns the version it carries.
fn applied(result: &Value) -> i64 {
    assert_eq!(result["status"], "applied", "{result}");
    assert!(violations(result).is_empty(), "{result}");
    result["dataVersion"]
        .as_i64()
        .expect("an applied command carries its dataVersion")
}

/// Asserts the command was rejected for exactly `ids`, with no version.
fn rejected(result: &Value, ids: &[&str]) {
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["dataVersion"], Value::Null, "{result}");
    assert_eq!(violations(result), ids, "{result}");
    assert_eq!(result["reviewId"], Value::Null, "{result}");
}

// ---------------------------------------------------------------------------
// The server, the open page and the three Capabilities.
// ---------------------------------------------------------------------------

/// The answer to a submission.
struct Sent {
    command_id: String,
    partition: String,
    replayed: bool,
    body: Value,
}

impl Sent {
    /// The id minted for the new row, or the id of the target: the part of the
    /// partition after the aggregate's name.
    fn id(&self) -> Uuid {
        let (_, id) = self.partition.split_once('/').expect("<Aggregate>/<id>");
        Uuid::parse_str(id).expect("a row id")
    }
}

/// The open `GET /data-version` stream. What was read but not parsed yet is
/// kept between calls, so no event is lost when a read returns early.
struct EventStream {
    body: Body,
    buffered: String,
}

impl EventStream {
    fn new(body: Body) -> Self {
        Self {
            body,
            buffered: String::new(),
        }
    }

    /// The next `dataVersion` event, if one comes within `wait`. Keep-alive
    /// comments are skipped.
    async fn next_version(&mut self, wait: Duration) -> Option<i64> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            while let Some(end) = self.buffered.find("\n\n") {
                let event: String = self.buffered.drain(..end + 2).collect();
                if event.starts_with(':') {
                    continue;
                }
                assert!(event.contains("event: dataVersion"), "{event}");
                let data = event
                    .lines()
                    .find_map(|l| l.strip_prefix("data: "))
                    .expect("an event carries data");
                let v: Value = serde_json::from_str(data).expect("JSON data");
                assert_eq!(v.as_object().expect("an object").len(), 1, "no row data");
                return Some(v["dataVersion"].as_i64().expect("a version"));
            }
            let frame = tokio::time::timeout_at(deadline, self.body.frame())
                .await
                .ok()??;
            if let Ok(data) = frame.expect("the stream does not fail").into_data() {
                self.buffered
                    .push_str(std::str::from_utf8(&data).expect("UTF-8"));
            }
        }
    }
}

/// The lookup of `DATABASE_URL_READONLY` and nothing else, aimed at `url`.
fn readonly_only(url: String) -> impl Fn(&str) -> Option<String> {
    move |name| (name == READONLY_VARIABLE).then(|| url.clone())
}

async fn migration_records(pool: &PgPool) -> Vec<(String, String, String, String)> {
    sqlx::query_as(
        "SELECT name, checksum, applied_at::text, xmin::text FROM schema_migrations ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("read schema_migrations")
}

/// Proof run, step 1: the first start applies every migration, a second one
/// applies none and leaves the records as they were. Returns the number of
/// migrations.
async fn migrate_twice(db: &TestDb) -> usize {
    assert!(
        !common::table_exists(&db.pool, "schema_migrations").await,
        "the database starts empty"
    );
    let all: Vec<String> = EMBEDDED.iter().map(|m| m.name.to_owned()).collect();
    assert!(!all.is_empty());

    let first = migrate::run(&db.pool, EMBEDDED)
        .await
        .expect("the first start migrates");
    assert_eq!(first.applied, all, "every migration, in order");
    let records = migration_records(&db.pool).await;
    assert_eq!(records.len(), all.len());
    for migration in EMBEDDED {
        let record = records
            .iter()
            .find(|r| r.0 == migration.name)
            .expect("each migration is recorded");
        assert_eq!(record.1, migration.checksum(), "{}", migration.name);
    }
    for relation in [
        "campagne",
        "pj",
        "campagne_active",
        "pj_actif",
        "data_version",
        "dataguard_queue",
    ] {
        assert!(common::table_exists(&db.pool, relation).await, "{relation}");
    }

    let second = migrate::run(&db.pool, EMBEDDED)
        .await
        .expect("the second start migrates");
    assert!(second.applied.is_empty(), "nothing is re-applied");
    assert_eq!(
        migration_records(&db.pool).await,
        records,
        "same names, same checksums, same rows"
    );
    all.len()
}

struct World {
    db: TestDb,
    app: Router,
    stop: watch::Sender<bool>,
    applier: JoinHandle<()>,
    /// The open `GET /data-version` stream.
    sse: EventStream,
    /// The highest `dataVersion` the page has been told of.
    seen: i64,
    /// Row counts (campaigns, PCs) at the last check: they never drop.
    floor: (i64, i64),
    campagnes: Executor,
    pjs: ListerPjs<Executor>,
    niveau: Capability,
}

impl World {
    async fn start() -> Self {
        let db = TestDb::create().await;
        migrate_twice(&db).await;

        let aggregates = Aggregates::embedded().expect("the aggregates");
        let registry = startup::registry(&aggregates).expect("the shipped registry");
        let engine = Arc::new(Engine::with_aggregates(
            db.pool.clone(),
            aggregates,
            registry,
            Arc::new(SystemClock),
        ));
        let (stop, stopped) = watch::channel(false);
        let applier = engine
            .applier()
            .await
            .expect("the applier starts")
            .expect("the applier lock is free");
        let mut applier_stop = stopped.clone();
        let applier = tokio::spawn(applier.run(async move {
            let _ = applier_stop.wait_for(|s| *s).await;
        }));
        let mut feed_stop = stopped.clone();
        let versions = VersionFeed::spawn(db.pool.clone(), async move {
            let _ = feed_stop.wait_for(|s| *s).await;
        })
        .await
        .expect("the version feed starts");
        let app = http::app(AppState { engine, versions });

        // The open page: the stream tells the current version first.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/data-version")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("the stream opens");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        let mut sse = EventStream::new(response.into_body());
        let seen = sse
            .next_version(WAIT)
            .await
            .expect("the stream tells the current version first");

        // The Capabilities see the test database through the restricted login.
        let read_url = url_with_database(&readonly_url(), &db.name);
        let dir = registry_dir();
        let campagnes = lister_campagnes::demarrer_avec(&dir, readonly_only(read_url.clone()))
            .await
            .expect("lister-campagnes starts");
        let pjs = ListerPjs::new(
            Executor::from_variables(&dir, readonly_only(read_url.clone()))
                .await
                .expect("lister-pjs starts"),
        );
        let niveau = Capability::from_variables(readonly_only(read_url))
            .await
            .expect("niveau-du-groupe starts");

        let floor = (0, 0);
        Self {
            db,
            app,
            stop,
            applier,
            sse,
            seen,
            floor,
            campagnes,
            pjs,
            niveau,
        }
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        drop(self.sse);
        let _ = tokio::time::timeout(WAIT, self.applier).await;
        self.db.drop_db().await;
    }

    // --- the wire -----------------------------------------------------------

    /// Status and body; the body is checked for the connection strings.
    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        let body = match body {
            Some(b) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).expect("a request"))
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("a body")
            .to_bytes();
        let text = String::from_utf8(bytes.to_vec()).expect("UTF-8");
        for secret in [database_url(), readonly_url(), self.db.url.clone()] {
            assert!(!text.contains(&secret), "a connection string leaked");
        }
        (status, serde_json::from_str(&text).expect("a JSON answer"))
    }

    /// Queues a command. The answer is `202` at once, whatever the command.
    async fn send(&self, submission: Value) -> Sent {
        let (status, body) = self.call(Method::POST, "/commands", Some(submission)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        Sent {
            command_id: body["commandId"].as_str().expect("commandId").to_owned(),
            partition: body["partition"].as_str().expect("partition").to_owned(),
            replayed: body["replayed"].as_bool().expect("replayed"),
            body,
        }
    }

    async fn lookup(&self, command_id: &str) -> Value {
        let (status, body) = self
            .call(Method::GET, &format!("/commands/{command_id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn result_of(&self, command_id: &str) -> Value {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let body = self.lookup(command_id).await;
            if let Some(result) = body.get("result") {
                return result.clone();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{command_id} was not settled within {WAIT:?}: {body}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits for the command's result, then for the open page to be told of
    /// its version. A rejection carries none, and the page is told of nothing.
    async fn settle(&mut self, sent: &Sent) -> Value {
        let result = self.result_of(&sent.command_id).await;
        if let Some(version) = result["dataVersion"].as_i64() {
            self.page_reaches(version).await;
        }
        result
    }

    async fn page_reaches(&mut self, version: i64) {
        while self.seen < version {
            let next =
                self.sse.next_version(WAIT).await.unwrap_or_else(|| {
                    panic!("the open page was never told of dataVersion {version}")
                });
            assert!(next > self.seen, "versions only grow");
            self.seen = next;
        }
    }

    /// The page hears nothing within a moment: no event for a rejection.
    async fn page_is_quiet(&mut self) {
        let heard = self.sse.next_version(Duration::from_millis(300)).await;
        assert_eq!(heard, None, "the page was told of a version");
    }

    async fn run(&mut self, submission: Value) -> (Sent, Value) {
        let sent = self.send(submission).await;
        let result = self.settle(&sent).await;
        (sent, result)
    }

    // --- seeding ------------------------------------------------------------

    async fn campagne(&mut self, name: &str) -> Uuid {
        let (sent, result) = self.run(creer(name, &key())).await;
        applied(&result);
        sent.id()
    }

    async fn pc(&mut self, campagne: Uuid, nom: &str, classe: &str, niveau: i64) -> Uuid {
        let (sent, result) = self
            .run(ajouter(campagne, nom, classe, json!(niveau), &key()))
            .await;
        applied(&result);
        sent.id()
    }

    // --- the reads, through the Capabilities --------------------------------

    async fn campaigns(&self) -> Vec<(String, String)> {
        lister_campagnes::lister_campagnes(&self.campagnes)
            .await
            .expect("lister-campagnes")
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect()
    }

    async fn pcs(&self, campagne: Uuid) -> Result<Vec<Pj>, ListerPjsError> {
        self.pjs.lister(&campagne.to_string()).await
    }

    async fn level(&self, campagne: Uuid) -> Result<NiveauDuGroupe, NiveauError> {
        self.niveau.run(&campagne.to_string()).await
    }

    /// The party as the open page shows it: the level, the PCs in order, and
    /// the version the read is at — the one the page was last told of.
    async fn assert_party(&self, campagne: Uuid, level: Option<u8>, pcs: &[(&str, i32)]) {
        let n = self.level(campagne).await.expect("party level");
        let count = u64::try_from(pcs.len()).expect("a small count");
        assert_eq!((n.level, n.pc_count, n.model), (level, count, "v1"));
        assert_eq!(i64::try_from(n.as_of).expect("a version"), self.seen);
        let shown: Vec<(String, i32)> = self
            .pcs(campagne)
            .await
            .expect("pcs")
            .into_iter()
            .map(|p| (p.name, p.level))
            .collect();
        let wanted: Vec<(String, i32)> = pcs.iter().map(|(n, l)| ((*n).to_owned(), *l)).collect();
        assert_eq!(shown, wanted);
    }

    // --- the tables, through the privileged pool ----------------------------

    async fn row_counts(&self) -> (i64, i64) {
        let campaigns = sqlx::query_scalar("SELECT count(*) FROM campagne")
            .fetch_one(&self.db.pool)
            .await
            .expect("count campagne");
        let pcs = sqlx::query_scalar("SELECT count(*) FROM pj")
            .fetch_one(&self.db.pool)
            .await
            .expect("count pj");
        (campaigns, pcs)
    }

    /// No hard delete: neither count has ever dropped.
    async fn counts_never_drop(&mut self) {
        let now = self.row_counts().await;
        assert!(
            now.0 >= self.floor.0 && now.1 >= self.floor.1,
            "{now:?} fell below {:?}",
            self.floor
        );
        self.floor = now;
    }

    async fn version(&self) -> i64 {
        sqlx::query_scalar("SELECT \"dataVersion\" FROM data_version")
            .fetch_one(&self.db.pool)
            .await
            .expect("read the version")
    }

    /// The row must exist; `None` is an active row.
    async fn pc_tombstone(&self, pc: Uuid) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT \"archiveLe\" FROM pj WHERE id = $1")
            .bind(pc)
            .fetch_one(&self.db.pool)
            .await
            .expect("the PC row is still there")
    }

    async fn campaign_tombstone(&self, campagne: Uuid) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT \"archiveLe\" FROM campagne WHERE id = $1")
            .bind(campagne)
            .fetch_one(&self.db.pool)
            .await
            .expect("the campaign row is still there")
    }

    async fn pc_row_count_of(&self, campagne: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM pj WHERE \"campagneId\" = $1")
            .bind(campagne)
            .fetch_one(&self.db.pool)
            .await
            .expect("count the PCs of a campaign")
    }
}

// ---------------------------------------------------------------------------
// Proof run, step 1: the migrations.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_first_start_applies_every_migration_and_the_second_applies_none() {
    let db = TestDb::create().await;
    let count = migrate_twice(&db).await;
    assert!(count >= 6, "the schema, the views, the role, the engine");
    db.drop_db().await;
}

// ---------------------------------------------------------------------------
// Proof run, step 2: the nominal scenario.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nominal_scenario_updates_level_and_lists_without_reload() {
    let mut w = World::start().await;
    assert_eq!(
        w.row_counts().await,
        (0, 0),
        "a fresh database holds no row"
    );
    assert_eq!(w.version().await, 0);
    assert_eq!(w.seen, 0);
    assert!(w.campaigns().await.is_empty());

    // A campaign appears under its trimmed name; its party has no level.
    let (sent, result) = w.run(creer(" Les Brumes ", &key())).await;
    assert_eq!(applied(&result), 1);
    let c = sent.id();
    assert_eq!(
        w.campaigns().await,
        [(c.to_string(), "Les Brumes".to_owned())]
    );
    let empty = w.level(c).await.expect("an empty party is not an error");
    assert_eq!((empty.level, empty.pc_count, empty.model), (None, 0, "v1"));
    assert_eq!(empty.as_of, 1);
    w.assert_party(c, None, &[]).await;
    w.counts_never_drop().await;

    // Two PCs sent back to back, before either is settled: both are applied,
    // in the order they were sent. Levels [3, 4] give 4 (3.5 rounds up).
    let first = w
        .send(ajouter(c, "Ysolde", "Barde", json!(3), &key()))
        .await;
    let second = w
        .send(ajouter(c, "Brannoc", "Guerrier", json!(4), &key()))
        .await;
    let (first_result, second_result) = (w.settle(&first).await, w.settle(&second).await);
    assert_eq!((applied(&first_result), applied(&second_result)), (2, 3));
    let (ysolde, brannoc) = (first.id(), second.id());
    w.assert_party(c, Some(4), &[("Ysolde", 3), ("Brannoc", 4)])
        .await;
    w.counts_never_drop().await;

    // An edit of a level: the next read never shows the pre-edit value.
    // Levels [3, 1]: the mean is exactly 2.
    let (_, result) = w
        .run(modifier(brannoc, "Brannoc", "Guerrier", json!(1), w.seen))
        .await;
    assert_eq!(applied(&result), 4);
    w.assert_party(c, Some(2), &[("Ysolde", 3), ("Brannoc", 1)])
        .await;

    // A third PC: levels [3, 1, 2], an exact mean of 2.
    let cyrielle = w.pc(c, "Cyrielle", "Magicienne", 2).await;
    w.assert_party(
        c,
        Some(2),
        &[("Ysolde", 3), ("Brannoc", 1), ("Cyrielle", 2)],
    )
    .await;

    // Levels [1, 1, 2]: a mean of 1.33 rounds down.
    let (_, result) = w
        .run(modifier(ysolde, "Ysolde", "Barde", json!(1), w.seen))
        .await;
    assert_eq!(applied(&result), 6);
    w.assert_party(
        c,
        Some(1),
        &[("Ysolde", 1), ("Brannoc", 1), ("Cyrielle", 2)],
    )
    .await;
    w.counts_never_drop().await;

    // Archiving a PC: it leaves the list and the level (levels [1, 2], 1.5
    // rounds up), and its row stays with a tombstone.
    let (_, result) = w.run(archiver_un_pj(brannoc)).await;
    assert_eq!(applied(&result), 7);
    w.assert_party(c, Some(2), &[("Ysolde", 1), ("Cyrielle", 2)])
        .await;
    assert!(w.pc_tombstone(brannoc).await.is_some());
    assert!(w.pc_tombstone(ysolde).await.is_none());
    w.counts_never_drop().await;

    // A single PC gives its own level.
    let (_, result) = w.run(archiver_un_pj(ysolde)).await;
    assert_eq!(applied(&result), 8);
    w.assert_party(c, Some(2), &[("Cyrielle", 2)]).await;

    // Another PC is added after all that: levels [2, 5] give 4 (3.5).
    let dorian = w.pc(c, "Dorian", "Rôdeur", 5).await;
    w.assert_party(c, Some(4), &[("Cyrielle", 2), ("Dorian", 5)])
        .await;

    // The last active PCs archived: the campaign is back to "no party level".
    let (_, result) = w.run(archiver_un_pj(cyrielle)).await;
    applied(&result);
    w.assert_party(c, Some(5), &[("Dorian", 5)]).await;
    let (_, result) = w.run(archiver_un_pj(dorian)).await;
    assert_eq!(applied(&result), 11);
    let empty = w.level(c).await.expect("an empty party is not an error");
    assert_eq!((empty.level, empty.pc_count), (None, 0), "never 0, never 1");
    w.assert_party(c, None, &[]).await;

    // Nothing was ever removed: four PCs and one campaign, all still there.
    w.counts_never_drop().await;
    assert_eq!(w.row_counts().await, (1, 4));
    for pc in [ysolde, brannoc, cyrielle, dorian] {
        assert!(w.pc_tombstone(pc).await.is_some());
    }
    assert!(
        w.campaign_tombstone(c).await.is_none(),
        "the campaign lives"
    );
    assert_eq!(w.version().await, 11, "one version per applied command");
    w.stop().await;
}

// ---------------------------------------------------------------------------
// Refusals.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invalid_pcs_are_refused_per_field_and_nothing_is_saved() {
    let mut w = World::start().await;
    let c = w.campagne("Les Brumes").await;
    let ysolde = w.pc(c, "Ysolde", "Barde", 3).await;
    w.assert_party(c, Some(3), &[("Ysolde", 3)]).await;
    w.counts_never_drop().await;

    let barde = || Some(json!("Barde"));
    let nom = || Some(json!("Brannoc"));
    let cases: Vec<(&str, Value, &[&str])> = vec![
        // The name.
        (
            "empty name",
            pc_payload(c, Some(json!("")), barde(), Some(json!(4))),
            &["pc-name-required"],
        ),
        (
            "blank name",
            pc_payload(c, Some(json!("   ")), barde(), Some(json!(4))),
            &["pc-name-required"],
        ),
        (
            "missing name",
            pc_payload(c, None, barde(), Some(json!(4))),
            &["pc-name-required"],
        ),
        // The class.
        (
            "empty class",
            pc_payload(c, nom(), Some(json!("")), Some(json!(4))),
            &["pc-class-required"],
        ),
        (
            "missing class",
            pc_payload(c, nom(), None, Some(json!(4))),
            &["pc-class-required"],
        ),
        // The level: each refusal on its own.
        (
            "level 0",
            pc_payload(c, nom(), barde(), Some(json!(0))),
            &["pc-level-range"],
        ),
        (
            "level 21",
            pc_payload(c, nom(), barde(), Some(json!(21))),
            &["pc-level-range"],
        ),
        (
            "negative level",
            pc_payload(c, nom(), barde(), Some(json!(-1))),
            &["pc-level-range"],
        ),
        (
            "decimal level",
            pc_payload(c, nom(), barde(), Some(json!(1.5))),
            &["pc-level-range"],
        ),
        (
            "numeric string",
            pc_payload(c, nom(), barde(), Some(json!("5"))),
            &["pc-level-range"],
        ),
        (
            "missing level",
            pc_payload(c, nom(), barde(), None),
            &["pc-level-range"],
        ),
        // Nothing partial: the valid fields are not saved beside a wrong one.
        (
            "wrong name and level",
            pc_payload(c, Some(json!("")), barde(), Some(json!(21))),
            &["pc-name-required", "pc-level-range"],
        ),
        (
            "nothing at all",
            pc_payload(c, None, None, None),
            &["pc-name-required", "pc-class-required", "pc-level-range"],
        ),
    ];

    let (version, counts) = (w.version().await, w.row_counts().await);
    for (what, payload, ids) in cases {
        // The submission is queued, as a client that skipped its own check
        // would send it: the application-time check decides.
        let (sent, result) = w.run(ajouter_payload(payload, &key())).await;
        assert!(!sent.replayed, "{what}");
        rejected(&result, ids);
        assert_eq!(w.version().await, version, "{what}: no version consumed");
        assert_eq!(w.row_counts().await, counts, "{what}: no row");
        w.assert_party(c, Some(3), &[("Ysolde", 3)]).await;
    }
    w.page_is_quiet().await;

    // An edit that bypassed the client check is refused the same way.
    let (_, result) = w
        .run(modifier(ysolde, "Ysolde", "Barde", json!(21), w.seen))
        .await;
    rejected(&result, &["pc-level-range"]);
    let (_, result) = w.run(modifier(ysolde, "", "Barde", json!(5), w.seen)).await;
    rejected(&result, &["pc-name-required"]);
    w.assert_party(c, Some(3), &[("Ysolde", 3)]).await;
    assert_eq!(w.row_counts().await, counts);

    // The sequence has no gap: the next valid command takes the next version.
    let (_, result) = w
        .run(ajouter(c, "Brannoc", "Guerrier", json!(4), &key()))
        .await;
    assert_eq!(applied(&result), version + 1);
    w.assert_party(c, Some(4), &[("Ysolde", 3), ("Brannoc", 4)])
        .await;
    w.counts_never_drop().await;
    w.stop().await;
}

// ---------------------------------------------------------------------------
// Isolation.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn campaigns_never_leak_and_not_found_is_one_answer() {
    let mut w = World::start().await;
    let a = w.campagne("Les Brumes").await;
    let b = w.campagne("Les Brumes").await; // two campaigns may share a name
    assert_ne!(a, b);
    let ysolde_a = w.pc(a, "Ysolde", "Barde", 3).await;
    let _ysolde_b = w.pc(b, "Ysolde", "Barde", 5).await; // a name may repeat across campaigns
    let brannoc_a = w.pc(a, "Brannoc", "Guerrier", 4).await;
    let (_, result) = w.run(archiver_un_pj(brannoc_a)).await;
    applied(&result);

    // An archived PC of A is in neither list nor level, for A or for B.
    w.assert_party(a, Some(3), &[("Ysolde", 3)]).await;
    w.assert_party(b, Some(5), &[("Ysolde", 5)]).await;

    // A PC added to B never changes A's party, and the other way round.
    w.pc(b, "Cyrielle", "Magicienne", 9).await;
    w.assert_party(a, Some(3), &[("Ysolde", 3)]).await;
    w.assert_party(b, Some(7), &[("Ysolde", 5), ("Cyrielle", 9)])
        .await;
    w.pc(a, "Dorian", "Rôdeur", 1).await;
    w.assert_party(a, Some(2), &[("Ysolde", 3), ("Dorian", 1)])
        .await;
    w.assert_party(b, Some(7), &[("Ysolde", 5), ("Cyrielle", 9)])
        .await;

    // In one campaign, a case-insensitive duplicate of an active name is
    // refused; the name is free again once its holder is archived.
    for duplicate in ["ysolde", "  YSOLDE  "] {
        let (_, result) = w
            .run(ajouter(a, duplicate, "Barde", json!(2), &key()))
            .await;
        rejected(&result, &["pc-name-unique-in-campaign"]);
    }
    w.assert_party(a, Some(2), &[("Ysolde", 3), ("Dorian", 1)])
        .await;
    w.pc(a, "Brannoc", "Guerrier", 4).await; // its first holder is archived
    w.assert_party(a, Some(3), &[("Ysolde", 3), ("Dorian", 1), ("Brannoc", 4)])
        .await;

    // An unknown, an archived and a foreign campaign id: one answer.
    let archived = w.campagne("Les Mouettes").await;
    w.pc(archived, "Elwen", "Clerc", 6).await;
    let (_, result) = w.run(archiver_une_campagne(archived)).await;
    applied(&result);
    let unknown = Uuid::new_v4();
    let foreign = ysolde_a; // well formed, and the id of a PC, not of a campaign
    let mut pcs_answers = Vec::new();
    let mut level_answers = Vec::new();
    for id in [unknown, archived, foreign] {
        let pcs = w.pcs(id).await.expect_err("no PC list");
        assert!(matches!(pcs, ListerPjsError::NotFound), "{pcs:?}");
        pcs_answers.push((pcs.to_string(), format!("{pcs:?}")));
        let level = w.level(id).await.expect_err("no party level");
        assert!(matches!(level, NiveauError::NotFound), "{level:?}");
        level_answers.push((level.to_string(), format!("{level:?}")));
    }
    assert!(
        pcs_answers.windows(2).all(|p| p[0] == p[1]),
        "{pcs_answers:?}"
    );
    assert!(
        level_answers.windows(2).all(|p| p[0] == p[1]),
        "{level_answers:?}"
    );
    assert_eq!(pcs_answers[0].0, "not found");
    assert_eq!(level_answers[0].0, "not found");

    // The campaign list shows the two active ones, newest first.
    let listed: Vec<String> = w.campaigns().await.into_iter().map(|c| c.0).collect();
    assert_eq!(listed, [b.to_string(), a.to_string()]);
    w.counts_never_drop().await;
    w.stop().await;
}

// ---------------------------------------------------------------------------
// Tombstones.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn archive_is_a_tombstone_cascades_and_repeats_as_a_noop() {
    let mut w = World::start().await;
    let c = w.campagne("Les Brumes").await;
    let ysolde = w.pc(c, "Ysolde", "Barde", 3).await;
    let brannoc = w.pc(c, "Brannoc", "Guerrier", 4).await;
    let cyrielle = w.pc(c, "Cyrielle", "Magicienne", 2).await;
    let dorian = w.pc(c, "Dorian", "Rôdeur", 5).await;
    let (_, result) = w.run(archiver_un_pj(dorian)).await;
    applied(&result);
    let dorian_at = w.pc_tombstone(dorian).await.expect("Dorian is tombstoned");
    let counts = w.row_counts().await;
    assert_eq!(counts, (1, 4));
    w.counts_never_drop().await;
    let version = w.version().await;

    // An archived PC of an active campaign accepts no edit.
    let (_, result) = w
        .run(modifier(dorian, "Dorian", "Rôdeur", json!(6), w.seen))
        .await;
    rejected(&result, &["pc-active"]);
    assert_eq!(w.pc_tombstone(dorian).await, Some(dorian_at));
    assert_eq!(w.version().await, version);

    // A fault on one PC of the cascade: the command is retried, and meanwhile
    // neither the campaign nor any other PC is touched. The trigger counts its
    // attempts in a sequence, which a rollback does not undo.
    sqlx::raw_sql(
        "CREATE SEQUENCE panne_essais;
         CREATE FUNCTION panne() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN
                IF NEW.nom = 'Brannoc' THEN PERFORM nextval('panne_essais'); RAISE EXCEPTION 'panne'; END IF;
                RETURN NEW;
              END $$;
         CREATE TRIGGER panne BEFORE UPDATE ON pj FOR EACH ROW EXECUTE FUNCTION panne();",
    )
    .execute(&w.db.pool)
    .await
    .expect("install the fault");
    let archive = w.send(archiver_une_campagne(c)).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let attempts: i64 = sqlx::query_scalar(
            "SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM panne_essais",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("read the attempts");
        if attempts >= 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the applier never tried the cascade"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        w.lookup(&archive.command_id).await.get("result").is_none(),
        "the command is not settled while the fault lasts"
    );
    assert_eq!(w.campaign_tombstone(c).await, None, "all or nothing");
    for pc in [ysolde, brannoc, cyrielle] {
        assert_eq!(w.pc_tombstone(pc).await, None, "all or nothing");
    }
    assert_eq!(w.pc_tombstone(dorian).await, Some(dorian_at));
    assert_eq!(w.version().await, version);
    assert_eq!(w.row_counts().await, counts);
    w.assert_party(
        c,
        Some(3),
        &[("Ysolde", 3), ("Brannoc", 4), ("Cyrielle", 2)],
    )
    .await;

    // The fault is gone: the whole cascade lands under one version.
    sqlx::raw_sql("DROP TRIGGER panne ON pj")
        .execute(&w.db.pool)
        .await
        .expect("remove the fault");
    let result = w.settle(&archive).await;
    assert_eq!(applied(&result), version + 1, "one version for the cascade");

    // Direct, privileged reads: every row is still there, tombstoned.
    assert_eq!(w.row_counts().await, counts, "no hard delete");
    w.counts_never_drop().await;
    let campaign_at = w
        .campaign_tombstone(c)
        .await
        .expect("the campaign is tombstoned");
    let mut pc_tombstones = Vec::new();
    for pc in [ysolde, brannoc, cyrielle] {
        let at = w.pc_tombstone(pc).await.expect("the PC is tombstoned");
        assert_eq!(at, campaign_at, "the cascade is one act");
        pc_tombstones.push(at);
    }
    assert_eq!(
        w.pc_tombstone(dorian).await,
        Some(dorian_at),
        "the earlier tombstone is kept"
    );
    assert_eq!(w.pc_row_count_of(c).await, 4);

    // No list and no level shows any of it.
    assert!(w.campaigns().await.is_empty());
    assert!(matches!(w.pcs(c).await, Err(ListerPjsError::NotFound)));
    assert!(matches!(w.level(c).await, Err(NiveauError::NotFound)));

    // Archiving again, under new keys: applied, nothing written, no new
    // version, and the original tombstones are kept.
    let version = w.version().await;
    let (_, result) = w.run(archiver_une_campagne(c)).await;
    assert_eq!(applied(&result), version, "no new version");
    let (_, result) = w.run(archiver_un_pj(ysolde)).await;
    assert_eq!(applied(&result), version, "no new version");
    let (_, result) = w.run(archiver_un_pj(dorian)).await;
    assert_eq!(applied(&result), version, "no new version");
    assert_eq!(w.version().await, version);
    assert_eq!(w.campaign_tombstone(c).await, Some(campaign_at));
    assert_eq!(w.pc_tombstone(ysolde).await, Some(pc_tombstones[0]));
    assert_eq!(w.pc_tombstone(dorian).await, Some(dorian_at));
    assert_eq!(w.row_counts().await, counts);

    // An archived campaign accepts no new PC; an archived PC accepts no edit.
    let (_, result) = w.run(ajouter(c, "Elwen", "Clerc", json!(6), &key())).await;
    rejected(&result, &["campaign-active"]);
    let (_, result) = w
        .run(modifier(ysolde, "Ysolde", "Barde", json!(9), w.seen))
        .await;
    // Its campaign is archived too: both invariants say so.
    rejected(&result, &["campaign-active", "pc-active"]);
    assert_eq!(w.version().await, version);
    assert_eq!(w.row_counts().await, counts);
    assert_eq!(w.pc_tombstone(ysolde).await, Some(pc_tombstones[0]));
    w.stop().await;
}

// ---------------------------------------------------------------------------
// Double submit and concurrency.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn double_submit_makes_no_duplicate_and_nothing_is_refused_for_concurrency() {
    let mut w = World::start().await;
    let c = w.campagne("Les Brumes").await;

    // The same create twice, with the same key: one campaign, one version.
    let (campaigns, _) = w.row_counts().await;
    let version = w.version().await;
    let k = key();
    let first = w.send(creer("Les Mouettes", &k)).await;
    let again = w.send(creer("Les Mouettes", &k)).await;
    assert!(!first.replayed && again.replayed);
    assert_eq!(again.command_id, first.command_id);
    assert_eq!(again.partition, first.partition);
    let result = w.settle(&first).await;
    assert_eq!(applied(&result), version + 1);
    let third = w.send(creer("Les Mouettes", &k)).await; // after it settled
    assert!(third.replayed);
    assert_eq!(third.command_id, first.command_id);
    assert_eq!(third.body["result"], result, "the original result");
    assert_eq!(w.row_counts().await.0, campaigns + 1);
    assert_eq!(w.version().await, version + 1);

    // The same add twice: one PC, one version.
    let (_, pcs) = w.row_counts().await;
    let version = w.version().await;
    let k = key();
    let first = w.send(ajouter(c, "Ysolde", "Barde", json!(3), &k)).await;
    let again = w.send(ajouter(c, "Ysolde", "Barde", json!(3), &k)).await;
    assert!(!first.replayed && again.replayed);
    assert_eq!(again.command_id, first.command_id);
    let result = w.settle(&first).await;
    assert_eq!(applied(&result), version + 1);
    let third = w.send(ajouter(c, "Ysolde", "Barde", json!(3), &k)).await;
    assert!(third.replayed);
    assert_eq!(third.body["result"], result, "the original result");
    assert_eq!(first.id(), third.id(), "the same PC");
    assert_eq!(w.row_counts().await.1, pcs + 1);
    assert_eq!(w.version().await, version + 1);
    w.assert_party(c, Some(3), &[("Ysolde", 3)]).await;

    // The same name under two keys is not a double submit: the first is
    // applied, the second is rejected for the invariant — both accepted at
    // once, neither refused for concurrency.
    let first = w
        .send(ajouter(c, "Brannoc", "Guerrier", json!(4), &key()))
        .await;
    let second = w
        .send(ajouter(c, "Brannoc", "Guerrier", json!(4), &key()))
        .await;
    assert!(!first.replayed && !second.replayed);
    assert_ne!(first.command_id, second.command_id);
    let (a, b) = (w.settle(&first).await, w.settle(&second).await);
    applied(&a);
    rejected(&b, &["pc-name-unique-in-campaign"]);
    w.assert_party(c, Some(4), &[("Ysolde", 3), ("Brannoc", 4)])
        .await;

    // Two commands on one campaign, sent back to back: both are processed, in
    // the order they were sent.
    let d = w.campagne("Les Écueils").await;
    let add = w
        .send(ajouter(d, "Cyrielle", "Magicienne", json!(2), &key()))
        .await;
    let archive = w.send(archiver_une_campagne(d)).await;
    let (add, archive) = (w.settle(&add).await, w.settle(&archive).await);
    assert_eq!(applied(&archive), applied(&add) + 1);
    assert_eq!(w.pc_row_count_of(d).await, 1);
    assert!(w.campaign_tombstone(d).await.is_some());

    // The other way round: the add meets the archive and is rejected for the
    // invariant `campaign-active`, never for concurrency.
    let e = w.campagne("Les Falaises").await;
    let archive = w.send(archiver_une_campagne(e)).await;
    let add = w
        .send(ajouter(e, "Dorian", "Rôdeur", json!(5), &key()))
        .await;
    let (archive, add) = (w.settle(&archive).await, w.settle(&add).await);
    applied(&archive);
    rejected(&add, &["campaign-active"]);
    assert_eq!(w.pc_row_count_of(e).await, 0);
    w.counts_never_drop().await;
    w.stop().await;
}

// ---------------------------------------------------------------------------
// The read-only role.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn writes_through_the_readonly_url_fail_at_the_database() {
    let mut w = World::start().await;
    let c = w.campagne("Les Brumes").await;
    w.pc(c, "Ysolde", "Barde", 3).await;
    let (counts, version) = (w.row_counts().await, w.version().await);

    let ro = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(readonly_options(&w.db.name))
        .await
        .expect("connect through DATABASE_URL_READONLY");

    // The login is a real restricted role, not the migrating one.
    let (user, superuser): (String, bool) = sqlx::query_as(
        "SELECT current_user::text, (SELECT rolsuper FROM pg_roles WHERE rolname = current_user)",
    )
    .fetch_one(&ro)
    .await
    .expect("read the role");
    assert_eq!(user, "app_lecture");
    assert!(!superuser);

    // No relation of the schema grants it a write privilege.
    let granted: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname::text, p.priv
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
          CROSS JOIN (VALUES ('INSERT'), ('UPDATE'), ('DELETE'), ('TRUNCATE')) AS p(priv)
          WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v')
            AND has_table_privilege('app_lecture', c.oid, p.priv)",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("read the privileges");
    assert!(granted.is_empty(), "{granted:?}");

    // Insert, update and delete, on the tables and on the views the role may
    // read: each fails at the database. On the tables it is a privilege error;
    // a view may also refuse as not updatable (55000), and that is still the
    // database refusing.
    for relation in ["campagne", "pj", "campagne_active", "pj_actif"] {
        let insert = match relation {
            "campagne" | "campagne_active" => format!("INSERT INTO {relation} (nom) VALUES ('x')"),
            _ => format!(
                "INSERT INTO {relation} (\"campagneId\", nom, classe, niveau) VALUES ('{c}', 'x', 'y', 1)"
            ),
        };
        let update = match relation {
            "campagne" | "campagne_active" => format!("UPDATE {relation} SET nom = 'x'"),
            _ => format!("UPDATE {relation} SET niveau = 1"),
        };
        for sql in [insert, update, format!("DELETE FROM {relation}")] {
            let state = code(sqlx::raw_sql(AssertSqlSafe(sql.clone())).execute(&ro).await);
            let table = matches!(relation, "campagne" | "pj");
            assert!(
                state == "42501" || (!table && state == "55000"),
                "{sql}: SQLSTATE {state}"
            );
        }
    }
    assert_eq!(w.row_counts().await, counts, "no row changed");
    assert_eq!(w.version().await, version);

    // There is no free-form query to write with either.
    let free_form = w.campagnes.run("DELETE FROM pj", &Map::new()).await;
    assert!(
        matches!(free_form, Err(lister_campagnes::QueryError::Unknown)),
        "a query that is not registered is refused"
    );
    assert_eq!(w.row_counts().await, counts);

    // A Capability never falls back to DATABASE_URL, and does not start
    // without DATABASE_URL_READONLY.
    let privileged = w.db.url.clone();
    let only_privileged = |name: &str| (name == "DATABASE_URL").then(|| privileged.clone());
    let nothing = |_: &str| -> Option<String> { None };
    let mut errors = Vec::new();
    for lookup in [
        &only_privileged as &dyn Fn(&str) -> Option<String>,
        &nothing,
    ] {
        match Capability::from_variables(lookup).await {
            Err(error) => {
                assert!(matches!(error, StartError::Missing(READONLY_VARIABLE)));
                errors.push(error);
            }
            Ok(_) => panic!("niveau-du-groupe started without {READONLY_VARIABLE}"),
        }
        match lister_campagnes::demarrer_avec(&registry_dir(), lookup).await {
            Err(error) => {
                assert!(matches!(error, StartError::Missing(READONLY_VARIABLE)));
                errors.push(error);
            }
            Ok(_) => panic!("lister-campagnes started without {READONLY_VARIABLE}"),
        }
        match Executor::from_variables(&registry_dir(), lookup).await {
            Err(error) => {
                assert!(matches!(error, StartError::Missing(READONLY_VARIABLE)));
                errors.push(error);
            }
            Ok(_) => panic!("lister-pjs started without {READONLY_VARIABLE}"),
        }
    }
    // No connection string in what they say.
    for error in &errors {
        for text in [error.to_string(), format!("{error:?}")] {
            for secret in [database_url(), readonly_url(), w.db.url.clone()] {
                assert!(!text.contains(&secret), "a connection string leaked");
            }
        }
    }
    ro.close().await;
    w.stop().await;
}
