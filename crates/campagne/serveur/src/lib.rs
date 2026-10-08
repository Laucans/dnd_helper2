//! The local server of the app: configuration, the migration runner and its
//! embedded migrations, and the loopback HTTP surface.

pub mod config;
pub mod embedded;
pub mod http;
pub mod migrate;
pub mod startup;
