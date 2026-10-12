use rusqlite::Connection;
use tuneox_pioneer::masterdb::{Error, MasterDb};

const KEY: &str = "tuneox-test-key";

fn encrypted_db() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("master.db")).unwrap();
    conn.pragma_update(None, "key", KEY).unwrap();
    conn.execute_batch("CREATE TABLE djmdContent (ID TEXT); INSERT INTO djmdContent VALUES ('1');")
        .unwrap();
    dir
}

#[test]
fn opens_with_the_right_key() {
    let dir = encrypted_db();
    let db = MasterDb::open_with_key(dir.path().join("master.db"), KEY).unwrap();
    assert_eq!(
        db.table_row_counts().unwrap(),
        [("djmdContent".to_string(), 1)]
    );
}

#[test]
fn wrong_key_is_reported_as_locked() {
    let dir = encrypted_db();
    let err = MasterDb::open_with_key(dir.path().join("master.db"), "wrong")
        .err()
        .unwrap();
    assert!(matches!(err, Error::Locked), "{err:?}");
}
