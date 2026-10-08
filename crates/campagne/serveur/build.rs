//! Embeds every file of `migrations/` in the binary, sorted by name. The
//! runner, not this script, validates the names: a misnamed file is a
//! startup error that names it.

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
}
