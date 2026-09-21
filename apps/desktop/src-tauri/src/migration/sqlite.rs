use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// P-11-F04: migration no longer shells out to a `sqlite3` CLI found on
/// PATH — bundled rusqlite runs the same checks in-process. `busy_timeout`
/// bounds every lock wait; there is no child process to hang or to inject
/// arguments into.
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReceipt {
    pub schema_version: u32,
    pub migration_id: String,
    pub source_paths: Vec<String>,
    pub target_paths: Vec<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub source_schema_version: u32,
    pub target_schema_version: u32,
    pub table_counts_before: BTreeMap<String, u64>,
    pub table_counts_after: BTreeMap<String, u64>,
    pub non_sensitive_hashes: BTreeMap<String, String>,
    pub executor_version: String,
    pub status: String,
    pub rollback_location: String,
    pub error_code: Option<String>,
}

fn open_checked(database: &Path) -> Result<Connection, String> {
    let connection = Connection::open(database).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn source_files(source: &Path) -> Vec<PathBuf> {
    let mut paths = vec![source.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{suffix}", source.to_string_lossy()));
        if path.exists() {
            paths.push(path);
        }
    }
    paths
}

fn copy_set(source: &Path, directory: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let paths = source_files(source);
    for path in &paths {
        let name = path
            .file_name()
            .ok_or_else(|| "SQLite source has no file name".to_string())?;
        // P-11-F12: copies are recovery evidence and migration input — flush
        // them so a crash cannot leave a zero-filled file under a valid name.
        crate::atomic_file::copy_file_synced(path, &directory.join(name))
            .map_err(|error| error.to_string())?;
    }
    Ok(paths)
}

fn integrity(connection: &Connection) -> Result<(), String> {
    let result: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if result != "ok" {
        return Err(format!("SQLite integrity check failed: {result}"));
    }
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| error.to_string())?;
    let mut rows = statement.query([]).map_err(|error| error.to_string())?;
    if let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let detail = (0..row.as_ref().column_count())
            .map(|index| row.get::<_, String>(index).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|");
        return Err(format!("SQLite foreign key check failed: {detail}"));
    }
    Ok(())
}

fn schema_version(connection: &Connection) -> Result<u32, String> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
        .map_err(|error| error.to_string())
}

fn table_counts(connection: &Connection) -> Result<BTreeMap<String, u64>, String> {
    let names = {
        let mut statement = connection
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|error| error.to_string())?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        names
    };
    let mut counts = BTreeMap::new();
    for name in names {
        let quoted = name.replace('"', "\"\"");
        // SQLite integers are i64 — rusqlite has no FromSql for u64.
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM \"{quoted}\""), [], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?;
        counts.insert(name, u64::try_from(count).unwrap_or(0));
    }
    Ok(counts)
}

fn write_receipt(path: &Path, receipt: &MigrationReceipt) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    crate::atomic_file::write(path, &data)
}

fn migration_id(receipt_path: &Path) -> String {
    receipt_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or("sqlite-migration")
        .to_string()
}

fn initial_receipt(
    source: &Path,
    target: &Path,
    rollback: &Path,
    receipt_path: &Path,
    executor_version: &str,
) -> MigrationReceipt {
    MigrationReceipt {
        schema_version: 1,
        migration_id: migration_id(receipt_path),
        source_paths: source_files(source)
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        target_paths: vec![target.to_string_lossy().into_owned()],
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        source_schema_version: 0,
        target_schema_version: 0,
        table_counts_before: BTreeMap::new(),
        table_counts_after: BTreeMap::new(),
        non_sensitive_hashes: BTreeMap::new(),
        executor_version: executor_version.to_string(),
        status: "failed".to_string(),
        rollback_location: rollback.to_string_lossy().into_owned(),
        error_code: Some("MIGRATION_FAILED".to_string()),
    }
}

