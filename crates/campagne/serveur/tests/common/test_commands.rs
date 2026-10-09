//! Test-only DataCapabilities and helpers for the engine tests. They live in
//! test code and never as a `data-capability.json`: the shipped server
//! registers none.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dataguard::{
    Aggregates, Applier, Change, CommandId, CommandResult, DataCapability, DataCapabilityManifest,
    Engine, Lookup, Malformed, ManualClock, Registry, State, Submission, Submitted, Write,
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::TestDb;
use campagne_serveur::embedded::EMBEDDED;
use campagne_serveur::migrate;

#[derive(Clone, Copy)]
enum Kind {
    Insert,
    Update,
    Archive,
}

pub struct TestCommand {
    manifest: DataCapabilityManifest,
    kind: Kind,
}

impl DataCapability for TestCommand {
    fn manifest(&self) -> &DataCapabilityManifest {
        &self.manifest
    }

    fn write(
        &self,
        target: Option<Uuid>,
        payload: &Map<String, Value>,
    ) -> Result<Write, Malformed> {
        let aggregate = self.manifest.target.aggregate.clone();
        let change = match self.kind {
            Kind::Insert => Change::Insert(payload.clone()),
            Kind::Update => Change::Update {
                id: target.ok_or(Malformed("target"))?,
                fields: payload.clone(),
            },
            Kind::Archive => Change::Archive {
                id: target.ok_or(Malformed("target"))?,
            },
        };
        Ok(Write { aggregate, change })
    }
}

pub const CREER_CAMPAGNE: &str = "test.creerCampagne@1";
pub const RENOMMER_CAMPAGNE: &str = "test.renommerCampagne@1";
pub const ARCHIVER_CAMPAGNE: &str = "test.archiverCampagne@1";
pub const AJOUTER_PJ: &str = "test.ajouterPj@1";
pub const MODIFIER_PJ: &str = "test.modifierPj@1";
pub const REGLER_NIVEAU: &str = "test.reglerNiveau@1";
pub const ARCHIVER_PJ: &str = "test.archiverPj@1";

fn command(
    name: &str,
    aggregate: &str,
    kind: Kind,
    touches: &[&str],
    mode: &str,
    key: &str,
) -> Arc<dyn DataCapability> {
    let effect = match kind {
        Kind::Insert => "insert",
        Kind::Update | Kind::Archive => "update",
    };
    let manifest = serde_json::from_value(json!({
        "dataCapability": name,
        "owner": "dataguard",
        "version": 1,
        "effect": effect,
        "target": { "aggregate": aggregate },
        "touches": touches,
        "payload": {},
        "mode": mode,
        "invariants": [],
        "permissions": [],
        "callableBy": ["campagne"],
        "idempotencyKey": key,
    }))
    .unwrap();
    Arc::new(TestCommand { manifest, kind })
}

/// Every test command, registered against the embedded aggregates.
pub fn registry() -> Registry {
    let aggregates = Aggregates::embedded().unwrap();
    let mut r = Registry::empty();
    for cap in [
        command(
            "test.creerCampagne",
            "Campagne",
            Kind::Insert,
            &[],
            "relative",
            "required",
        ),
        command(
            "test.renommerCampagne",
            "Campagne",
            Kind::Update,
            &["Campagne.name"],
            "confirm_on_stale",
            "optional",
        ),
        command(
            "test.archiverCampagne",
            "Campagne",
            Kind::Archive,
            &["Campagne.archivedAt"],
            "overwrite",
            "optional",
        ),
        command(
            "test.ajouterPj",
            "PJ",
            Kind::Insert,
            &[],
            "relative",
            "required",
        ),
        command(
            "test.modifierPj",
            "PJ",
            Kind::Update,
            &["PJ.name", "PJ.class", "PJ.level"],
            "confirm_on_stale",
            "optional",
        ),
        command(
            "test.reglerNiveau",
            "PJ",
            Kind::Update,
            &["PJ.level"],
            "overwrite",
            "optional",
        ),
        command(
            "test.archiverPj",
            "PJ",
            Kind::Archive,
            &["PJ.archivedAt"],
            "overwrite",
            "optional",
        ),
    ] {
        r.register(&aggregates, cap).unwrap();
    }
    r
}

pub fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-03-01T20:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A migrated database and an engine on it with every test command, driven
/// by a manual clock.
pub struct Harness {
    pub db: TestDb,
    pub engine: Engine,
    pub clock: Arc<ManualClock>,
}

impl Harness {
    pub async fn new() -> Self {
        Self::with_registry(registry()).await
    }

    pub async fn with_registry(registry: Registry) -> Self {
        let db = TestDb::create().await;
        migrate::run(&db.pool, EMBEDDED).await.unwrap();
        let clock = Arc::new(ManualClock::new(t0()));
        let engine = Engine::new(db.pool.clone(), registry, clock.clone()).unwrap();
        Self { db, engine, clock }
    }

