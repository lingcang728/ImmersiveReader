use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandRecord {
    pub request_id: String,
    pub command_name: String,
    pub input_hash: String,
    pub task_id: Option<String>,
    pub result_json: Option<String>,
    pub error_code: Option<String>,
    pub resulting_revision: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandClaim {
    New,
    Existing(CommandRecord),
}

pub struct ControlDb {
    connection: Connection,
}

impl ControlDb {
    pub fn open_current() -> Result<Self, String> {
        let locations = crate::storage::StorageLocations::current()?;
        Self::open(&locations.data_root.join(r"App\control.db"))
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Control database has no parent directory".to_string())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                PRAGMA foreign_keys = ON;
                CREATE TABLE IF NOT EXISTS task_snapshots (
                  id TEXT PRIMARY KEY NOT NULL,
                  kind TEXT NOT NULL,
                  revision INTEGER NOT NULL,
                  last_sequence INTEGER NOT NULL,
                  snapshot_json TEXT NOT NULL,
                  updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS task_events (
                  task_id TEXT NOT NULL,
                  sequence INTEGER NOT NULL,
                  revision INTEGER NOT NULL,
                  event_json TEXT NOT NULL,
                  created_at TEXT NOT NULL,
                  PRIMARY KEY (task_id, sequence),
                  FOREIGN KEY (task_id) REFERENCES task_snapshots(id) ON DELETE CASCADE
                );
                CREATE TABLE IF NOT EXISTS command_results (
                  request_id TEXT PRIMARY KEY NOT NULL,
                  command_name TEXT NOT NULL,
                  input_hash TEXT NOT NULL,
                  task_id TEXT,
                  created_at TEXT NOT NULL,
                  completed_at TEXT,
                  result_json TEXT,
                  error_code TEXT,
                  resulting_revision INTEGER
                );
                CREATE TABLE IF NOT EXISTS cache_leases (
                  task_id TEXT PRIMARY KEY NOT NULL,
                  cache_relative_path TEXT NOT NULL,
                  reason TEXT NOT NULL,
                  bytes INTEGER NOT NULL,
                  held INTEGER NOT NULL,
                  updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS engine_instances (
                  engine TEXT PRIMARY KEY NOT NULL,
                  pid INTEGER,
                  port INTEGER,
                  protocol_version INTEGER,
                  status TEXT NOT NULL,
                  started_at TEXT,
                  updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS publish_transaction_index (
                  transaction_id TEXT PRIMARY KEY NOT NULL,
                  task_id TEXT NOT NULL,
                  book_id TEXT NOT NULL,
                  phase TEXT NOT NULL,
                  journal_relative_path TEXT NOT NULL,
                  updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS migration_runs (
                  migration_id TEXT PRIMARY KEY NOT NULL,
                  preview_id TEXT NOT NULL,
                  scope TEXT NOT NULL,
                  status TEXT NOT NULL,
                  receipt_path TEXT,
                  result_json TEXT,
                  created_at TEXT NOT NULL,
                  completed_at TEXT
                );
                PRAGMA user_version = 1;
                "#,
            )
            .map_err(|error| error.to_string())?;
        Ok(Self { connection })
    }

    pub fn claim_command(
        &self,
        request_id: &str,
        command_name: &str,
        input_hash: &str,
    ) -> Result<CommandClaim, String> {
        let existing = self
            .connection
            .query_row(
                "SELECT request_id, command_name, input_hash, task_id, result_json, error_code, resulting_revision FROM command_results WHERE request_id = ?1",
                [request_id],
                |row| {
                    Ok(CommandRecord {
                        request_id: row.get(0)?,
                        command_name: row.get(1)?,
                        input_hash: row.get(2)?,
                        task_id: row.get(3)?,
                        result_json: row.get(4)?,
                        error_code: row.get(5)?,
                        resulting_revision: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(record) = existing {
            if record.command_name != command_name || record.input_hash != input_hash {
                return Err("IDEMPOTENCY_KEY_REUSED".to_string());
            }
            return Ok(CommandClaim::Existing(record));
        }
        self.connection
            .execute(
                "INSERT INTO command_results(request_id, command_name, input_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![request_id, command_name, input_hash, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        Ok(CommandClaim::New)
    }

    pub fn complete_command(
        &self,
        request_id: &str,
        result_json: &str,
        error_code: Option<&str>,
        resulting_revision: Option<i64>,
    ) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE command_results SET completed_at = ?2, result_json = ?3, error_code = ?4, resulting_revision = ?5 WHERE request_id = ?1",
                params![
                    request_id,
                    chrono::Utc::now().to_rfc3339(),
                    result_json,
                    error_code,
                    resulting_revision
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("Command request was not claimed".to_string());
        }
        Ok(())
    }

    pub fn table_names(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|error| error.to_string())?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(names)
    }
}

#[cfg(test)]
mod tests;