fn execute(
    source: &Path,
    target: &Path,
    rollback: &Path,
    receipt_path: &Path,
    receipt: &mut MigrationReceipt,
) -> Result<(), String> {
    if !source.is_file() {
        return Err("SQLite source database does not exist".to_string());
    }
    if target.exists() {
        return Err("SQLite target database already exists".to_string());
    }
    let copied = copy_set(source, rollback)?;
    receipt.source_paths = copied
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    if let Ok(hash) = crate::publish::hash_file(source) {
        receipt
            .non_sensitive_hashes
            .insert("sourceDatabaseSha256".to_string(), hash);
    }
    write_receipt(receipt_path, receipt)?;
    let parent = target
        .parent()
        .ok_or_else(|| "SQLite target has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;

    // P-11-F06/F-07: SQLite never opens the source file itself. A source in
    // WAL mode can carry live or stale -wal/-shm sidecars — opening it
    // directly would replay (and possibly checkpoint on close) against the
    // file this migration promised to leave read-only. Instead the
    // db+wal+shm set is copied into a private work dir; SQLite's own WAL
    // recovery runs on the COPY (salt/checksum validation ignores torn or
    // stale frames), and VACUUM INTO consolidates the result. The source is
    // only ever touched by fs::copy.
    let work_root = parent.join(format!("migration-work-{}", uuid::Uuid::new_v4()));
    let outcome = (|| -> Result<(), String> {
        copy_set(source, &work_root)?;
        let work_name = source
            .file_name()
            .ok_or_else(|| "SQLite source has no file name".to_string())?;
        let work_db = work_root.join(work_name);
        let work = open_checked(&work_db)?;
        integrity(&work)?;
        receipt.source_schema_version = schema_version(&work)?;
        receipt.table_counts_before = table_counts(&work)?;
        let temporary = target.with_extension(format!("migration.{}.db", uuid::Uuid::new_v4()));
        work.execute("VACUUM INTO ?1", [temporary.to_string_lossy().as_ref()])
            .map_err(|error| error.to_string())?;
        drop(work);
        let verified = (|| {
            let check = open_checked(&temporary)?;
            integrity(&check)?;
            receipt.target_schema_version = schema_version(&check)?;
            receipt.table_counts_after = table_counts(&check)?;
            drop(check);
            if receipt.source_schema_version != receipt.target_schema_version
                || receipt.table_counts_before != receipt.table_counts_after
            {
                return Err("SQLite target schema or row counts differ".to_string());
            }
            // P-11-F07: re-check the commit point — the early `target.exists()`
            // gate ran before VACUUM, and a stale `-wal`/`-shm`/`‑journal`
            // sibling left by a deleted same-name database would be replayed
            // against the fresh file on first open (silent corruption).
            if target.exists() {
                return Err("SQLite target database already exists".to_string());
            }
            for suffix in ["-wal", "-shm", "-journal"] {
                let mut name = target.as_os_str().to_os_string();
                name.push(suffix);
                if Path::new(&name).exists() {
                    return Err(format!(
                        "SQLite target has a stale {suffix} sidecar — remove it before migrating"
                    ));
                }
            }
            fs::rename(&temporary, target).map_err(|error| error.to_string())?;
            Ok(())
        })();
        if verified.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        verified
    })();
    // The work copy is scratch space — always remove it; the pristine set in
    // `rollback` remains the recovery evidence.
    let _ = fs::remove_dir_all(&work_root);
    outcome
}

pub fn migrate_sqlite_verified(
    source: &Path,
    target: &Path,
    rollback: &Path,
    receipt_path: &Path,
    executor_version: &str,
) -> Result<MigrationReceipt, String> {
    let mut receipt = initial_receipt(source, target, rollback, receipt_path, executor_version);
    match execute(source, target, rollback, receipt_path, &mut receipt) {
        Ok(()) => {
            receipt.status = "success".to_string();
            receipt.error_code = None;
            receipt.completed_at = Some(chrono::Utc::now().to_rfc3339());
            if let Err(error) = write_receipt(receipt_path, &receipt) {
                let _ = fs::remove_file(target);
                return Err(format!("Migration receipt could not be committed: {error}"));
            }
            Ok(receipt)
        }
        Err(error) => {
            receipt.completed_at = Some(chrono::Utc::now().to_rfc3339());
            let _ = write_receipt(receipt_path, &receipt);
            Err(error)
        }
    }
}
