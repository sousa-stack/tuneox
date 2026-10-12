//! Prints the tables (with row counts) and columns of a rekordbox master.db.
//!
//!     TUNEOX_MASTERDB_KEY=... cargo run -p tuneox-pioneer --example masterdb_schema -- path/to/master.db

use tuneox_pioneer::masterdb::MasterDb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: masterdb_schema <path/to/master.db>")?;
    let key = std::env::var("TUNEOX_MASTERDB_KEY")
        .map_err(|_| "set TUNEOX_MASTERDB_KEY to the master.db key")?;

    let db = MasterDb::open(&path, &key)?;
    for (table, rows) in db.table_row_counts()? {
        println!("{table} ({rows} rows)");
        println!("    {}", db.columns(&table)?.join(", "));
    }
    Ok(())
}
