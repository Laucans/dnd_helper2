//! Embeds every file of `migrations/` in the binary, sorted by name. The
//! runner, not this script, validates the names: a misnamed file is a
//! startup error that names it.
//!
//! Also lists the DataCapabilities the server registers, by discovering the
//! folders of `crates/dataguard/data-capabilities/`: a new DataCapability is
//! a folder plus its line in this crate's `[dependencies]`, never an edit of
//! `startup.rs` — so two of them land in parallel without meeting there.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=migrations");

    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("migrations");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            // Hidden files (.DS_Store, editor swap files) and editor backups
            // are not migrations; every other file is embedded and validated.
            let stray = name.starts_with('.') || name.ends_with('~');
            (entry.file_type().unwrap().is_file() && !stray).then(|| {
                println!("cargo:rerun-if-changed=migrations/{name}");
                name
            })
        })
        .collect();
    names.sort();

    let mut out = String::from("pub static EMBEDDED: &[crate::migrate::Migration<'static>] = &[\n");
    for name in &names {
        out.push_str(&format!(
            "    crate::migrate::Migration {{ name: {name:?}, bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/migrations/\", {name:?})) }},\n"
        ));
    }
    out.push_str("];\n");

    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("migrations.rs"),
        out,
    )
    .unwrap();

    data_capabilities();
}

/// `(KEY, capability())` of every DataCapability under
/// `crates/dataguard/data-capabilities/`, each by its crate name, sorted —
/// what `startup::registry` registers.
fn data_capabilities() {
    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../dataguard/data-capabilities");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            let path = entry.path();
            path.join("Cargo.toml").is_file().then(|| {
                println!(
                    "cargo:rerun-if-changed={}",
                    path.join("Cargo.toml").display()
                );
                // The crate's name, as `[package] name = "…"` has it, as a Rust identifier.
                let manifest = fs::read_to_string(path.join("Cargo.toml")).unwrap();
                manifest
                    .lines()
                    .find_map(|line| {
                        line.trim()
                            .strip_prefix("name = ")
                            .map(|n| n.trim_matches('"').replace('-', "_"))
                    })
                    .unwrap_or_else(|| entry.file_name().into_string().unwrap().replace('-', "_"))
            })
        })
        .collect();
    names.sort();

    let mut out = String::from(
        "/// One DataCapability as its crate offers it: its key, and the capability or why its manifest does not deserialise.\n\
         pub type Offered = (&'static str, Result<std::sync::Arc<dyn dataguard::DataCapability>, serde_json::Error>);\n\
         /// Every DataCapability under crates/dataguard/data-capabilities/, discovered at build time.\n\
         pub fn data_capabilities() -> Vec<Offered> {\n    vec![\n",
    );
    for name in &names {
        out.push_str(&format!("        ({name}::KEY, {name}::capability()),\n"));
    }
    out.push_str("    ]\n}\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("data_capabilities.rs"),
        out,
    )
    .unwrap();
}
