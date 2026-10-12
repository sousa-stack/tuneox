//! Compiles the rekordbox database keys into the crate.
//!
//! The keys never live in the source tree. Each one is taken from an
//! environment variable of the same name (GitHub Actions secrets in CI), or
//! failing that from the gitignored `.keys.env` file at the workspace root.
//! A missing key is not a build error; `keys` reports it at runtime.

use std::path::Path;

const KEYS: [&str; 2] = ["TUNEOX_MASTERDB_KEY", "TUNEOX_ONELIBRARY_KEY"];

fn main() {
    let keys_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.keys.env");
    println!("cargo:rerun-if-changed={}", keys_file.display());
    let file = std::fs::read_to_string(&keys_file).unwrap_or_default();

    for name in KEYS {
        println!("cargo:rerun-if-env-changed={name}");
        let value = std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| {
                file.lines()
                    .filter_map(|line| line.trim().split_once('='))
                    .find(|(k, _)| k.trim() == name)
                    .map(|(_, v)| v.trim().to_string())
            });
        if let Some(value) = value {
            println!("cargo:rustc-env={name}={value}");
        }
    }
}
