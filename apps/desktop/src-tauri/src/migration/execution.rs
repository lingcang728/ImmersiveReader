use super::{preview_for, LegacyLocations, MigrationReceipt, MigrationScope};
use crate::control::{CommandClaim, ControlDb};
use crate::storage::StorageLocations;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const COMMAND_NAME: &str = "execute_legacy_migration";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationExecutionResult {
    pub migration_id: String,
    pub preview_id: String,
    pub scope: MigrationScope,
    pub status: String,
    pub receipt_path: String,
    pub completed_kinds: Vec<String>,
}

fn command_input_hash(preview_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"settings\0");
    hash.update(preview_id.as_bytes());
    format!("{:x}", hash.finalize())
}

fn sha256(path: &Path) -> Result<String, String> {
    crate::publish::hash_file(path)
}

fn source_schema(path: &Path) -> Result<u32, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "Settings schema version is missing".to_string())
}

fn write_receipt(path: &Path, receipt: &MigrationReceipt) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    crate::atomic_file::write(path, &bytes)
}

fn restore_settings(source: &Path, target: &Path, original: &[u8]) {
    if source == target {
        let _ = crate::atomic_file::write(target, original);
    } else {
        let _ = fs::remove_file(target);
    }
}

fn replay(record: crate::control::CommandRecord) -> Result<MigrationExecutionResult, String> {
    if let Some(error) = record.error_code {
        return Err(error);
    }
    let json = record
        .result_json
        .ok_or_else(|| "COMMAND_IN_PROGRESS".to_string())?;
    serde_json::from_str(&json).map_err(|error| error.to_string())
}

fn failed_receipt(
    migration_id: &str,
    source: &Path,
    target: &Path,
    rollback: &Path,
    source_version: u32,
    error_code: &str,
) -> MigrationReceipt {
    MigrationReceipt {
        schema_version: 1,
        migration_id: migration_id.to_string(),
        source_paths: vec![source.to_string_lossy().into_owned()],
        target_paths: vec![target.to_string_lossy().into_owned()],
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
        source_schema_version: source_version,
        target_schema_version: 3,
        table_counts_before: BTreeMap::new(),
        table_counts_after: BTreeMap::new(),
        non_sensitive_hashes: BTreeMap::new(),
        executor_version: env!("CARGO_PKG_VERSION").to_string(),
        status: "failed".to_string(),
        rollback_location: rollback.to_string_lossy().into_owned(),
        error_code: Some(error_code.to_string()),
    }
}

/// P2-14 conflict gate: a target that already holds exactly what this
/// migration would write is an idempotent replay, not a conflict. Without it
/// the migration could never run on a previously-used install — the app
/// creates settings.json on first launch, so the `target.exists()` conflict
/// flag would fire forever.
fn settings_already_migrated(legacy: &LegacyLocations, target: &StorageLocations) -> bool {
    if legacy.settings == target.settings_path {
        return false;
    }
    let Ok(expected) = crate::settings::load_compatible_from(&legacy.settings) else {
        return false;
    };
    let Ok(actual) = crate::settings::load_compatible_from(&target.settings_path) else {
        return false;
    };
    expected == actual
}