    pub async fn applier(&self) -> Applier {
        self.engine
            .applier()
            .await
            .unwrap()
            .expect("the applier lock is free")
    }

    pub async fn submit(&self, s: Value) -> Submitted {
        self.engine.submit(sub(s)).await.unwrap()
    }

    pub async fn lookup(&self, id: CommandId) -> Lookup {
        self.engine.lookup(id).await.unwrap().unwrap()
    }

    pub async fn result(&self, id: CommandId) -> CommandResult {
        match self.lookup(id).await {
            Lookup::Settled { result } => result,
            pending => panic!("{id} is not settled: {pending:?}"),
        }
    }

    pub async fn state(&self, id: CommandId) -> State {
        self.lookup(id).await.state()
    }

    pub async fn version(&self) -> i64 {
        self.engine.current_version().await.unwrap()
    }

    /// Creates a campaign and applies it; returns its id.
    pub async fn campaign(&self, applier: &mut Applier, name: &str) -> Uuid {
        let s = self
            .submit(json!({
                "dataCapability": CREER_CAMPAGNE,
                "payload": { "name": name },
                "idempotencyKey": Uuid::new_v4().to_string(),
            }))
            .await;
        applier.drain().await.unwrap();
        assert_eq!(self.result(s.command_id).await.status, State::Applied);
        id_of(&s)
    }

    /// Adds a PC and applies it; returns its id.
    pub async fn pc(&self, applier: &mut Applier, campaign: Uuid, name: &str, level: i64) -> Uuid {
        let s = self.add_pc(campaign, name, json!(level)).await;
        applier.drain().await.unwrap();
        assert_eq!(
            self.result(s.command_id).await.status,
            State::Applied,
            "{name}"
        );
        id_of(&s)
    }

    /// Submits an add, not applied.
    pub async fn add_pc(&self, campaign: Uuid, name: &str, level: Value) -> Submitted {
        self.submit(json!({
            "dataCapability": AJOUTER_PJ,
            "payload": {
                "campagneId": campaign.to_string(),
                "name": name,
                "class": "Barde",
                "level": level,
            },
            "idempotencyKey": Uuid::new_v4().to_string(),
        }))
        .await
    }

    pub async fn archive_campaign(&self, campaign: Uuid) -> Submitted {
        self.submit(json!({
            "dataCapability": ARCHIVER_CAMPAGNE,
            "target": { "id": campaign.to_string() },
        }))
        .await
    }

    pub async fn archive_pc(&self, pc: Uuid) -> Submitted {
        self.submit(json!({
            "dataCapability": ARCHIVER_PJ,
            "target": { "id": pc.to_string() },
        }))
        .await
    }

    /// A `confirm_on_stale` edit built on `based_on`.
    pub async fn edit_pc(&self, pc: Uuid, fields: Value, based_on: i64) -> Submitted {
        self.submit(json!({
            "dataCapability": MODIFIER_PJ,
            "target": { "id": pc.to_string() },
            "payload": fields,
            "basedOn": { "version": based_on },
        }))
        .await
    }

    pub async fn set_level(&self, pc: Uuid, level: i64) -> Submitted {
        self.submit(json!({
            "dataCapability": REGLER_NIVEAU,
            "target": { "id": pc.to_string() },
            "payload": { "level": level },
        }))
        .await
    }

    /// Every row of both tables, as text: what "no row changed" compares.
    pub async fn rows(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT 'campagne ' || c::text FROM campagne c
             UNION ALL SELECT 'pj ' || p::text FROM pj p
             ORDER BY 1",
        )
        .fetch_all(&self.db.pool)
        .await
        .unwrap()
    }

    pub async fn pc_row(&self, pc: Uuid) -> (String, i32, Option<DateTime<Utc>>) {
        sqlx::query_as("SELECT nom, niveau, \"archiveLe\" FROM pj WHERE id = $1")
            .bind(pc)
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }

    pub async fn campaign_archived_at(&self, campaign: Uuid) -> Option<DateTime<Utc>> {
        sqlx::query_scalar("SELECT \"archiveLe\" FROM campagne WHERE id = $1")
            .bind(campaign)
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }

    pub async fn queue_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM dataguard_queue")
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }

    /// No active PC belongs to an archived campaign.
    pub async fn assert_no_orphan(&self) {
        let orphans: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pj p JOIN campagne c ON c.id = p.\"campagneId\"
             WHERE p.\"archiveLe\" IS NULL AND c.\"archiveLe\" IS NOT NULL",
        )
        .fetch_one(&self.db.pool)
        .await
        .unwrap();
        assert_eq!(orphans, 0, "an active PC belongs to an archived campaign");
    }

    pub async fn drop_db(self) {
        drop(self.engine);
        self.db.drop_db().await;
    }
}

pub fn sub(s: Value) -> Submission {
    serde_json::from_value(s).unwrap()
}

/// The row id a submission targets or creates.
pub fn id_of(s: &Submitted) -> Uuid {
    Uuid::parse_str(s.partition.split_once('/').unwrap().1).unwrap()
}
