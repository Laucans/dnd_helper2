//! The write side of the app: the DataQueue (a PostgreSQL table), the
//! DataGuard that checks every command against the invariants of its
//! aggregate, and the Resolver that carries an archive along the relations
//! that ask for it.
//!
//! Only DataCapabilities write, and only through [`Engine::submit`]. A
//! command queues in its partition (`<Aggregate>/<id>`), is never rejected
//! for concurrency, and is applied by the single [`Applier`], which issues
//! the global `dataVersion`.
//!
//! Errors and logs carry invariant ids and SQLSTATEs, never a server message:
//! a driver error can quote a role, a host or a database name.

pub mod applier;
pub mod clock;
pub mod engine;
pub mod error;
pub mod guard;
pub mod invariants;
pub mod manifest;
pub mod model;
pub mod notify;
pub mod queue;
pub mod registry;
pub mod resolver;
pub mod store;
pub mod text;

pub use applier::Applier;
pub use clock::{Clock, ManualClock, SystemClock};
pub use engine::Engine;
pub use error::{EngineError, HoldError, OperationError, SubmitError};
pub use manifest::{AggregateManifest, Aggregates, DataCapabilityManifest, ManifestError};
pub use model::{
    BasedOn, Change, CommandId, CommandResult, Hold, Lookup, Malformed, QueueEntry, State,
    Submission, Submitted, Target, Write,
};
pub use notify::{DATA_VERSION_CHANNEL, VersionFeed};
pub use registry::{DataCapability, RegistrationError, Registry};

/// The `by` of every command: one GM per install, no account (milestone rule
/// 52). Set by the engine; a value a caller supplies is ignored.
pub const GM_IDENTITY: &str = "mj-local";
