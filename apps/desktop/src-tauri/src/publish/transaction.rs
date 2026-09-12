use super::validation::{managed_relative, validate_book};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

static BOOK_CLAIM_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

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
    let transactions = entries
        .into_iter()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "json")
        })
        // P3-18: per-entry tolerance (same skip-and-log pattern as
        // `trash::reconcile` / `LibraryIssue`) — one corrupt journal must not
        // hide every other transaction.
        .filter_map(|entry| {
            let id = entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_string);
            match id {
                Some(id) => match load_transaction(root, &id) {
                    Ok(transaction) => Some(transaction),
                    Err(error) => {
                        eprintln!("list_transactions: skipping unreadable journal {id}: {error}");
                        None
                    }
                },
                None => {
                    eprintln!(
                        "list_transactions: skipping journal with invalid file name {}",
                        entry.path().display()
                    );
                    None
                }
            }
        })
        .collect();
    Ok(transactions)
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

fn ensure_final_path_identity(
    root: &Path,
    transaction: &PublishTransaction,
) -> Result<(), String> {
    let final_path = managed_relative(root, &transaction.final_relative_path)?;
    if !final_path.exists() {
        return Ok(());
    }

    let manifest_path = final_path.join("manifest.json");
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(&manifest_path).map_err(|error| {
            format!("Final publish path exists but its manifest cannot be read: {error}")
        })?,
    )
    .map_err(|error| format!("Final publish path has invalid metadata: {error}"))?;
    let existing_book_id = manifest
        .get("bookId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Final publish path manifest has no bookId".to_string())?;
    if existing_book_id != transaction.book_id {
        return Err(format!(
            "Final publish path already belongs to another book: {existing_book_id}"
        ));
    }
    Ok(())
}

