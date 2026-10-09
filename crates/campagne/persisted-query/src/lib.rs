//! The read executor of the `campagne` system and the registry of its
//! persisted queries (`crates/campagne/queries`, contract C).
//!
//! A Capability reads through [`Executor::run`] and nothing else: a registered
//! query name plus its variables. The executor connects with the read-only
//! role only, recomputes the hash of the query files on every call, and sends
//! one statement per call inside a read-only snapshot.

mod executor;
mod operation;
mod registry;

pub use executor::{
    DATABASE_URL_READONLY, Executor, QueryError, StartError, connect_options, registry_dir,
};
