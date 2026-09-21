use super::migrate_sqlite_verified;
use std::fs;
use std::path::{Path, PathBuf};

fn root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("immersive-sqlite-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("test root must exist");
    path
}

fn sqlite_exec(database: &Path, sql: &str) {
    let connection = rusqlite::Connection::open(database).expect("fixture db must open");
    connection.execute_batch(sql).expect("fixture sql must run");
}

fn sqlite_scalar(database: &Path, sql: &str) -> String {
    use rusqlite::types::Value;
    let connection = rusqlite::Connection::open(database).expect("fixture db must open");
    let value = connection
        .query_row(sql, [], |row| row.get::<_, Value>(0))
        .expect("scalar query must return a row");
    match value {
        Value::Integer(integer) => integer.to_string(),
        Value::Real(real) => real.to_string(),
        Value::Text(text) => text,
        Value::Blob(_) => "<blob>".to_string(),
        Value::Null => "<null>".to_string(),
    }
}

#[test]
fn verified_migration_preserves_schema_version_and_counts() {
    let root = root("success");
    let source = root.join("legacy.db");
    let target = root.join(r"Data\Zhihu\zhihu-packer.db");
    let rollback = root.join(r"Data\Migrations\run-1\rollback");
    let receipt = root.join(r"Data\Migrations\run-1\receipt.json");
    sqlite_exec(
        &source,
        "PRAGMA journal_mode=WAL; CREATE TABLE items(id INTEGER PRIMARY KEY, title TEXT NOT NULL); INSERT INTO items(title) VALUES ('one'),('two'); PRAGMA user_version=7;",
    );

    let result = migrate_sqlite_verified(&source, &target, &rollback, &receipt, "1.1.0-test")
        .expect("verified migration must succeed");

    assert_eq!(result.status, "success");
    assert_eq!(result.source_schema_version, 7);
    assert_eq!(result.target_schema_version, 7);
    assert_eq!(result.table_counts_before.get("items"), Some(&2));
    assert_eq!(result.table_counts_before, result.table_counts_after);
    assert_eq!(sqlite_scalar(&target, "PRAGMA integrity_check;"), "ok");
    assert!(rollback.join("legacy.db").exists());
    assert!(receipt.exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn corrupt_source_never_creates_authoritative_target() {
    let root = root("corrupt");
    let source = root.join("legacy.db");
    let target = root.join(r"Data\Zhihu\zhihu-packer.db");
    let rollback = root.join(r"Data\Migrations\run-2\rollback");
    let receipt = root.join(r"Data\Migrations\run-2\receipt.json");
    fs::write(&source, b"not a sqlite database").expect("corrupt fixture must write");

    let error = migrate_sqlite_verified(&source, &target, &rollback, &receipt, "1.1.0-test")
        .expect_err("corrupt source must fail migration");

    assert!(
        error.contains("integrity") || error.contains("SQLite") || error.contains("database"),
        "unexpected error: {error}"
    );
    assert!(!target.exists());
    assert_eq!(
        fs::read(&source).expect("source must remain"),
        b"not a sqlite database"
    );
    assert!(rollback.join("legacy.db").exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn migration_reads_through_a_live_wal() {
    // A WAL-mode source whose frames have not been checkpointed must still
    // migrate completely: the -wal/-shm sidecars travel with the copy and
    // SQLite recovers them on the working copy — the source stays untouched.
    let root = root("wal");
    let source = root.join("legacy.db");
    let target = root.join(r"Data\Zhihu\zhihu-packer.db");
    let rollback = root.join(r"Data\Migrations\run-3\rollback");
    let receipt = root.join(r"Data\Migrations\run-3\receipt.json");
    // Keep the writer connection open so the WAL is not auto-checkpointed
    // away before the migration copies the file set.
    let connection = rusqlite::Connection::open(&source).expect("fixture db must open");
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
             CREATE TABLE items(id INTEGER PRIMARY KEY, title TEXT NOT NULL);
             INSERT INTO items(title) VALUES ('wal-one'),('wal-two');
             PRAGMA user_version=3;",
        )
        .expect("fixture sql must run");
    assert!(
        PathBuf::from(format!("{}-wal", source.to_string_lossy())).exists(),
        "fixture must leave a live -wal file"
    );

    let result = migrate_sqlite_verified(&source, &target, &rollback, &receipt, "1.1.0-test")
        .expect("migration must read through the live WAL");

    assert_eq!(result.status, "success");
    assert_eq!(result.source_schema_version, 3);
    assert_eq!(result.table_counts_before.get("items"), Some(&2));
    assert_eq!(sqlite_scalar(&target, "SELECT COUNT(*) FROM items"), "2");
    assert!(rollback.join("legacy.db-wal").exists());
    drop(connection);
    fs::remove_dir_all(root).expect("fixture must be removed");
}
