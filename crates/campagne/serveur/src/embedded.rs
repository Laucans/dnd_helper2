//! The migrations of `migrations/`, and the DataCapabilities of
//! `crates/dataguard/data-capabilities/`, both listed at build time by
//! `build.rs`.

include!(concat!(env!("OUT_DIR"), "/migrations.rs"));
include!(concat!(env!("OUT_DIR"), "/data_capabilities.rs"));
