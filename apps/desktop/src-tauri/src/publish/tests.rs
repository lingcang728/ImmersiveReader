use super::{
    commit_transaction, commit_transaction_until, hash_file, load_transaction, recover_transaction,
    PublishPhase, PublishTransaction,
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
    let incoming = root.join(r".incoming\tx-1");
    write_book(&incoming, 2, "new");
    PublishTransaction {
        schema_version: 1,
        transaction_id: "tx-1".to_string(),
        task_id: "task-1".to_string(),
        book_id: "podcast:abc".to_string(),
        incoming_relative_path: r".incoming\tx-1".to_string(),
        final_relative_path: r"Podcast\abc".to_string(),
        rollback_relative_path: r".revisions\podcast-abc\1".to_string(),
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

    commit_transaction_until(&root, &transaction, Some(PublishPhase::OldMoved))
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

    commit_transaction_until(&root, &transaction, Some(PublishPhase::Prepared))
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

    commit_transaction_until(&root, &transaction, Some(PublishPhase::NewMoved))
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
    commit_transaction_until(&root, &first, Some(PublishPhase::Prepared))
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
