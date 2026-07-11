use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn sqlite(executable: &Path, database: &Path, sql: &str) -> Result<String, String> {
    let output = Command::new(executable)
        .arg("-batch")
        .arg("-noheader")
        .arg(database)
        .arg(sql)
        .output()
        .map_err(|error| format!("SQLite executable failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "SQLite command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("SQLite output was not UTF-8: {error}"))
}

fn sqlite_string(value: &Path) -> String {
    value.to_string_lossy().replace('\'', "''")
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

fn copy_rollback(source: &Path, rollback: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(rollback).map_err(|error| error.to_string())?;
    let paths = source_files(source);
    for path in &paths {
        let name = path
            .file_name()
            .ok_or_else(|| "SQLite source has no file name".to_string())?;
        fs::copy(path, rollback.join(name)).map_err(|error| error.to_string())?;
    }
    Ok(paths)
}

fn integrity(executable: &Path, database: &Path) -> Result<(), String> {
    let result = sqlite(executable, database, "PRAGMA integrity_check;")?;
    if result != "ok" {
        return Err(format!("SQLite integrity check failed: {result}"));
    }
    let foreign_keys = sqlite(executable, database, "PRAGMA foreign_key_check;")?;
    if !foreign_keys.is_empty() {
        return Err(format!("SQLite foreign key check failed: {foreign_keys}"));
    }
    Ok(())
}

fn schema_version(executable: &Path, database: &Path) -> Result<u32, String> {
    sqlite(executable, database, "PRAGMA user_version;")?
        .parse::<u32>()
        .map_err(|error| format!("Invalid SQLite user_version: {error}"))
}

fn table_counts(executable: &Path, database: &Path) -> Result<BTreeMap<String, u64>, String> {
    let names = sqlite(
        executable,
        database,
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name;",
    )?;
    names
        .lines()
        .filter(|name| !name.is_empty())
        .map(|name| {
            let quoted = name.replace('"', "\"\"");
            let count = sqlite(
                executable,
                database,
                &format!("SELECT COUNT(*) FROM \"{quoted}\";"),
            )?
            .parse::<u64>()
            .map_err(|error| format!("Invalid row count for {name}: {error}"))?;
            Ok((name.to_string(), count))
        })
        .collect()
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
    executable: &Path,
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
    let copied = copy_rollback(source, rollback)?;
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
    let checkpoint = sqlite(executable, source, "PRAGMA wal_checkpoint(TRUNCATE);")?;
    if !checkpoint.starts_with("0|") {
        return Err(format!("SQLite WAL checkpoint failed: {checkpoint}"));
    }
    integrity(executable, source)?;
    receipt.source_schema_version = schema_version(executable, source)?;
    receipt.table_counts_before = table_counts(executable, source)?;
    let parent = target
        .parent()
        .ok_or_else(|| "SQLite target has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = target.with_extension(format!("migration.{}.db", uuid::Uuid::new_v4()));
    let vacuum = format!("VACUUM INTO '{}';", sqlite_string(&temporary));
    if let Err(error) = sqlite(executable, source, &vacuum) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let verified = (|| {
        integrity(executable, &temporary)?;
        receipt.target_schema_version = schema_version(executable, &temporary)?;
        receipt.table_counts_after = table_counts(executable, &temporary)?;
        if receipt.source_schema_version != receipt.target_schema_version
            || receipt.table_counts_before != receipt.table_counts_after
        {
            return Err("SQLite target schema or row counts differ".to_string());
        }
        fs::rename(&temporary, target).map_err(|error| error.to_string())?;
        Ok(())
    })();
    if verified.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    verified
}

pub fn migrate_sqlite_verified(
    executable: &Path,
    source: &Path,
    target: &Path,
    rollback: &Path,
    receipt_path: &Path,
    executor_version: &str,
) -> Result<MigrationReceipt, String> {
    let mut receipt = initial_receipt(source, target, rollback, receipt_path, executor_version);
    match execute(
        executable,
        source,
        target,
        rollback,
        receipt_path,
        &mut receipt,
    ) {
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
