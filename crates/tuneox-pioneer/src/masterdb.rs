//! The local rekordbox 6/7 library database, `master.db`.
//!
//! `master.db` is a SQLCipher database. TuneOx only imports from it, so it is
//! always opened read-only.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::keys::{self, MissingKey};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    MissingKey(#[from] MissingKey),
    #[error("could not unlock master.db: wrong key, or not a rekordbox database")]
    Locked,
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct MasterDb {
    conn: Connection,
}

impl MasterDb {
    /// Opens `master.db` with the built-in key.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_key(path, keys::masterdb()?)
    }

    pub fn open_with_key(path: impl AsRef<Path>, key: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "key", key)?;
        // SQLCipher doesn't check the key until the first read.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .map_err(|_| Error::Locked)?;
        Ok(Self { conn })
    }

    /// Every table in the database with its row count, for schema research.
    pub fn table_row_counts(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        names
            .into_iter()
            .map(|name| {
                let count = self.conn.query_row(
                    &format!("SELECT count(*) FROM \"{name}\""),
                    [],
                    |row| row.get(0),
                )?;
                Ok((name, count))
            })
            .collect()
    }

    /// Column names of one table, for schema research.
    pub fn columns(&self, table: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM pragma_table_info(?1)")?;
        let columns = stmt
            .query_map([table], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(columns)
    }
}
