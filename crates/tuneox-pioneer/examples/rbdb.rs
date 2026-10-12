//! Research tool for rekordbox's SQLCipher databases.
//!
//! Without a query, prints every table with its row count and columns.
//! With a query, runs it read-only and prints the rows tab-separated.
//!
//!     cargo run -p tuneox-pioneer --example rbdb -- masterdb .re/fixtures/master.db
//!     cargo run -p tuneox-pioneer --example rbdb -- onelibrary path/to/exportLibrary.db "SELECT ..."

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use tuneox_pioneer::keys;

const USAGE: &str = "usage: rbdb <masterdb|onelibrary> <path> [sql]";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(kind), Some(path)) = (args.next(), args.next()) else {
        return Err(USAGE.into());
    };
    let key = match kind.as_str() {
        "masterdb" => keys::masterdb()?,
        "onelibrary" => keys::onelibrary()?,
        _ => return Err(USAGE.into()),
    };
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.pragma_update(None, "key", key)?;

    match args.next() {
        Some(sql) => print_query(&conn, &sql),
        None => print_schema(&conn),
    }
}

fn print_schema(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let mut tables =
        conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for name in names {
        let rows: i64 = conn.query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| {
            r.get(0)
        })?;
        let mut columns = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
        let columns = columns
            .query_map([&name], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        println!("{name} ({rows} rows)\n    {}", columns.join(", "));
    }
    Ok(())
}

fn print_query(conn: &Connection, sql: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut stmt = conn.prepare(sql)?;
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
