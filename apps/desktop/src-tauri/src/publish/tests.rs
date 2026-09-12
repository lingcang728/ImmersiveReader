use super::{
    commit_transaction, commit_transaction_until,
    hash_file, load_transaction, recover_transaction,
    transaction::CrashPoint, PublishPhase, PublishTransaction,
};
use std::fs;
use std::path::{Path, PathBuf};

fn root(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("immersive-publish-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("test root must exist");
    path
}

fn write_book_with_identity(path: &Path, revision: u64, title: &str, book_id: &str, source_id: &str) {
    fs::create_dir_all(path).expect("book directory must exist");
    fs::write(
        path.join("manifest.json"),
        format!(
            r#"{{"schemaVersion":1,"bookId":"{book_id}","title":"{title}","source":"podcast","sourceId":"{source_id}","generatedAt":"2026-07-11","updatedAt":"2026-07-11","chapters":[]}}"#
        ),
    )
    .expect("manifest must write");
    let manifest_hash = hash_file(&path.join("manifest.json")).expect("manifest must hash");
    fs::write(
        path.join("provenance.json"),
        format!(
            r#"{{"schemaVersion":1,"bookId":"{book_id}","sourceId":"{source_id}","sourceKind":"podcast","createdByTaskId":"task-1","lastSuccessfulTaskId":"task-1","revision":{revision},"manifestSha256":"{manifest_hash}","engineVersion":"test","updatedAt":"2026-07-11"}}"#
        ),
    )
    .expect("provenance must write");
}

fn write_book(path: &Path, revision: u64, title: &str) {
    write_book_with_identity(path, revision, title, "podcast:abc", "abc");
}

fn prepared(root: &Path) -> PublishTransaction {
    let incoming = root.join(".incoming/tx-1");
    write_book(&incoming, 2, "new");
    PublishTransaction {
        schema_version: 1,
        transaction_id: "tx-1".to_string(),
        task_id: "task-1".to_string(),
        book_id: "podcast:abc".to_string(),
        // Journals always store forward-slash relative paths — managed_relative
        // enforces the shared contract and rejects `\` separators outright.
        incoming_relative_path: ".incoming/tx-1".to_string(),
        final_relative_path: "Podcast/abc".to_string(),
        rollback_relative_path: ".revisions/podcast-abc/1".to_string(),
        manifest_sha256: hash_file(&incoming.join("manifest.json")).expect("manifest must hash"),
        provenance_sha256: hash_file(&incoming.join("provenance.json"))
            .expect("provenance must hash"),
        revision: 2,
        phase: PublishPhase::Prepared,
        created_at: "2026-07-11T00:00:00Z".to_string(),
        updated_at: "2026-07-11T00:00:00Z".to_string(),
    }
}

#[test]
fn recovers_idempotently_after_old_version_was_moved() {
    let root = root("old-moved");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    let transaction = prepared(&root);

    commit_transaction_until(
        &root,
        &transaction,
        Some(CrashPoint::AfterPhase(PublishPhase::OldMoved)),
    )
    .expect_err("crash injection must interrupt publication");
    let interrupted = load_transaction(&root, "tx-1").expect("journal must load");
    assert_eq!(interrupted.phase, PublishPhase::OldMoved);
    assert!(!root.join(r"Podcast\abc").exists());

    let recovered = recover_transaction(&root, "tx-1").expect("recovery must succeed");
    assert_eq!(recovered.phase, PublishPhase::Committed);
    assert!(root.join(r"Podcast\abc").exists());
    let repeated = recover_transaction(&root, "tx-1").expect("recovery must be idempotent");
    assert_eq!(repeated.phase, PublishPhase::Committed);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn recovers_idempotently_from_prepared_journal() {
    let root = root("prepared");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    let transaction = prepared(&root);

    commit_transaction_until(
        &root,
        &transaction,
        Some(CrashPoint::AfterPhase(PublishPhase::Prepared)),
    )
    .expect_err("crash injection must interrupt publication");
    let interrupted = load_transaction(&root, "tx-1").expect("journal must load");
    assert_eq!(interrupted.phase, PublishPhase::Prepared);

    let recovered = recover_transaction(&root, "tx-1").expect("recovery must succeed");
    assert_eq!(recovered.phase, PublishPhase::Committed);
    let repeated = recover_transaction(&root, "tx-1").expect("recovery must be idempotent");
    assert_eq!(repeated.phase, PublishPhase::Committed);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn corrupted_new_version_rolls_back_last_successful_book() {
    let root = root("new-moved");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    let transaction = prepared(&root);

    commit_transaction_until(
        &root,
        &transaction,
        Some(CrashPoint::AfterPhase(PublishPhase::NewMoved)),
    )
    .expect_err("crash injection must interrupt publication");
    fs::write(root.join(r"Podcast\abc\manifest.json"), b"corrupt")
        .expect("new manifest must be corruptible");

    let recovered = recover_transaction(&root, "tx-1").expect("recovery must roll back");
    assert_eq!(recovered.phase, PublishPhase::RolledBack);
    let manifest = fs::read_to_string(root.join(r"Podcast\abc\manifest.json"))
        .expect("old manifest must be restored");
    assert!(manifest.contains(r#""title":"old""#));
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn rejects_two_active_transactions_for_the_same_book() {
    let root = root("book-lock");
    let first = prepared(&root);
    commit_transaction_until(
        &root,
        &first,
        Some(CrashPoint::AfterPhase(PublishPhase::Prepared)),
    )
    .expect_err("first transaction must remain prepared");
    let mut second = first.clone();
    second.transaction_id = "tx-2".to_string();

    let error = commit_transaction(&root, &second)
        .expect_err("second transaction for the same book must be rejected");

    assert!(error.contains("active for this book"));
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn prepared_validation_failure_preserves_last_successful_book() {
    let root = root("prepared-validation-failure");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    let transaction = prepared(&root);
    fs::write(
        root.join(r".incoming\tx-1\manifest.json"),
        b"not-json",
    )
    .expect("incoming manifest must be corruptible");

    let recovered = commit_transaction(&root, &transaction).expect("failure must be journaled");
    assert_eq!(recovered.phase, PublishPhase::RolledBack);
    let old_manifest = fs::read_to_string(root.join(r"Podcast\abc\manifest.json"))
        .expect("old final must remain readable");
    assert!(old_manifest.contains(r#""title":"old""#));
    assert!(!root.join(r".incoming\failed-tx-1").exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn rejects_a_final_path_owned_by_another_book() {
    let root = root("final-path-identity");
    write_book_with_identity(
        &root.join(r"Podcast\abc"),
        1,
        "old other",
        "podcast:other",
        "other",
    );
    let transaction = prepared(&root);

    let error = commit_transaction(&root, &transaction)
        .expect_err("different book identity must not share a final path");
    assert!(error.contains("already belongs to another book"));
    assert!(root.join(r"Podcast\abc").exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn recovers_when_killed_between_incoming_rename_and_journal() {
    // P2-13: the process died after `rename(incoming, final)` but before the
    // NewMoved journal write — the journal says OldMoved while final already
    // holds the new book. Recovery must finish the commit, not deadlock.
    let root = root("new-move-unjournaled");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    let transaction = prepared(&root);

    commit_transaction_until(&root, &transaction, Some(CrashPoint::NewMoveBeforeJournal))
        .expect_err("crash injection must interrupt publication");
    let interrupted = load_transaction(&root, "tx-1").expect("journal must load");
    assert_eq!(interrupted.phase, PublishPhase::OldMoved);
    assert!(root.join(r"Podcast\abc").exists(), "rename already ran");
    assert!(!root.join(r".incoming\tx-1").exists(), "incoming is gone");
    assert!(root.join(r".revisions\podcast-abc\1").exists());

    let recovered = recover_transaction(&root, "tx-1").expect("recovery must succeed");
    assert_eq!(recovered.phase, PublishPhase::Committed);
    let manifest = fs::read_to_string(root.join(r"Podcast\abc\manifest.json"))
        .expect("final manifest must read");
    assert!(manifest.contains(r#""title":"new""#));
    assert!(root.join(r".revisions\podcast-abc\1").exists());
    let repeated = recover_transaction(&root, "tx-1").expect("recovery must be idempotent");
    assert_eq!(repeated.phase, PublishPhase::Committed);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn recovers_when_killed_between_old_move_and_journal() {
    // The symmetric window: final→rollback rename ran but the OldMoved journal
    // write never happened. Journal says Prepared; final is already gone.
    let root = root("old-move-unjournaled");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    let transaction = prepared(&root);

    commit_transaction_until(&root, &transaction, Some(CrashPoint::OldMoveBeforeJournal))
        .expect_err("crash injection must interrupt publication");
    let interrupted = load_transaction(&root, "tx-1").expect("journal must load");
    assert_eq!(interrupted.phase, PublishPhase::Prepared);
    assert!(!root.join(r"Podcast\abc").exists(), "old book already moved");
    assert!(root.join(r".revisions\podcast-abc\1").exists());

    let recovered = recover_transaction(&root, "tx-1").expect("recovery must succeed");
    assert_eq!(recovered.phase, PublishPhase::Committed);
    let manifest = fs::read_to_string(root.join(r"Podcast\abc\manifest.json"))
        .expect("final manifest must read");
    assert!(manifest.contains(r#""title":"new""#));
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn rollback_quarantines_an_unvalidated_final_instead_of_deadlocking() {
    // Journal OldMoved + incoming gone + final holds content that is NOT this
    // transaction's verified book. Rollback must quarantine it and restore the
    // archived version — never a permanent "unexpectedly has a final path".
    let root = root("old-moved-foreign-final");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    let transaction = prepared(&root);
    commit_transaction_until(
        &root,
        &transaction,
        Some(CrashPoint::AfterPhase(PublishPhase::OldMoved)),
    )
    .expect_err("crash injection must interrupt publication");
    // Simulate foreign/corrupt content landing at final while incoming vanishes.
    fs::remove_dir_all(root.join(r".incoming\tx-1")).expect("incoming removable");
    write_book_with_identity(&root.join(r"Podcast\abc"), 9, "foreign", "podcast:zzz", "zzz");

    let recovered = recover_transaction(&root, "tx-1").expect("recovery must roll back");
    assert_eq!(recovered.phase, PublishPhase::RolledBack);
    let manifest = fs::read_to_string(root.join(r"Podcast\abc\manifest.json"))
        .expect("old manifest must be restored");
    assert!(manifest.contains(r#""title":"old""#));
    let quarantined =
        fs::read_to_string(root.join(r".incoming\failed-tx-1\manifest.json"))
            .expect("foreign final must be quarantined, not destroyed");
    assert!(quarantined.contains(r#""title":"foreign""#));
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn rollback_without_any_archive_still_converges() {
    // First publish: no rollback copy exists. If incoming also vanishes there
    // is nothing to restore — the transaction must still reach a terminal
    // state so a re-publish can start cleanly instead of erroring forever.
    let root = root("first-publish-loss");
    let transaction = prepared(&root); // final never existed
    commit_transaction_until(
        &root,
        &transaction,
        Some(CrashPoint::AfterPhase(PublishPhase::OldMoved)),
    )
    .expect_err("crash injection must interrupt publication");
    fs::remove_dir_all(root.join(r".incoming\tx-1")).expect("incoming removable");

    let recovered = recover_transaction(&root, "tx-1").expect("recovery must converge");
    assert_eq!(recovered.phase, PublishPhase::RolledBack);
    assert!(!root.join(r"Podcast\abc").exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn rejects_a_republish_whose_archive_slot_is_occupied() {
    // The protective error stays: a fresh Prepared transaction may not
    // silently overwrite an already-archived rollback directory. Callers that
    // legitimately retry pick a free archive slot (podcast publish does).
    let root = root("rollback-occupied");
    write_book(&root.join(r"Podcast\abc"), 1, "old");
    write_book(&root.join(r".revisions\podcast-abc\1"), 0, "archived");
    let transaction = prepared(&root);

    let error = commit_transaction(&root, &transaction)
        .expect_err("occupied rollback path must be rejected");
    assert!(error.contains("rollback path already exists"));
    let manifest = fs::read_to_string(root.join(r"Podcast\abc\manifest.json"))
        .expect("existing final must be untouched");
    assert!(manifest.contains(r#""title":"old""#));
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn managed_relative_matches_contract_path_verdicts() {
    // P1-22 leftover: managed_relative must be exactly as strict as the shared
    // contract check (same verdict set as contracts::is_safe_relative_path).
    let root = root("managed-relative");
    for path in [
        "",
        "   ",
        "/abs",
        "C:abs",
        "c:/abs",
        "sub\\001",
        "a\0b",
        "a//b",
        "./a",
        "a/./b",
        "..",
        "../a",
        "a/../b",
        "a/",
        ".incoming/tx-1",
        "Podcast/abc",
        ".revisions/podcast-abc/1",
        "播客/episode-abc",
        ".hidden/x",
    ] {
        assert_eq!(
            super::validation::managed_relative(&root, path).is_ok(),
            crate::contracts::is_safe_relative_path(path),
            "verdict mismatch for {path:?}"
        );
    }
    fs::remove_dir_all(root).expect("fixture must be removed");
}
