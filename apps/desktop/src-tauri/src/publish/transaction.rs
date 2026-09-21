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
    // Accept integral floats (`1.0`) — the schema `const` is numeric-equal
    // semantics, and a journal written/rewritten by another implementation
    // must not strand publish recovery.
    #[serde(deserialize_with = "crate::contracts::deserialize_schema_version")]
    pub schema_version: u32,
    pub transaction_id: String,
    pub task_id: String,
    pub book_id: String,
    pub incoming_relative_path: String,
    pub final_relative_path: String,
    pub rollback_relative_path: String,
    pub manifest_sha256: String,
    pub provenance_sha256: String,
    #[serde(deserialize_with = "crate::contracts::deserialize_u64")]
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

/// A journal that fails to load wedges the book permanently: the commit path
/// keeps refusing to write a fresh one while the unreadable file sits in
/// `.transactions`, and the recovery UI cannot even show it. Move it aside to
/// `<id>.json.corrupt-<epoch>` instead — evidence is preserved, the name no
/// longer collides with a fresh journal, and `list_transactions`/`exists()`
/// stop seeing it. Returns Ok(false) when the journal was already gone.
fn quarantine_unreadable_journal(root: &Path, transaction_id: &str) -> Result<bool, String> {
    let journal = journal_path(root, transaction_id)?;
    if !journal.exists() {
        return Ok(false);
    }
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    for attempt in 0..10_u32 {
        let suffix = if attempt == 0 {
            format!("corrupt-{epoch}")
        } else {
            format!("corrupt-{epoch}-{attempt}")
        };
        let quarantined = journal.with_file_name(format!("{transaction_id}.json.{suffix}"));
        match fs::rename(&journal, &quarantined) {
            Ok(()) => {
                crate::storage::app_log(
                    "publish",
                    &format!(
                        "unreadable publish journal quarantined: {}",
                        quarantined.display()
                    ),
                );
                return Ok(true);
            }
            // A previous quarantine may hold the same epoch name.
            Err(_) if quarantined.exists() => continue,
            Err(error) => {
                return Err(format!(
                    "failed to quarantine unreadable publish journal {}: {error}",
                    journal.display()
                ))
            }
        }
    }
    Err(format!(
        "failed to quarantine unreadable publish journal {}",
        journal.display()
    ))
}

/// Phase the journal file at `.transactions/<name>.json` reports — `None`
/// means no protective journal exists: file absent, unparseable (quarantined
/// on sight so the next sweep frees the staging dir), or already terminal.
fn non_terminal_journal(root: &Path, name: &str) -> Option<PublishTransaction> {
    let journal = root.join(".transactions").join(format!("{name}.json"));
    if !journal.exists() {
        return None;
    }
    match load_transaction(root, name) {
        Ok(transaction)
            if !matches!(
                transaction.phase,
                PublishPhase::Committed | PublishPhase::RolledBack
            ) =>
        {
            Some(transaction)
        }
        Ok(_) => None,
        Err(error) => {
            eprintln!("unreadable publish journal {name}: {error}; quarantining");
            let _ = quarantine_unreadable_journal(root, name);
            None
        }
    }
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
    let mut transactions = Vec::new();
    for entry in entries {
        if !entry
            .path()
            .extension()
            .is_some_and(|value| value == "json")
        {
            continue;
        }
        let Some(id) = entry
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            eprintln!(
                "list_transactions: skipping journal with invalid file name {}",
                entry.path().display()
            );
            continue;
        };
        match load_transaction(root, &id) {
            Ok(transaction) => transactions.push(transaction),
            Err(error) => {
                // One corrupt journal must neither hide the rest nor wedge
                // its book's re-publish forever — quarantine it so the file
                // stops colliding with a fresh journal (kept as evidence).
                eprintln!("list_transactions: unreadable journal {id}: {error}");
                if let Err(quarantine_error) = quarantine_unreadable_journal(root, &id) {
                    eprintln!("list_transactions: could not quarantine {id}: {quarantine_error}");
                }
            }
        }
    }
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

