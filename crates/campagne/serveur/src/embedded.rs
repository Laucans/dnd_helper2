//! The migrations of `migrations/`, embedded at build time by `build.rs`.

include!(concat!(env!("OUT_DIR"), "/migrations.rs"));
