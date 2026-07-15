use crate::tasks::{
    LifecycleState, RequiredAction, TaskErrorCode, TaskEvent, TaskKind, TaskOutcome, TaskSnapshot,
};
use rusqlite::{params, Connection, Error as SqliteError, ErrorCode, OptionalExtension};
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRunRecord {
    pub migration_id: String,
    pub preview_id: String,
    pub scope: String,
    pub status: String,
    pub receipt_path: Option<String>,
    pub result_json: Option<String>,
}

pub struct ControlDb {
    connection: Connection,
}

fn is_retryable_initialization_lock(error: &SqliteError) -> bool {
    matches!(
        error,
        SqliteError::SqliteFailure(failure, _)
            if matches!(failure.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// Repair podcast tasks that were interrupted before a task contract existed.
pub fn repair_orphaned_podcast_tasks() -> Result<u32, String> {
    let locations = crate::storage::StorageLocations::current()?;
    let mut control = ControlDb::open_current()?;
    control.repair_orphaned_podcast_tasks_at(&locations.data_root)
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
        let schema = r#"
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
                "#;
        let mut last_lock_error = None;
        for attempt in 0..=8 {
            match connection.execute_batch(schema) {
                Ok(()) => return Ok(Self { connection }),
                Err(error) if is_retryable_initialization_lock(&error) && attempt < 8 => {
                    last_lock_error = Some(error.to_string());
                    std::thread::sleep(Duration::from_millis(25 * (attempt + 1)));
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Err(last_lock_error.unwrap_or_else(|| "Control database initialization failed".to_string()))
    }

    pub fn claim_command(
        &self,
        request_id: &str,
        command_name: &str,
        input_hash: &str,
    ) -> Result<CommandClaim, String> {
        let inserted = self
            .connection
            .execute(
                "INSERT INTO command_results(request_id, command_name, input_hash, created_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(request_id) DO NOTHING",
                params![request_id, command_name, input_hash, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        if inserted == 1 {
            return Ok(CommandClaim::New);
        }
        let record = self
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
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "COMMAND_CLAIM_MISSING".to_string())?;
        {
            if record.command_name != command_name || record.input_hash != input_hash {
                return Err("IDEMPOTENCY_KEY_REUSED".to_string());
            }
        }
        Ok(CommandClaim::Existing(record))
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

    pub fn begin_migration_run(
        &self,
        migration_id: &str,
        preview_id: &str,
        scope: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO migration_runs(migration_id, preview_id, scope, status, created_at) VALUES (?1, ?2, ?3, 'running', ?4)",
                params![
                    migration_id,
                    preview_id,
                    scope,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn complete_migration_run(
        &self,
        migration_id: &str,
        status: &str,
        receipt_path: Option<&str>,
        result_json: &str,
    ) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE migration_runs SET status = ?2, receipt_path = ?3, result_json = ?4, completed_at = ?5 WHERE migration_id = ?1",
                params![
                    migration_id,
                    status,
                    receipt_path,
                    result_json,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("Migration run was not started".to_string());
        }
        Ok(())
    }

    pub fn record_publish_transaction(
        &self,
        transaction_id: &str,
        task_id: &str,
        book_id: &str,
        phase: &str,
        journal_relative_path: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO publish_transaction_index(transaction_id, task_id, book_id, phase, journal_relative_path, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(transaction_id) DO UPDATE SET task_id=excluded.task_id, book_id=excluded.book_id, phase=excluded.phase, journal_relative_path=excluded.journal_relative_path, updated_at=excluded.updated_at",
                params![
                    transaction_id,
                    task_id,
                    book_id,
                    phase,
                    journal_relative_path,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn migration_run(&self, migration_id: &str) -> Result<Option<MigrationRunRecord>, String> {
        self.connection
            .query_row(
                "SELECT migration_id, preview_id, scope, status, receipt_path, result_json FROM migration_runs WHERE migration_id = ?1",
                [migration_id],
                |row| {
                    Ok(MigrationRunRecord {
                        migration_id: row.get(0)?,
                        preview_id: row.get(1)?,
                        scope: row.get(2)?,
                        status: row.get(3)?,
                        receipt_path: row.get(4)?,
                        result_json: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn migration_runs(&self) -> Result<Vec<MigrationRunRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT migration_id, preview_id, scope, status, receipt_path, result_json FROM migration_runs ORDER BY created_at DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(MigrationRunRecord {
                    migration_id: row.get(0)?,
                    preview_id: row.get(1)?,
                    scope: row.get(2)?,
                    status: row.get(3)?,
                    receipt_path: row.get(4)?,
                    result_json: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn persist_task_event(&mut self, event: &TaskEvent) -> Result<(), String> {
        if event.schema_version != 1
            || event.task_id != event.snapshot.id
            || event.sequence != event.snapshot.last_sequence
            || event.revision != event.snapshot.revision
        {
            return Err("INVALID_TASK_EVENT".to_string());
        }
        let event_sequence =
            i64::try_from(event.sequence).map_err(|_| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        let event_revision =
            i64::try_from(event.revision).map_err(|_| "INVALID_TASK_REVISION".to_string())?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let current = transaction
            .query_row(
                "SELECT revision, last_sequence FROM task_snapshots WHERE id = ?1",
                [&event.task_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        match current {
            Some((revision, sequence)) => {
                if event_sequence != sequence + 1 {
                    return Err("EVENT_SEQUENCE_CONFLICT".to_string());
                }
                if event_revision <= revision {
                    return Err("REVISION_CONFLICT".to_string());
                }
            }
            None if event.sequence != 1 || event.revision != 1 => {
                return Err("EVENT_SEQUENCE_CONFLICT".to_string());
            }
            None => {}
        }
        let snapshot_json =
            serde_json::to_string(&event.snapshot).map_err(|error| error.to_string())?;
        let event_json = serde_json::to_string(event).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO task_snapshots(id, kind, revision, last_sequence, snapshot_json, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, revision=excluded.revision, last_sequence=excluded.last_sequence, snapshot_json=excluded.snapshot_json, updated_at=excluded.updated_at",
                params![
                    event.task_id,
                    match &event.snapshot.kind {
                        crate::tasks::TaskKind::Podcast => "podcast",
                        crate::tasks::TaskKind::Zhihu => "zhihu",
                    },
                    event_revision,
                    event_sequence,
                    snapshot_json,
                    event.snapshot.updated_at
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO task_events(task_id, sequence, revision, event_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    event.task_id,
                    event_sequence,
                    event_revision,
                    event_json,
                    event.created_at
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn task_snapshot(&self, task_id: &str) -> Result<Option<TaskSnapshot>, String> {
        let json = self
            .connection
            .query_row(
                "SELECT snapshot_json FROM task_snapshots WHERE id = ?1",
                [task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        json.map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .transpose()
    }

    pub fn task_snapshots(
        &self,
        kind: Option<crate::tasks::TaskKind>,
    ) -> Result<Vec<TaskSnapshot>, String> {
        let kind = kind.map(|value| match value {
            crate::tasks::TaskKind::Podcast => "podcast",
            crate::tasks::TaskKind::Zhihu => "zhihu",
        });
        let mut statement = self
            .connection
            .prepare(
                "SELECT snapshot_json FROM task_snapshots WHERE (?1 IS NULL OR kind = ?1) ORDER BY updated_at DESC, id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([kind], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let json = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&json).map_err(|error| error.to_string())
        })
        .collect()
    }

    /// Drop terminal task history older than `max_age_days` (events cascade-delete).
    pub fn prune_terminal_tasks_older_than(&mut self, max_age_days: i64) -> Result<u32, String> {
        if max_age_days <= 0 {
            return Ok(0);
        }
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days)).to_rfc3339();
        let snapshots = self.task_snapshots(None)?;
        let mut removed = 0_u32;
        for snapshot in snapshots {
            if snapshot.lifecycle_state != LifecycleState::Terminal {
                continue;
            }
            if snapshot.updated_at.as_str() >= cutoff.as_str() {
                continue;
            }
            self.connection
                .execute("DELETE FROM task_snapshots WHERE id = ?1", [&snapshot.id])
                .map_err(|error| error.to_string())?;
            removed = removed.saturating_add(1);
        }
        Ok(removed)
    }

    pub fn task_snapshots_for_book(&self, book_id: &str) -> Result<Vec<TaskSnapshot>, String> {
        if book_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .task_snapshots(None)?
            .into_iter()
            .filter(|snapshot| snapshot.book_id.as_deref() == Some(book_id))
            .collect())
    }

    pub fn record_engine_instance(
        &self,
        engine: &str,
        pid: u32,
        port: Option<u16>,
        protocol_version: Option<u32>,
        started_at: &str,
    ) -> Result<(), String> {
        if engine.trim().is_empty() || pid == 0 {
            return Err("INVALID_ENGINE_INSTANCE".to_string());
        }
        self.connection
            .execute(
                "INSERT INTO engine_instances(engine, pid, port, protocol_version, status, started_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6) ON CONFLICT(engine) DO UPDATE SET pid=excluded.pid, port=excluded.port, protocol_version=excluded.protocol_version, status='running', started_at=excluded.started_at, updated_at=excluded.updated_at",
                params![
                    engine,
                    i64::from(pid),
                    port.map(i64::from),
                    protocol_version.map(i64::from),
                    started_at,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn mark_engine_crashed(
        &mut self,
        engine: &str,
        pid: u32,
        exit_code: Option<i32>,
    ) -> Result<bool, String> {
        let instance = self
            .connection
            .query_row(
                "SELECT pid, status FROM engine_instances WHERE engine = ?1",
                [engine],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((stored_pid, status)) = instance else {
            return Ok(false);
        };
        if status != "running" || stored_pid != Some(i64::from(pid)) {
            return Ok(false);
        }
        let kind = task_kind_for_engine(engine)?;
        let snapshots = self.task_snapshots(Some(kind))?;
        for snapshot in snapshots {
            if is_active_task(&snapshot.lifecycle_state) {
                self.persist_task_event(&interrupted_event(snapshot, exit_code)?)?;
            }
        }
        let changed = self
            .connection
            .execute(
                "UPDATE engine_instances SET status = 'interrupted', updated_at = ?2 WHERE engine = ?1 AND pid = ?3 AND status = 'running'",
                params![engine, chrono::Utc::now().to_rfc3339(), i64::from(pid)],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    pub fn recover_stale_engine_instances(&mut self) -> Result<u32, String> {
        let stale: Vec<(String, u32)> = {
            let mut statement = self
                .connection
                .prepare("SELECT engine, pid FROM engine_instances WHERE status = 'running' AND pid IS NOT NULL")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|error| error.to_string())?;
            rows.map(|row| {
                let (engine, pid) = row.map_err(|error| error.to_string())?;
                let pid = u32::try_from(pid).map_err(|_| "INVALID_ENGINE_INSTANCE".to_string())?;
                Ok((engine, pid))
            })
            .collect::<Result<Vec<_>, String>>()?
        };
        let mut recovered = 0;
        for (engine, pid) in stale {
            if self.mark_engine_crashed(&engine, pid, None)? {
                recovered += 1;
            }
        }
        Ok(recovered)
    }

    /// Mark podcast tasks that never got a task contract / input copy as explicit
    /// INPUT_COPY_FAILED (not auto-recoverable). Idempotent for already-failed rows.
    pub fn repair_orphaned_podcast_tasks_at(
        &mut self,
        data_root: &Path,
    ) -> Result<u32, String> {
        let snapshots = self.task_snapshots(Some(TaskKind::Podcast))?;
        let mut repaired = 0u32;
        for snapshot in snapshots {
            if snapshot.lifecycle_state == LifecycleState::Terminal {
                continue;
            }
            let contract = data_root
                .join("Podcast")
                .join("Tasks")
                .join(&snapshot.id)
                .join("task.json");
            if contract.is_file() {
                continue;
            }
            let mut next = snapshot;
            let now = chrono::Utc::now().to_rfc3339();
            next.last_sequence = next
                .last_sequence
                .checked_add(1)
                .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
            next.revision = next
                .revision
                .checked_add(1)
                .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
            next.lifecycle_state = LifecycleState::Terminal;
            next.outcome = TaskOutcome::Failed;
            next.error_code = Some(TaskErrorCode::InputCopyFailed);
            next.error_message = Some(
                "任务缺少输入副本与任务合同，无法自动恢复。请重新选择原音频文件。".to_string(),
            );
            next.engine_stage = "input_copy_failed".to_string();
            next.engine_status = "exited".to_string();
            next.recoverable = false;
            next.can_pause = false;
            next.can_resume = false;
            next.can_retry = false;
            next.can_cancel = false;
            next.updated_at = now.clone();
            next.last_heartbeat_at = Some(now.clone());
            let event = TaskEvent {
                schema_version: 1,
                task_id: next.id.clone(),
                sequence: next.last_sequence,
                revision: next.revision,
                event_type: "input_copy_failed".to_string(),
                snapshot: next,
                created_at: now,
            };
            self.persist_task_event(&event)?;
            repaired = repaired.saturating_add(1);
        }
        Ok(repaired)
    }

    pub fn cancel_active_tasks(&mut self) -> Result<Vec<String>, String> {
        let snapshots = self.task_snapshots(None)?;
        let mut podcast_task_ids = Vec::new();
        for snapshot in snapshots {
            if !is_active_task(&snapshot.lifecycle_state) {
                continue;
            }
            if matches!(snapshot.kind, TaskKind::Podcast) {
                podcast_task_ids.push(snapshot.id.clone());
            }
            self.persist_task_event(&cancelled_event(snapshot)?)?;
        }
        self.connection
            .execute(
                "UPDATE engine_instances SET status = 'stopped', updated_at = ?1 WHERE status = 'running'",
                [chrono::Utc::now().to_rfc3339()],
            )
            .map_err(|error| error.to_string())?;
        Ok(podcast_task_ids)
    }

    pub fn active_podcast_task_ids(&self) -> Result<Vec<String>, String> {
        Ok(self
            .task_snapshots(Some(TaskKind::Podcast))?
            .into_iter()
            .filter(|snapshot| is_active_task(&snapshot.lifecycle_state))
            .map(|snapshot| snapshot.id)
            .collect())
    }

    pub fn validate_task_control(
        &self,
        task_id: &str,
        kind: TaskKind,
        expected_revision: u64,
    ) -> Result<TaskSnapshot, String> {
        let snapshot = self
            .task_snapshot(task_id)?
            .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
        if snapshot.kind != kind {
            return Err("TASK_KIND_CONFLICT".to_string());
        }
        if snapshot.revision != expected_revision {
            return Err("REVISION_CONFLICT".to_string());
        }
        Ok(snapshot)
    }

    pub fn mark_task_starting(&mut self, task_id: &str) -> Result<Option<TaskEvent>, String> {
        let Some(mut snapshot) = self.task_snapshot(task_id)? else {
            return Err("TASK_NOT_FOUND".to_string());
        };
        if !matches!(snapshot.kind, TaskKind::Podcast | TaskKind::Zhihu)
            || snapshot.lifecycle_state != LifecycleState::Queued
        {
            return Err("TASK_NOT_QUEUED".to_string());
        }
        let now = chrono::Utc::now().to_rfc3339();
        snapshot.last_sequence = snapshot
            .last_sequence
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
        snapshot.lifecycle_state = LifecycleState::Starting;
        snapshot.engine_stage = "launching".to_string();
        snapshot.engine_status = "starting".to_string();
        // Reset after input_copy's 0–100% so pipeline stages don't inherit a fake 100%.
        snapshot.progress.mode = crate::tasks::ProgressMode::Determinate;
        snapshot.progress.percent = Some(map_pipeline_percent("launching", Some(0.0)));
        snapshot.progress.label = Some(stable_worker_label("launching"));
        snapshot.updated_at = now.clone();
        let event = TaskEvent {
            schema_version: 1,
            task_id: snapshot.id.clone(),
            sequence: snapshot.last_sequence,
            revision: snapshot.revision,
            event_type: "worker_starting".to_string(),
            snapshot,
            created_at: now,
        };
        self.persist_task_event(&event)?;
        Ok(Some(event))
    }

    pub fn rollback_starting_task(
        &mut self,
        task_id: &str,
    ) -> Result<Option<TaskEvent>, String> {
        let Some(mut snapshot) = self.task_snapshot(task_id)? else {
            return Err("TASK_NOT_FOUND".to_string());
        };
        if snapshot.lifecycle_state != LifecycleState::Starting {
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        snapshot.last_sequence = snapshot
            .last_sequence
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
        snapshot.lifecycle_state = LifecycleState::Queued;
        snapshot.outcome = TaskOutcome::None;
        snapshot.error_code = None;
        snapshot.error_message = None;
        snapshot.engine_stage = "queued".to_string();
        snapshot.engine_status = "waiting".to_string();
        snapshot.progress.mode = crate::tasks::ProgressMode::Indeterminate;
        snapshot.progress.percent = None;
        snapshot.can_pause = false;
        snapshot.can_resume = false;
        snapshot.can_retry = false;
        snapshot.can_cancel = true;
        snapshot.updated_at = now.clone();
        let event = TaskEvent {
            schema_version: 1,
            task_id: snapshot.id.clone(),
            sequence: snapshot.last_sequence,
            revision: snapshot.revision,
            event_type: "worker_start_rejected".to_string(),
            snapshot,
            created_at: now,
        };
        self.persist_task_event(&event)?;
        Ok(Some(event))
    }

    pub fn control_task(
        &mut self,
        task_id: &str,
        action: &str,
        expected_revision: u64,
    ) -> Result<TaskEvent, String> {
        let snapshot = self
            .task_snapshot(task_id)?
            .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
        if snapshot.revision != expected_revision {
            return Err("REVISION_CONFLICT".to_string());
        }
        let event = controlled_event(snapshot, action)?;
        self.persist_task_event(&event)?;
        Ok(event)
    }

    pub fn record_worker_line(
        &mut self,
        task_id: &str,
        stream: &str,
        line: &str,
    ) -> Result<Option<TaskEvent>, String> {
        if !matches!(stream, "stdout" | "stderr") {
            return Err("INVALID_WORKER_STREAM".to_string());
        }
        let Some(mut snapshot) = self.task_snapshot(task_id)? else {
            return Err("TASK_NOT_FOUND".to_string());
        };
        if matches!(snapshot.lifecycle_state, LifecycleState::Terminal) {
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let json = worker_json(line);
        let event_kind = json
            .as_ref()
            .and_then(|value| value.get("type").and_then(|v| v.as_str()))
            .unwrap_or(if stream == "stderr" { "stderr" } else { "stdout" });
        let is_fatal = event_kind == "fatal" || event_kind == "completed";
        // Generic "working" must not clobber a more specific stage (and must not
        // count as a stage change that defeats event throttling).
        let detected_stage = worker_stage(line);
        let next_stage = if detected_stage == "working" && !snapshot.engine_stage.is_empty() {
            snapshot.engine_stage.clone()
        } else {
            detected_stage
        };
        let next_percent = worker_percent(line);
        let stage_changed = snapshot.engine_stage != next_stage;
        let prev_percent = snapshot.progress.percent.unwrap_or(-1.0);
        let mapped_preview = map_pipeline_percent(&next_stage, next_percent);
        // Only advance on whole-percentage progress (or first determinate sample).
        let percent_advanced = next_percent.is_some_and(|_| {
            let next_whole = mapped_preview.floor() as i32;
            let prev_whole = prev_percent.floor() as i32;
            next_whole > prev_whole || prev_percent < 0.0
        });
        // Cap progress-only UI/DB events at 4/s; stage changes and fatals always pass.
        let rate_ok = snapshot
            .last_heartbeat_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| {
                let elapsed = chrono::Utc::now().signed_duration_since(value.with_timezone(&chrono::Utc));
                elapsed.num_milliseconds() >= 250
            })
            .unwrap_or(true);
        let heartbeat_stale = snapshot
            .last_heartbeat_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| {
                let elapsed = chrono::Utc::now().signed_duration_since(value.with_timezone(&chrono::Utc));
                elapsed.num_milliseconds() >= 4000
            })
            .unwrap_or(true);
        if !(is_fatal || stage_changed || heartbeat_stale || (percent_advanced && rate_ok)) {
            return Ok(None);
        }

        snapshot.last_sequence = snapshot
            .last_sequence
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
        snapshot.lifecycle_state = LifecycleState::Running;
        snapshot.outcome = TaskOutcome::None;
        snapshot.engine_stage = next_stage.clone();
        snapshot.engine_status = "working".to_string();
        snapshot.can_pause = true;
        snapshot.can_cancel = true;
        snapshot.can_resume = false;
        snapshot.last_heartbeat_at = Some(now.clone());
        // Never surface raw worker log spam as the primary label — UI maps stage to Chinese.
        snapshot.progress.label = Some(stable_worker_label(&next_stage));
        if stream == "stderr" && is_fatal {
            snapshot.error_message = Some(line.trim().chars().take(500).collect());
        }
        if let Some(json) = json.as_ref() {
            if let Some(completed) = json.get("completedUnits").and_then(|v| v.as_u64()) {
                snapshot.progress.completed_units = Some(completed);
            }
            if let Some(total) = json.get("totalUnits").and_then(|v| v.as_u64()) {
                snapshot.progress.total_units = Some(total);
            }
            if let Some(unit) = json.get("unit").and_then(|v| v.as_str()) {
                snapshot.progress.unit = Some(unit.to_string());
            }
            if event_kind == "checkpoint" {
                snapshot.checkpoint_at = Some(now.clone());
            }
            if event_kind == "fatal" {
                if let Some(message) = json.get("message").and_then(|v| v.as_str()) {
                    snapshot.error_message = Some(message.chars().take(500).collect());
                }
            }
        }
        // Map raw worker % into stage bands. Unmeasurable stages stay indeterminate
        // (no invented mid-band numbers); overall percent never decreases.
        if let Some(raw) = next_percent {
            let mapped = map_pipeline_percent(&next_stage, Some(raw));
            snapshot.progress.mode = crate::tasks::ProgressMode::Determinate;
            let floor = snapshot.progress.percent.unwrap_or(0.0);
            let stage_floor = map_pipeline_percent(&next_stage, Some(0.0));
            snapshot.progress.percent =
                Some(mapped.max(floor).max(stage_floor).clamp(0.0, 100.0));
        } else if stage_changed {
            // Real work not reported for this stage — do not forge a percent.
            snapshot.progress.mode = crate::tasks::ProgressMode::Indeterminate;
        } else if snapshot.progress.percent.is_none() {
            snapshot.progress.mode = crate::tasks::ProgressMode::Indeterminate;
        }
        snapshot.updated_at = now.clone();
        let event = TaskEvent {
            schema_version: 1,
            task_id: snapshot.id.clone(),
            sequence: snapshot.last_sequence,
            revision: snapshot.revision,
            event_type: format!("worker_{event_kind}"),
            snapshot,
            created_at: now,
        };
        self.persist_task_event(&event)?;
        Ok(Some(event))
    }

    pub fn record_external_snapshot(
        &mut self,
        mut next: TaskSnapshot,
        event_type: &str,
    ) -> Result<Option<TaskEvent>, String> {
        let current = self
            .task_snapshot(&next.id)?
            .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
        if current.kind != next.kind {
            return Err("TASK_KIND_CONFLICT".to_string());
        }
        // Sidecar truth may supersede a false local crash/interrupt terminal for Zhihu.
        let allow_terminal_override = current.lifecycle_state == LifecycleState::Terminal
            && matches!(current.kind, TaskKind::Zhihu)
            && matches!(current.outcome, TaskOutcome::Interrupted)
            && (matches!(
                next.lifecycle_state,
                LifecycleState::Running
                    | LifecycleState::Paused
                    | LifecycleState::Starting
                    | LifecycleState::Queued
            ) || matches!(
                next.outcome,
                TaskOutcome::Success | TaskOutcome::PartialSuccess | TaskOutcome::Failed
            ));
        if current.lifecycle_state == LifecycleState::Terminal && !allow_terminal_override {
            return Ok(None);
        }
        let same_payload = current.lifecycle_state == next.lifecycle_state
            && current.outcome == next.outcome
            && current.required_action == next.required_action
            && current.progress == next.progress
            && current.error_code == next.error_code
            && current.error_message == next.error_message
            && current.engine_stage == next.engine_stage
            && current.engine_status == next.engine_status
            && current.can_pause == next.can_pause
            && current.can_resume == next.can_resume
            && current.can_retry == next.can_retry
            && current.can_cancel == next.can_cancel;
        // Ignore pure heartbeat noise unless the caller explicitly records a heartbeat.
        if same_payload && event_type != "engine_heartbeat" {
            return Ok(None);
        }
        if same_payload
            && event_type == "engine_heartbeat"
            && current.last_heartbeat_at == next.last_heartbeat_at
        {
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        if next.last_heartbeat_at.is_none() {
            next.last_heartbeat_at = Some(now.clone());
        }
        next.last_sequence = current
            .last_sequence
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        next.revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
        next.created_at = current.created_at;
        next.updated_at = now.clone();
        let event = TaskEvent {
            schema_version: 1,
            task_id: next.id.clone(),
            sequence: next.last_sequence,
            revision: next.revision,
            event_type: event_type.to_string(),
            snapshot: next,
            created_at: now,
        };
        self.persist_task_event(&event)?;
        Ok(Some(event))
    }

    /// Recover a previously failed/interrupted terminal task to success (e.g. re-publish).
    pub fn mark_terminal_task_success(
        &mut self,
        task_id: &str,
        label: &str,
        book_id: Option<String>,
    ) -> Result<Option<TaskEvent>, String> {
        let Some(mut snapshot) = self.task_snapshot(task_id)? else {
            return Err("TASK_NOT_FOUND".to_string());
        };
        if matches!(snapshot.outcome, TaskOutcome::Success)
            && matches!(snapshot.lifecycle_state, LifecycleState::Terminal)
        {
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        snapshot.last_sequence = snapshot
            .last_sequence
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
        snapshot.lifecycle_state = LifecycleState::Terminal;
        snapshot.outcome = TaskOutcome::Success;
        snapshot.error_code = None;
        snapshot.error_message = None;
        snapshot.retry_after_seconds = None;
        snapshot.required_action = RequiredAction::None;
        snapshot.engine_stage = "completed".to_string();
        snapshot.engine_status = "exited".to_string();
        snapshot.recoverable = false;
        snapshot.can_pause = false;
        snapshot.can_resume = false;
        snapshot.can_retry = false;
        snapshot.can_cancel = false;
        if let Some(book_id) = book_id.filter(|value| !value.trim().is_empty()) {
            snapshot.book_id = Some(book_id);
        }
        snapshot.progress.mode = crate::tasks::ProgressMode::Determinate;
        snapshot.progress.percent = Some(100.0);
        snapshot.progress.label = Some(label.to_string());
        snapshot.updated_at = now.clone();
        snapshot.last_heartbeat_at = Some(now.clone());
        let event = TaskEvent {
            schema_version: 1,
            task_id: snapshot.id.clone(),
            sequence: snapshot.last_sequence,
            revision: snapshot.revision,
            event_type: "recovered_success".to_string(),
            snapshot,
            created_at: now,
        };
        self.persist_task_event(&event)?;
        Ok(Some(event))
    }

    pub fn finish_worker_task(
        &mut self,
        task_id: &str,
        success: bool,
        message: Option<&str>,
    ) -> Result<Option<TaskEvent>, String> {
        let Some(mut snapshot) = self.task_snapshot(task_id)? else {
            return Err("TASK_NOT_FOUND".to_string());
        };
        if matches!(snapshot.lifecycle_state, LifecycleState::Terminal) {
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        snapshot.last_sequence = snapshot
            .last_sequence
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
        snapshot.lifecycle_state = LifecycleState::Terminal;
        snapshot.outcome = if success {
            TaskOutcome::Success
        } else {
            TaskOutcome::Failed
        };
        snapshot.error_code = if success {
            None
        } else {
            worker_error_code(message).or(Some(TaskErrorCode::Unknown))
        };
        snapshot.error_message = message.map(|value| value.trim().chars().take(500).collect());
        snapshot.retry_after_seconds = if success {
            None
        } else {
            worker_retry_after_seconds(message)
        };
        snapshot.required_action =
            if snapshot.error_code == Some(TaskErrorCode::BudgetConfirmationRequired) {
                RequiredAction::ApproveBudget
            } else {
                RequiredAction::None
            };
        snapshot.engine_stage = if success {
            "completed".to_string()
        } else {
            "failed".to_string()
        };
        snapshot.engine_status = "exited".to_string();
        snapshot.recoverable = !success;
        snapshot.can_pause = false;
        snapshot.can_resume = false;
        snapshot.can_retry = !success && snapshot.required_action != RequiredAction::ApproveBudget;
        snapshot.can_cancel = false;
        if success {
            snapshot.progress.mode = crate::tasks::ProgressMode::Determinate;
            snapshot.progress.percent = Some(100.0);
        }
        snapshot.updated_at = now.clone();
        let event = TaskEvent {
            schema_version: 1,
            task_id: snapshot.id.clone(),
            sequence: snapshot.last_sequence,
            revision: snapshot.revision,
            event_type: if success {
                "worker_completed".to_string()
            } else {
                "worker_failed".to_string()
            },
            snapshot,
            created_at: now,
        };
        self.persist_task_event(&event)?;
        Ok(Some(event))
    }

    pub fn task_events(
        &self,
        task_id: &str,
        after_sequence: u64,
        limit: u32,
    ) -> Result<Vec<TaskEvent>, String> {
        let after_sequence =
            i64::try_from(after_sequence).map_err(|_| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT event_json FROM task_events WHERE task_id = ?1 AND sequence > ?2 ORDER BY sequence LIMIT ?3",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(
                params![task_id, after_sequence, limit.clamp(1, 1_000)],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let json = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&json).map_err(|error| error.to_string())
        })
        .collect()
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

fn task_kind_for_engine(engine: &str) -> Result<TaskKind, String> {
    match engine {
        "podcast" => Ok(TaskKind::Podcast),
        "zhihu" => Ok(TaskKind::Zhihu),
        _ => Err("UNKNOWN_ENGINE".to_string()),
    }
}

fn worker_json(line: &str) -> Option<serde_json::Value> {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn worker_percent(line: &str) -> Option<f64> {
    if let Some(value) = worker_json(line) {
        if let Some(percent) = value.get("percent").and_then(|v| v.as_f64()) {
            if percent.is_finite() {
                return Some(percent.clamp(0.0, 100.0));
            }
        }
    }
    // Prefer structured JSON; plain log `%` tokens (tqdm bars, etc.) are unreliable.
    None
}

/// Stage bands for the full podcast pipeline (translate/polish optional).
/// When a stage is skipped the worker jumps past it; the next stage floor still
/// keeps overall progress monotonic via `mapped.max(floor)` in apply_worker_line.
fn map_pipeline_percent(stage: &str, raw: Option<f64>) -> f64 {
    let (lo, hi) = match stage {
        "input_copy" | "prepare" => (0.0, 6.0),
        "launching" | "starting" => (6.0, 8.0),
        "load_model" => (8.0, 12.0),
        "normalizing" => (12.0, 18.0),
        "chunking" => (18.0, 24.0),
        // Transcribe is the real heavy stage (audio duration / chunks).
        "transcribing" | "transcribe" => (24.0, 70.0),
        // Translation weight only applies when the worker enters this stage.
        "translating" | "translate" => (70.0, 86.0),
        // Chinese tasks skip translate and land here after transcribe (~70+).
        "polishing" => (70.0, 92.0),
        "postprocess" => (86.0, 94.0),
        "writing_output" => (94.0, 98.0),
        "publish" => (98.0, 99.5),
        "completed" => (100.0, 100.0),
        _ => (12.0, 90.0),
    };
    match raw {
        Some(value) => {
            let t = value.clamp(0.0, 100.0) / 100.0;
            lo + t * (hi - lo)
        }
        // Unmeasurable stages: stay indeterminate at the band floor without inventing mid-band numbers.
        None => lo,
    }
}

fn worker_stage(line: &str) -> String {
    if let Some(value) = worker_json(line) {
        if let Some(stage) = value
            .get("stage")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return normalize_stage_name(stage);
        }
    }
    let lower = line.to_ascii_lowercase();
    // Budget / retry messages still mean "translating", not "failed".
    if lower.contains("budget") || lower.contains("retrying") || lower.contains("translat") {
        return "translating".to_string();
    }
    for (needle, stage) in [
        ("normaliz", "normalizing"),
        ("chunk", "chunking"),
        ("polish", "polishing"),
        ("writ", "writing_output"),
        ("postprocess", "postprocess"),
        ("transcrib", "transcribing"),
        ("audio done", "postprocess"),
        ("copy", "input_copy"),
        ("publish", "publish"),
        ("model", "load_model"),
    ] {
        if lower.contains(needle) {
            return stage.to_string();
        }
    }
    "working".to_string()
}

fn normalize_stage_name(stage: &str) -> String {
    match stage.to_ascii_lowercase().as_str() {
        "translate" | "translation" => "translating".to_string(),
        "transcribe" | "transcription" => "transcribing".to_string(),
        "write_output" | "writing" | "output" => "writing_output".to_string(),
        "load_model" | "model" => "load_model".to_string(),
        "input_copy" | "copy" | "prepare" => "input_copy".to_string(),
        other => other.to_string(),
    }
}

fn stable_worker_label(stage: &str) -> String {
    match stage {
        "input_copy" => "正在准备音频".to_string(),
        "load_model" => "正在加载模型".to_string(),
        "normalizing" => "正在标准化音频".to_string(),
        "chunking" => "正在切分音频".to_string(),
        "transcribing" => "正在语音转写".to_string(),
        "translating" => "正在翻译".to_string(),
        "polishing" => "正在润色文稿".to_string(),
        "postprocess" => "正在后处理".to_string(),
        "writing_output" => "正在生成文稿".to_string(),
        "publish" => "正在发布到书库".to_string(),
        "completed" => "即将完成".to_string(),
        _ => "处理中".to_string(),
    }
}

fn worker_error_code(message: Option<&str>) -> Option<TaskErrorCode> {
    let raw = message?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    match value.get("errorCode")?.as_str()? {
        "INPUT_CHANGED" => Some(TaskErrorCode::InputChanged),
        "PIPELINE_INCOMPATIBLE" => Some(TaskErrorCode::PipelineIncompatible),
        "MODEL_INCOMPATIBLE" => Some(TaskErrorCode::ModelIncompatible),
        "CONFIG_INCOMPATIBLE" => Some(TaskErrorCode::ConfigIncompatible),
        "ENGINE_UNAVAILABLE" => Some(TaskErrorCode::EngineUnavailable),
        "BUDGET_CONFIRMATION_REQUIRED" => Some(TaskErrorCode::BudgetConfirmationRequired),
        "PUBLISH_FAILED" => Some(TaskErrorCode::PublishFailed),
        "PUBLISH_RECOVERY_REQUIRED" => Some(TaskErrorCode::PublishRecoveryRequired),
        "UPSTREAM_UNAUTHORIZED" => Some(TaskErrorCode::UpstreamUnauthorized),
        "RATE_LIMITED" => Some(TaskErrorCode::RateLimited),
        "UPSTREAM_TIMEOUT" => Some(TaskErrorCode::UpstreamTimeout),
        "UPSTREAM_UNAVAILABLE" => Some(TaskErrorCode::UpstreamUnavailable),
        _ => None,
    }
}

fn worker_retry_after_seconds(message: Option<&str>) -> Option<u64> {
    let raw = message?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value
        .get("retryAfterSeconds")
        .and_then(serde_json::Value::as_u64)
}

fn is_active_task(state: &LifecycleState) -> bool {
    matches!(
        state,
        LifecycleState::Starting
            | LifecycleState::Running
            | LifecycleState::Pausing
            | LifecycleState::Paused
            | LifecycleState::Stopping
    )
}

fn interrupted_event(
    mut snapshot: TaskSnapshot,
    exit_code: Option<i32>,
) -> Result<TaskEvent, String> {
    let now = chrono::Utc::now().to_rfc3339();
    snapshot.last_sequence = snapshot
        .last_sequence
        .checked_add(1)
        .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
    snapshot.revision = snapshot
        .revision
        .checked_add(1)
        .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
    snapshot.lifecycle_state = LifecycleState::Terminal;
    snapshot.outcome = TaskOutcome::Interrupted;
    snapshot.error_code = Some(TaskErrorCode::EngineCrashed);
    snapshot.error_message = Some(match exit_code {
        Some(code) => format!("受管引擎异常退出（exit code {code}）。"),
        None => "应用重启后发现受管引擎未正常结束。".to_string(),
    });
    snapshot.engine_stage = "crashed".to_string();
    snapshot.engine_status = "exited".to_string();
    snapshot.recoverable = true;
    snapshot.can_pause = false;
    snapshot.can_resume = false;
    snapshot.can_retry = true;
    snapshot.can_cancel = false;
    snapshot.updated_at = now.clone();
    Ok(TaskEvent {
        schema_version: 1,
        task_id: snapshot.id.clone(),
        sequence: snapshot.last_sequence,
        revision: snapshot.revision,
        event_type: "engine_crashed".to_string(),
        snapshot,
        created_at: now,
    })
}

fn cancelled_event(mut snapshot: TaskSnapshot) -> Result<TaskEvent, String> {
    let now = chrono::Utc::now().to_rfc3339();
    snapshot.last_sequence = snapshot
        .last_sequence
        .checked_add(1)
        .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
    snapshot.revision = snapshot
        .revision
        .checked_add(1)
        .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
    snapshot.lifecycle_state = LifecycleState::Terminal;
    snapshot.outcome = TaskOutcome::Cancelled;
    snapshot.error_code = Some(TaskErrorCode::CancelledByUser);
    snapshot.error_message = Some("用户明确选择退出并清理，任务及其缓存已丢弃。".to_string());
    snapshot.engine_stage = "cancelled".to_string();
    snapshot.engine_status = "stopped".to_string();
    snapshot.recoverable = false;
    snapshot.can_pause = false;
    snapshot.can_resume = false;
    snapshot.can_retry = false;
    snapshot.can_cancel = false;
    snapshot.updated_at = now.clone();
    Ok(TaskEvent {
        schema_version: 1,
        task_id: snapshot.id.clone(),
        sequence: snapshot.last_sequence,
        revision: snapshot.revision,
        event_type: "cancelled_and_discarded".to_string(),
        snapshot,
        created_at: now,
    })
}

fn controlled_event(mut snapshot: TaskSnapshot, action: &str) -> Result<TaskEvent, String> {
    let now = chrono::Utc::now().to_rfc3339();
    snapshot.last_sequence = snapshot
        .last_sequence
        .checked_add(1)
        .ok_or_else(|| "INVALID_TASK_EVENT_SEQUENCE".to_string())?;
    snapshot.revision = snapshot
        .revision
        .checked_add(1)
        .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
    let event_type = match action {
        "pause" if snapshot.lifecycle_state == LifecycleState::Running => {
            snapshot.lifecycle_state = LifecycleState::Paused;
            snapshot.engine_stage = "paused".to_string();
            snapshot.engine_status = "paused".to_string();
            snapshot.can_pause = false;
            snapshot.can_resume = true;
            "paused"
        }
        "resume" if snapshot.lifecycle_state == LifecycleState::Paused => {
            snapshot.lifecycle_state = LifecycleState::Running;
            snapshot.engine_stage = "resuming".to_string();
            snapshot.engine_status = "working".to_string();
            snapshot.can_pause = true;
            snapshot.can_resume = false;
            "resumed"
        }
        "cancel" if is_active_task(&snapshot.lifecycle_state) => {
            snapshot.lifecycle_state = LifecycleState::Terminal;
            snapshot.outcome = TaskOutcome::Cancelled;
            snapshot.error_code = Some(TaskErrorCode::CancelledByUser);
            snapshot.error_message = Some("用户取消了任务，缓存 lease 保留以便恢复。".to_string());
            snapshot.engine_stage = "cancelled".to_string();
            snapshot.engine_status = "stopped".to_string();
            snapshot.recoverable = true;
            snapshot.can_pause = false;
            snapshot.can_resume = false;
            snapshot.can_retry = true;
            snapshot.can_cancel = false;
            "cancelled"
        }
        "cancel_and_discard" if is_active_task(&snapshot.lifecycle_state) => {
            snapshot.lifecycle_state = LifecycleState::Terminal;
            snapshot.outcome = TaskOutcome::Cancelled;
            snapshot.error_code = Some(TaskErrorCode::CancelledByUser);
            snapshot.error_message = Some("用户取消并丢弃了任务及缓存。".to_string());
            snapshot.engine_stage = "cancelled".to_string();
            snapshot.engine_status = "stopped".to_string();
            snapshot.recoverable = false;
            snapshot.can_pause = false;
            snapshot.can_resume = false;
            snapshot.can_retry = false;
            snapshot.can_cancel = false;
            "cancelled_and_discarded"
        }
        _ => return Err("INVALID_TASK_CONTROL".to_string()),
    };
    snapshot.updated_at = now.clone();
    Ok(TaskEvent {
        schema_version: 1,
        task_id: snapshot.id.clone(),
        sequence: snapshot.last_sequence,
        revision: snapshot.revision,
        event_type: event_type.to_string(),
        snapshot,
        created_at: now,
    })
}

#[cfg(test)]
mod tests;