fn ensure_final_path_identity(root: &Path, transaction: &PublishTransaction) -> Result<(), String> {
    let final_path = managed_relative(root, &transaction.final_relative_path)?;
    if !final_path.exists() {
        return Ok(());
    }

    let manifest_path = final_path.join("manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|error| {
            format!("Final publish path exists but its manifest cannot be read: {error}")
        })?)
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
/// An occupied `failed-<tx>` slot (a previous crashed attempt already left a
/// copy) gets a `-N` suffix instead of deadlocking recovery forever.
fn quarantine_final(
    root: &Path,
    transaction: &PublishTransaction,
    final_path: &Path,
) -> Result<(), String> {
    if !final_path.exists() {
        return Ok(());
    }
    let incoming = root.join(".incoming");
    let mut failed = incoming.join(format!("failed-{}", transaction.transaction_id));
    for attempt in 1..32_u32 {
        if !failed.exists() {
            break;
        }
        failed = incoming.join(format!("failed-{}-{attempt}", transaction.transaction_id));
    }
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

/// Find a free sibling of the recorded rollback path (`…-recover<N>`). The
/// recorded slot can hold a genuine archive from a previous crashed attempt —
/// relocating beats either overwriting it or wedging on "slot exists".
fn free_rollback_slot(root: &Path, recorded_relative: &str) -> Result<String, String> {
    for attempt in 1..=32_u32 {
        let candidate = format!("{recorded_relative}-recover{attempt}");
        if !managed_relative(root, &candidate)?.exists() {
            return Ok(candidate);
        }
    }
    Err("Publish rollback path already exists".to_string())
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
                let mut rollback_path =
                    managed_relative(root, &transaction.rollback_relative_path)?;
                if final_path.exists() {
                    if rollback_path.exists() {
                        // A previous attempt already archived into the recorded
                        // slot (its OldMoved journal write was lost). That copy
                        // may be the genuine pre-publish version — never
                        // overwrite it. Allocate a fresh `-recover<N>` slot and
                        // repoint the journal BEFORE the rename so a crash in
                        // between still converges on the next pass.
                        transaction.rollback_relative_path =
                            free_rollback_slot(root, &transaction.rollback_relative_path)?;
                        save_transaction(root, transaction)?;
                        rollback_path =
                            managed_relative(root, &transaction.rollback_relative_path)?;
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
    // F-03: the per-book lock must cover the WHOLE transaction lifecycle —
    // journal read, claim checks, and every advance() rename. The old code
    // only guarded the claim/create window, so a concurrent commit and a
    // recovery pass could race the same final/rollback paths.
    let lock = book_claim_lock(root, &transaction.book_id)?;
    let _claim_guard = lock
        .lock()
        .map_err(|_| "Publish claim lock is poisoned".to_string())?;
    let journal = journal_path(root, &transaction.transaction_id)?;
    let mut current;
    if journal.exists() {
        current = match load_transaction(root, &transaction.transaction_id) {
            Ok(loaded) => loaded,
            Err(error) => {
                // F-01: a journal we cannot parse wedges this book forever —
                // every retry hits `load` → Err before a fresh journal can be
                // written. Quarantine it (evidence preserved) and let this
                // commit start clean; `.revisions` residue stays archived.
                eprintln!(
                    "commit_transaction: unreadable journal {}: {error}; quarantining",
                    transaction.transaction_id
                );
                quarantine_unreadable_journal(root, &transaction.transaction_id)?;
                if transaction.phase != PublishPhase::Prepared {
                    return Err("New publish transaction must be prepared".to_string());
                }
                ensure_single_book_transaction(root, transaction)?;
                ensure_final_path_identity(root, transaction)?;
                save_transaction(root, transaction)?;
                transaction.clone()
            }
        };
    } else {
        if transaction.phase != PublishPhase::Prepared {
            return Err("New publish transaction must be prepared".to_string());
        }
        ensure_single_book_transaction(root, transaction)?;
        ensure_final_path_identity(root, transaction)?;
        save_transaction(root, transaction)?;
        current = transaction.clone();
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
    // F-03: recovery runs the same advance() renames as commit — take the
    // same per-book lock so it cannot interleave with an in-flight publish.
    // The journal is re-read under the lock so a just-committed transaction
    // observed stale cannot be driven twice.
    let peeked = load_transaction(root, transaction_id)?;
    let lock = book_claim_lock(root, &peeked.book_id)?;
    let _claim_guard = lock
        .lock()
        .map_err(|_| "Publish claim lock is poisoned".to_string())?;
    let mut transaction = load_transaction(root, transaction_id)?;
    advance(root, &mut transaction, None)?;
    Ok(transaction)
}

/// Keep `.revisions/<source>` rollback slots bounded — without a cap every
/// republish archives another copy forever.
const MAX_REVISION_SLOTS_PER_SOURCE: usize = 8;

/// P-11-F05: `failed-*` quarantine copies are the only salvage of rejected
/// content — they survive long enough to rescue (a month), then the sweep
/// reclaims them. The window is deliberately generous; anything newer is
/// never auto-deleted.
const FAILED_QUARANTINE_KEEP: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Startup GC for publish residue no journal can ever reclaim:
/// - `.incoming/<tx>` staging dirs that no *in-flight* journal protects —
///   journal missing, terminal (committed/rolled_back: the transaction is
///   over and any leftover staging is dead weight), or unreadable (quarantined
///   on sight so the next launch's sweep frees the dir). `failed-*`
///   quarantine copies are exempt until `FAILED_QUARANTINE_KEEP` old.
/// - `.transactions/*.json` journals in the terminal `rolled_back` state —
///   the rollback already ran, so the file is a dead record. `committed`
///   journals stay: both publishers use them for idempotent re-publish.
/// - `.revisions/<source>` slots beyond the newest few per source.
///
/// Runs inside `setup`, before any publish can be in flight — anything left
/// here is crash residue by definition.
pub fn sweep_publish_residue(root: &Path) {
    let incoming = root.join(".incoming");
    if let Ok(entries) = fs::read_dir(&incoming) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("failed-") {
                // Salvage dirs outlive every other residue — only age them
                // out after the retention window.
                let stale = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .map(|modified| {
                        modified
                            .elapsed()
                            .map(|age| age > FAILED_QUARANTINE_KEEP)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !stale {
                    continue;
                }
                if let Err(error) = fs::remove_dir_all(&path) {
                    eprintln!(
                        "publish residue sweep could not remove stale quarantine {}: {error}",
                        path.display()
                    );
                }
                continue;
            }
            // A reparse point gets unlinked, never traversed.
            let is_reparse = entry
                .metadata()
                .map(|meta| crate::atomic_file::is_reparse_point(&meta))
                .unwrap_or(false);
            if is_reparse {
                let _ = fs::remove_dir(&path);
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            // Staging dirs survive only while a non-terminal journal protects
            // them; terminal or missing journals make the staging dead weight.
            if non_terminal_journal(root, &name).is_some() {
                continue;
            }
            if let Err(error) = fs::remove_dir_all(&path) {
                eprintln!(
                    "publish residue sweep could not remove orphaned staging {}: {error}",
                    path.display()
                );
            }
        }
    }
    // Dead journals: a rolled_back entry has nothing left to recover — its
    // rollback already ran. Removing it also frees the `.incoming` staging
    // check above on the next pass (same name). Unreadable `.json` files were
    // already quarantined to `*.corrupt-*` by list_transactions / the check.
    let transactions_dir = root.join(".transactions");
    if let Ok(entries) = fs::read_dir(&transactions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|value| value == "json") {
                continue;
            }
            let Some(stem) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .map(str::to_string)
            else {
                continue;
            };
            match load_transaction(root, &stem) {
                Ok(transaction) if transaction.phase == PublishPhase::RolledBack => {
                    let _ = fs::remove_file(&path);
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = quarantine_unreadable_journal(root, &stem);
                }
            }
        }
    }
    let revisions = root.join(".revisions");
    if let Ok(sources) = fs::read_dir(&revisions) {
        for source in sources.flatten() {
            let source_path = source.path();
            let is_reparse = source
                .metadata()
                .map(|meta| crate::atomic_file::is_reparse_point(&meta))
                .unwrap_or(false);
            if is_reparse {
                let _ = fs::remove_dir(&source_path);
                continue;
            }
            if !source_path.is_dir() {
                continue;
            }
            let Ok(mut slots) = fs::read_dir(&source_path).map(|entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        let modified = entry
                            .metadata()
                            .and_then(|meta| meta.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        (entry.path(), modified)
                    })
                    .collect::<Vec<_>>()
            }) else {
                continue;
            };
            if slots.len() <= MAX_REVISION_SLOTS_PER_SOURCE {
                continue;
            }
            // Newest first; everything past the cap is deleted.
            slots.sort_by_key(|slot| std::cmp::Reverse(slot.1));
            for (slot, _) in slots.into_iter().skip(MAX_REVISION_SLOTS_PER_SOURCE) {
                let is_reparse = slot
                    .symlink_metadata()
                    .map(|meta| crate::atomic_file::is_reparse_point(&meta))
                    .unwrap_or(false);
                let result = if is_reparse {
                    fs::remove_dir(&slot)
                } else {
                    fs::remove_dir_all(&slot)
                };
                if let Err(error) = result {
                    eprintln!(
                        "publish residue sweep could not remove revision slot {}: {error}",
                        slot.display()
                    );
                }
            }
        }
    }
}