/// The claimed body of `execute_settings_migration`. Returns either the
/// success result or `(error_code, detail)`; the caller settles the command
/// claim exactly once for every outcome — a `?` after `claim_command` used to
/// leak the claim and leave the request_id in-progress forever.
fn execute_claimed_settings_migration(
    control: &ControlDb,
    legacy: &LegacyLocations,
    target: &StorageLocations,
    preview_id: &str,
) -> Result<MigrationExecutionResult, (String, String)> {
    let fail = |code: &str, detail: String| (code.to_string(), detail);

    let fresh = preview_for(legacy, target, MigrationScope::Settings)
        .map_err(|error| fail("MIGRATION_FAILED", error))?;
    if fresh.preview_id != preview_id {
        return Err(fail(
            "MIGRATION_PREVIEW_STALE",
            "Migration preview is stale".to_string(),
        ));
    }
    if fresh.conflict_count > 0 && !settings_already_migrated(legacy, target) {
        return Err(fail(
            "MIGRATION_CONFLICT",
            "Migration target already exists".to_string(),
        ));
    }
    if !legacy.settings.is_file() {
        return Err(fail(
            "MIGRATION_SOURCE_MISSING",
            "Legacy settings source is missing".to_string(),
        ));
    }

    let migration_id = format!("settings-{}", uuid::Uuid::new_v4());
    let migration_root = target.data_root.join("Migrations").join(&migration_id);
    let rollback = migration_root.join(r"rollback\settings.json");
    let receipt_path = migration_root.join("receipt.json");
    fs::create_dir_all(rollback.parent().unwrap())
        .map_err(|error| fail("MIGRATION_FAILED", error.to_string()))?;
    let original =
        fs::read(&legacy.settings).map_err(|error| fail("MIGRATION_FAILED", error.to_string()))?;
    fs::write(&rollback, &original)
        .map_err(|error| fail("MIGRATION_FAILED", error.to_string()))?;
    let source_version =
        source_schema(&legacy.settings).map_err(|error| fail("MIGRATION_FAILED", error))?;
    let mut receipt = failed_receipt(
        &migration_id,
        &legacy.settings,
        &target.settings_path,
        rollback.parent().unwrap(),
        source_version,
        "MIGRATION_FAILED",
    );
    receipt.started_at = chrono::Utc::now().to_rfc3339();
    receipt.non_sensitive_hashes.insert(
        "sourceSettingsSha256".to_string(),
        sha256(&legacy.settings).map_err(|error| fail("MIGRATION_FAILED", error))?,
    );
    write_receipt(&receipt_path, &receipt).map_err(|error| fail("MIGRATION_FAILED", error))?;
    control
        .begin_migration_run(&migration_id, preview_id, "settings")
        .map_err(|error| fail("MIGRATION_FAILED", error))?;

    let migrated = (|| {
        let settings = crate::settings::load_compatible_from(&legacy.settings)?;
        crate::settings::save_compatible_to(&target.settings_path, &settings)?;
        let verified = crate::settings::load_compatible_from(&target.settings_path)?;
        if verified.schema_version != 3 || verified.library_root != settings.library_root {
            return Err("Settings verification failed".to_string());
        }
        receipt.status = "success".to_string();
        receipt.error_code = None;
        receipt.completed_at = Some(chrono::Utc::now().to_rfc3339());
        receipt.non_sensitive_hashes.insert(
            "targetSettingsSha256".to_string(),
            sha256(&target.settings_path)?,
        );
        write_receipt(&receipt_path, &receipt)
    })();

    if let Err(error) = migrated {
        restore_settings(&legacy.settings, &target.settings_path, &original);
        receipt.completed_at = Some(chrono::Utc::now().to_rfc3339());
        let _ = write_receipt(&receipt_path, &receipt);
        let failure = serde_json::json!({ "error": error }).to_string();
        // A stuck 'running' migration_runs row is cosmetic; the claim tail
        // still settles the command even if this bookkeeping write fails.
        let _ = control.complete_migration_run(
            &migration_id,
            "failed",
            Some(&receipt_path.to_string_lossy()),
            &failure,
        );
        return Err(fail("MIGRATION_FAILED", error));
    }

    let result = MigrationExecutionResult {
        migration_id: migration_id.clone(),
        preview_id: preview_id.to_string(),
        scope: MigrationScope::Settings,
        status: "success".to_string(),
        receipt_path: receipt_path.to_string_lossy().into_owned(),
        completed_kinds: vec!["app_settings".to_string()],
    };
    let result_json =
        serde_json::to_string(&result).map_err(|error| fail("MIGRATION_FAILED", error.to_string()))?;
    control
        .complete_migration_run(
            &migration_id,
            "success",
            Some(&result.receipt_path),
            &result_json,
        )
        .map_err(|error| fail("MIGRATION_FAILED", error))?;
    Ok(result)
}

pub fn execute_settings_migration(
    legacy: &LegacyLocations,
    target: &StorageLocations,
    preview_id: &str,
    request_id: &str,
) -> Result<MigrationExecutionResult, String> {
    let control = ControlDb::open(&target.data_root.join(r"App\control.db"))?;
    match control.claim_command(request_id, COMMAND_NAME, &command_input_hash(preview_id))? {
        CommandClaim::Existing(record) => return replay(*record),
        CommandClaim::New => {}
    }

    // P2-14: once claimed, every exit path must settle the claim via
    // complete_command — otherwise the request_id stays in-progress forever
    // and every retry replays COMMAND_IN_PROGRESS.
    match execute_claimed_settings_migration(&control, legacy, target, preview_id) {
        Ok(result) => {
            let result_json =
                serde_json::to_string(&result).map_err(|error| error.to_string())?;
            control.complete_command(request_id, &result_json, None, None)?;
            Ok(result)
        }
        Err((code, detail)) => {
            let failure = serde_json::json!({ "error": detail }).to_string();
            control.complete_command(request_id, &failure, Some(&code), None)?;
            Err(code)
        }
    }
}
