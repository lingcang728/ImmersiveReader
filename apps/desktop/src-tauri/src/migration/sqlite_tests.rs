use super::migrate_sqlite_verified;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("immersive-sqlite-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("test root must exist");
    path
}

fn sqlite(database: &Path, sql: &str) -> String {
    let output = Command::new("sqlite3")
        .arg(database)
        .arg(sql)
        .output()
        .expect("installed sqlite3 must start");
    assert!(
        output.status.success(),
        "sqlite failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("sqlite output must be utf8")
        .trim()
        .to_string()
}

#[test]
fn verified_migration_preserves_schema_version_and_counts() {
    let root = root("success");
    let source = root.join("legacy.db");
    let target = root.join(r"Data\Zhihu\zhihu-packer.db");
    let rollback = root.join(r"Data\Migrations\run-1\rollback");
    let receipt = root.join(r"Data\Migrations\run-1\receipt.json");
    sqlite(
        &source,
        "PRAGMA journal_mode=WAL; CREATE TABLE items(id INTEGER PRIMARY KEY, title TEXT NOT NULL); INSERT INTO items(title) VALUES ('one'),('two'); PRAGMA user_version=7;",
    );

    let result = migrate_sqlite_verified(
        Path::new("sqlite3"),
        &source,
        &target,
        &rollback,
        &receipt,
        "1.1.0-test",
    )
    .expect("verified migration must succeed");

    assert_eq!(result.status, "success");
    assert_eq!(result.source_schema_version, 7);
    assert_eq!(result.target_schema_version, 7);
    assert_eq!(result.table_counts_before.get("items"), Some(&2));
    assert_eq!(result.table_counts_before, result.table_counts_after);
    assert_eq!(sqlite(&target, "PRAGMA integrity_check;"), "ok");
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

    let error = migrate_sqlite_verified(
        Path::new("sqlite3"),
        &source,
        &target,
        &rollback,
        &receipt,
        "1.1.0-test",
    )
    .expect_err("corrupt source must fail migration");

    assert!(error.contains("integrity") || error.contains("SQLite"));
    assert!(!target.exists());
    assert_eq!(
        fs::read(&source).expect("source must remain"),
        b"not a sqlite database"
    );
    assert!(rollback.join("legacy.db").exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}
