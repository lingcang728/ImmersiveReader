use super::validation::{managed_relative, validate_book};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishPhase {
    Prepared,
    OldMoved,
    NewMoved,
    Committed,
    RolledBack,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishTransaction {
    pub schema_version: u32,
    pub transaction_id: String,
    pub task_id: String,
    pub book_id: String,
    pub incoming_relative_path: String,
    pub final_relative_path: String,
    pub rollback_relative_path: String,
    pub manifest_sha256: String,
    pub provenance_sha256: String,
    pub revision: u64,
    pub phase: PublishPhase,
    pub created_at: String,
    pub updated_at: String,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn journal_path(root: &Path, transaction_id: &str) -> Result<PathBuf, String> {
    if !valid_id(transaction_id) {
        return Err("Invalid publish transaction id".to_string());
    }
    Ok(root
        .join(".transactions")
        .join(format!("{transaction_id}.json")))
}

fn save_transaction(root: &Path, transaction: &PublishTransaction) -> Result<(), String> {
    let path = journal_path(root, &transaction.transaction_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(transaction).map_err(|error| error.to_string())?;
    crate::atomic_file::write(&path, &data)
}

pub fn load_transaction(root: &Path, transaction_id: &str) -> Result<PublishTransaction, String> {
    let raw = fs::read(journal_path(root, transaction_id)?).map_err(|error| error.to_string())?;
    let transaction: PublishTransaction =
        serde_json::from_slice(&raw).map_err(|error| error.to_string())?;
    if transaction.schema_version != 1 || transaction.transaction_id != transaction_id {
        return Err("Invalid publish transaction journal".to_string());
    }
    Ok(transaction)
}

pub fn list_transactions(root: &Path) -> Result<Vec<PublishTransaction>, String> {
    let journals = root.join(".transactions");
    if !journals.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(journals)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries
        .into_iter()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "json")
        })
        .map(|entry| {
            let id = entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| "Invalid publish journal file name".to_string())?
                .to_string();
            load_transaction(root, &id)
        })
        .collect()
}

fn ensure_single_book_transaction(
    root: &Path,
    transaction: &PublishTransaction,
) -> Result<(), String> {
    let conflict = list_transactions(root)?.into_iter().any(|existing| {
        existing.transaction_id != transaction.transaction_id
            && existing.book_id == transaction.book_id
            && !matches!(
                existing.phase,
                PublishPhase::Committed | PublishPhase::RolledBack
            )
    });
    if conflict {
        return Err("Another publish transaction is active for this book".to_string());
    }
    Ok(())
}

fn set_phase(
    root: &Path,
    transaction: &mut PublishTransaction,
    phase: PublishPhase,
) -> Result<(), String> {
    transaction.phase = phase;
    transaction.updated_at = chrono::Utc::now().to_rfc3339();
    save_transaction(root, transaction)
}

fn inject_stop(stop_after: Option<PublishPhase>, phase: PublishPhase) -> Result<(), String> {
    if stop_after == Some(phase) {
        return Err(format!("Injected crash after {phase:?}"));
    }
    Ok(())
}

fn rollback(root: &Path, transaction: &mut PublishTransaction) -> Result<(), String> {
    let final_path = managed_relative(root, &transaction.final_relative_path)?;
    let rollback_path = managed_relative(root, &transaction.rollback_relative_path)?;
    if final_path.exists() {
        let failed = root
            .join(".incoming")
            .join(format!("failed-{}", transaction.transaction_id));
        if failed.exists() {
            return Err("Failed publication quarantine already exists".to_string());
        }
        fs::create_dir_all(
            failed
                .parent()
                .ok_or_else(|| "Invalid failed publication path".to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::rename(&final_path, failed).map_err(|error| error.to_string())?;
    }
    if rollback_path.exists() {
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::rename(rollback_path, final_path).map_err(|error| error.to_string())?;
    }
    set_phase(root, transaction, PublishPhase::RolledBack)
}

fn advance(
    root: &Path,
    transaction: &mut PublishTransaction,
    stop_after: Option<PublishPhase>,
) -> Result<(), String> {
    loop {
        match transaction.phase {
            PublishPhase::Prepared => {
                if validate_book(root, &transaction.incoming_relative_path, transaction).is_err() {
                    rollback(root, transaction)?;
                    return Ok(());
                }
                let final_path = managed_relative(root, &transaction.final_relative_path)?;
                let rollback_path = managed_relative(root, &transaction.rollback_relative_path)?;
                if final_path.exists() {
                    if rollback_path.exists() {
                        return Err("Publish rollback path already exists".to_string());
                    }
                    if let Some(parent) = rollback_path.parent() {
                        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                    }
                    fs::rename(&final_path, &rollback_path).map_err(|error| error.to_string())?;
                }
                set_phase(root, transaction, PublishPhase::OldMoved)?;
                inject_stop(stop_after, PublishPhase::OldMoved)?;
            }
            PublishPhase::OldMoved => {
                if validate_book(root, &transaction.incoming_relative_path, transaction).is_err() {
                    rollback(root, transaction)?;
                    return Ok(());
                }
                let incoming = managed_relative(root, &transaction.incoming_relative_path)?;
                let final_path = managed_relative(root, &transaction.final_relative_path)?;
                if let Some(parent) = final_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::rename(incoming, final_path).map_err(|error| error.to_string())?;
                set_phase(root, transaction, PublishPhase::NewMoved)?;
                inject_stop(stop_after, PublishPhase::NewMoved)?;
            }
            PublishPhase::NewMoved => {
                if validate_book(root, &transaction.final_relative_path, transaction).is_ok() {
                    set_phase(root, transaction, PublishPhase::Committed)?;
                } else {
                    rollback(root, transaction)?;
                }
                return Ok(());
            }
            PublishPhase::Committed => {
                validate_book(root, &transaction.final_relative_path, transaction)?;
                return Ok(());
            }
            PublishPhase::RolledBack => return Ok(()),
        }
    }
}

pub fn commit_transaction(
    root: &Path,
    transaction: &PublishTransaction,
) -> Result<PublishTransaction, String> {
    let mut current = if journal_path(root, &transaction.transaction_id)?.exists() {
        load_transaction(root, &transaction.transaction_id)?
    } else {
        if transaction.phase != PublishPhase::Prepared {
            return Err("New publish transaction must be prepared".to_string());
        }
        ensure_single_book_transaction(root, transaction)?;
        save_transaction(root, transaction)?;
        transaction.clone()
    };
    advance(root, &mut current, None)?;
    Ok(current)
}

#[cfg(test)]
pub(crate) fn commit_transaction_until(
    root: &Path,
    transaction: &PublishTransaction,
    stop_after: Option<PublishPhase>,
) -> Result<PublishTransaction, String> {
    let mut current = transaction.clone();
    save_transaction(root, &current)?;
    inject_stop(stop_after, PublishPhase::Prepared)?;
    advance(root, &mut current, stop_after)?;
    Ok(current)
}

pub fn recover_transaction(
    root: &Path,
    transaction_id: &str,
) -> Result<PublishTransaction, String> {
    let mut transaction = load_transaction(root, transaction_id)?;
    advance(root, &mut transaction, None)?;
    Ok(transaction)
}