fn book_claim_lock(root: &Path, book_id: &str) -> Result<Arc<Mutex<()>>, String> {
    let key = format!("{}\u{0}{book_id}", root.to_string_lossy());
    let locks = BOOK_CLAIM_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .map_err(|_| "Publish claim lock registry is poisoned".to_string())?;
    Ok(locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
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

/// Test-only crash injection points. `AfterPhase` dies right after a phase
/// journal write persisted (the classic "mid-flight" case). The
/// `*BeforeJournal` variants die in the window between the filesystem rename
/// and the journal write that records it — P2-13 showed this window was real:
/// a kill there used to leave recovery deadlocked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CrashPoint {
    /// `phase` was journaled, then the process died.
    AfterPhase(PublishPhase),
    /// final→rollback rename completed but the OldMoved journal write did not.
    /// On disk: journal Prepared, rollback occupied, final missing.
    OldMoveBeforeJournal,
    /// incoming→final rename completed but the NewMoved journal write did not.
    /// On disk: journal OldMoved, final holds the new book, incoming gone.
    NewMoveBeforeJournal,
}

fn inject_crash(crash: Option<CrashPoint>, point: CrashPoint) -> Result<(), String> {
    if crash == Some(point) {
        return Err(format!("Injected crash at {point:?}"));
    }
    Ok(())
}

/// Move unvalidated content at the final path into `.incoming/failed-<tx>` so
/// rollback never destroys data — the quarantined copy stays recoverable.
fn quarantine_final(
    root: &Path,
    transaction: &PublishTransaction,
    final_path: &Path,
) -> Result<(), String> {
    if !final_path.exists() {
        return Ok(());
    }
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
    fs::rename(final_path, failed).map_err(|error| error.to_string())
}

/// Restore the archived previous version to the final path. A missing rollback
/// path is not an error: first publishes (and externally cleaned roots) have
/// nothing to restore, and erroring here used to deadlock recovery forever.
fn restore_rollback(
    transaction: &PublishTransaction,
    rollback_path: &Path,
    final_path: &Path,
) -> Result<(), String> {
    if !rollback_path.exists() {
        eprintln!(
            "publish transaction {} has no rollback copy to restore",
            transaction.transaction_id
        );
        return Ok(());
    }
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::rename(rollback_path, final_path).map_err(|error| error.to_string())
}

fn rollback(root: &Path, transaction: &mut PublishTransaction) -> Result<(), String> {
    let final_path = managed_relative(root, &transaction.final_relative_path)?;
    let rollback_path = managed_relative(root, &transaction.rollback_relative_path)?;
    match transaction.phase {
        PublishPhase::Prepared => set_phase(root, transaction, PublishPhase::RolledBack),
        PublishPhase::OldMoved => {
            // Anything left at final here failed validation (a valid final
            // short-circuits to NewMoved inside `advance`), so quarantine it
            // exactly like the NewMoved path instead of deadlocking on the
            // old "unexpectedly has a final path" hard error.
            quarantine_final(root, transaction, &final_path)?;
            // A first publish has no rollback copy — nothing to restore.
            restore_rollback(transaction, &rollback_path, &final_path)?;
            set_phase(root, transaction, PublishPhase::RolledBack)
        }
        PublishPhase::NewMoved => {
            quarantine_final(root, transaction, &final_path)?;
            restore_rollback(transaction, &rollback_path, &final_path)?;
            set_phase(root, transaction, PublishPhase::RolledBack)
        }
        PublishPhase::Committed => Err("Committed transaction cannot be rolled back".to_string()),
        PublishPhase::RolledBack => Ok(()),
    }
}

fn advance(
    root: &Path,
    transaction: &mut PublishTransaction,
    crash: Option<CrashPoint>,
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
                    inject_crash(crash, CrashPoint::OldMoveBeforeJournal)?;
                }
                set_phase(root, transaction, PublishPhase::OldMoved)?;
                inject_crash(crash, CrashPoint::AfterPhase(PublishPhase::OldMoved))?;
            }
            PublishPhase::OldMoved => {
                if validate_book(root, &transaction.incoming_relative_path, transaction).is_err() {
                    // P2-13: a kill between `rename(incoming, final)` and the
                    // NewMoved journal write leaves incoming gone while final
                    // already holds this transaction's verified book. The move
                    // finished — journal it and continue forward instead of
                    // rolling back into a live final path (that deadlocked).
                    if validate_book(root, &transaction.final_relative_path, transaction).is_ok() {
                        set_phase(root, transaction, PublishPhase::NewMoved)?;
                        inject_crash(crash, CrashPoint::AfterPhase(PublishPhase::NewMoved))?;
                        continue;
                    }
                    rollback(root, transaction)?;
                    return Ok(());
                }
                let incoming = managed_relative(root, &transaction.incoming_relative_path)?;
                let final_path = managed_relative(root, &transaction.final_relative_path)?;
                if let Some(parent) = final_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::rename(incoming, final_path).map_err(|error| error.to_string())?;
                inject_crash(crash, CrashPoint::NewMoveBeforeJournal)?;
                set_phase(root, transaction, PublishPhase::NewMoved)?;
                inject_crash(crash, CrashPoint::AfterPhase(PublishPhase::NewMoved))?;
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
    let journal = journal_path(root, &transaction.transaction_id)?;
    let mut current;
    if journal.exists() {
        current = load_transaction(root, &transaction.transaction_id)?;
    } else {
        if transaction.phase != PublishPhase::Prepared {
            return Err("New publish transaction must be prepared".to_string());
        }
        let lock = book_claim_lock(root, &transaction.book_id)?;
        let _claim_guard = lock
            .lock()
            .map_err(|_| "Publish claim lock is poisoned".to_string())?;
        if journal.exists() {
            current = load_transaction(root, &transaction.transaction_id)?;
        } else {
            ensure_single_book_transaction(root, transaction)?;
            ensure_final_path_identity(root, transaction)?;
            save_transaction(root, transaction)?;
            current = transaction.clone();
        }
    }
    advance(root, &mut current, None)?;
    Ok(current)
}

#[cfg(test)]
pub(crate) fn commit_transaction_until(
    root: &Path,
    transaction: &PublishTransaction,
    crash: Option<CrashPoint>,
) -> Result<PublishTransaction, String> {
    let mut current = transaction.clone();
    save_transaction(root, &current)?;
    inject_crash(crash, CrashPoint::AfterPhase(PublishPhase::Prepared))?;
    advance(root, &mut current, crash)?;
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
