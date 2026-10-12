//! Runs one read-only SQL query against a rekordbox master.db and prints the
//! rows tab-separated. Research tool for mapping the schema.
//!
//!     TUNEOX_MASTERDB_KEY=... cargo run -p tuneox-pioneer --example masterdb_query -- path/to/master.db "SELECT ..."

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(sql)) = (args.next(), args.next()) else {
        return Err("usage: masterdb_query <path/to/master.db> <sql>".into());
    };
    let key = std::env::var("TUNEOX_MASTERDB_KEY")
        .map_err(|_| "set TUNEOX_MASTERDB_KEY to the master.db key")?;

    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.pragma_update(None, "key", &key)?;

    let mut stmt = conn.prepare(&sql)?;
    let width = stmt.column_count();
    println!("{}", stmt.column_names().join("\t"));
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let cells = (0..width)
            .map(|i| match row.get_ref(i) {
                Ok(ValueRef::Null) => "NULL".to_string(),
                Ok(ValueRef::Integer(v)) => v.to_string(),
                Ok(ValueRef::Real(v)) => v.to_string(),
                Ok(ValueRef::Text(v)) => String::from_utf8_lossy(v).into_owned(),
                Ok(ValueRef::Blob(v)) => format!("<blob {} bytes>", v.len()),
                Err(e) => format!("<{e}>"),
            })
            .collect::<Vec<_>>();
        println!("{}", cells.join("\t"));
    }
    Ok(())
}
