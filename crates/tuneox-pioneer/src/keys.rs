//! SQLCipher keys for rekordbox's databases.
//!
//! Both are fixed values, the same in every rekordbox install. They are
//! compiled in by `build.rs` from the environment or a gitignored `.keys.env`,
//! so a build made without them still works but can't open these databases.

#[derive(Debug, thiserror::Error)]
#[error("TuneOx was built without {0}; add it to .keys.env or the build environment")]
pub struct MissingKey(&'static str);

/// Key for the local rekordbox 6/7 library, `master.db`.
pub fn masterdb() -> Result<&'static str, MissingKey> {
    option_env!("TUNEOX_MASTERDB_KEY").ok_or(MissingKey("TUNEOX_MASTERDB_KEY"))
}

/// Key for OneLibrary USB exports, `exportLibrary.db`.
pub fn onelibrary() -> Result<&'static str, MissingKey> {
    option_env!("TUNEOX_ONELIBRARY_KEY").ok_or(MissingKey("TUNEOX_ONELIBRARY_KEY"))
}
