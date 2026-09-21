use super::{permanently_delete, restore, TrashDeleteResult, TrashRestoreResult};
use crate::control::{CommandClaim, CommandRecord, ControlDb};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

fn input_hash(trash_id: &str, expected_revision: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(trash_id.as_bytes());
    digest.update([0]);
    digest.update(expected_revision.to_le_bytes());
    format!("{:x}", digest.finalize())
}

fn replay<T: DeserializeOwned>(record: CommandRecord) -> Result<T, String> {
    if let Some(error_code) = record.error_code {
        return Err(error_code);
    }
    let json = record
        .result_json
        .ok_or_else(|| "COMMAND_IN_PROGRESS".to_string())?;
    serde_json::from_str(&json).map_err(|error| error.to_string())
}

fn execute<T: DeserializeOwned + Serialize>(
    control: &ControlDb,
    request_id: &str,
    command_name: &str,
    hash: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if request_id.trim().is_empty() {
        return Err("INVALID_ARGUMENT".to_string());
    }
    match control.claim_command(request_id, command_name, hash)? {
        CommandClaim::Existing(record) => return replay(*record),
        CommandClaim::New => {}
    }
    match operation() {
        Ok(result) => {
            let json = serde_json::to_string(&result).map_err(|error| error.to_string())?;
            control.complete_command(request_id, &json, None, None)?;
            Ok(result)
        }
        Err(error_code) => {
            // P-11-F15: only deterministic request-shape errors are cached as
            // the claim's terminal result — the same request would fail the
            // same way, so replay is honest. Transient/environment failures
            // (IO, locking, half-finished journals) release the claim so an
            // immediate retry re-executes; restore/permanent_delete are
            // idempotent under reconcile() so re-execution is safe.
            if is_deterministic_trash_error(&error_code) {
                control.complete_command(request_id, "{}", Some(&error_code), None)?;
            } else {
                control.release_command(request_id)?;
            }
            Err(error_code)
        }
    }
}

/// Errors that depend only on the request's shape, not on the environment —
/// the only ones safe to cache for idempotent replay. `CONFLICT` is
/// deliberately absent: it reports the *destination* already exists, which is
/// external filesystem state the user can clear by hand — caching it would
/// wedge the restore for the whole retention window (the original P1). Raw
/// IO error strings (file locks, AV scans) are likewise never in this list.
fn is_deterministic_trash_error(error: &str) -> bool {
    matches!(
        error,
        "INVALID_ARGUMENT"
            | "NOT_FOUND"
            | "REVISION_CONFLICT"
            | "PATH_OUTSIDE_MANAGED_ROOT"
            | "INVALID_TRASH_ENTRY"
            | "INVALID_TRASH_JOURNAL"
    )
}

pub fn restore_idempotent(
    root: &Path,
    control: &ControlDb,
    trash_id: &str,
    expected_revision: u64,
    request_id: &str,
) -> Result<TrashRestoreResult, String> {
    execute(
        control,
        request_id,
        "restore_trash_item",
        &input_hash(trash_id, expected_revision),
        || restore(root, trash_id, expected_revision),
    )
}

pub fn delete_idempotent(
    root: &Path,
    control: &ControlDb,
    trash_id: &str,
    expected_revision: u64,
    request_id: &str,
) -> Result<TrashDeleteResult, String> {
    execute(
        control,
        request_id,
        "permanently_delete_trash_item",
        &input_hash(trash_id, expected_revision),
        || permanently_delete(root, trash_id, expected_revision),
    )
}
