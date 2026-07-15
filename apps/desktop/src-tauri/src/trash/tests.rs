use super::{list, move_book, permanently_delete, reconcile, restore, restore_idempotent};
use crate::contracts::Manifest;
use crate::control::ControlDb;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> (PathBuf, PathBuf, Manifest) {
    let root = std::env::temp_dir().join(format!("immersive-trash-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let book = root.join("手动").join("测试书目");
    fs::create_dir_all(&book).expect("book root must exist");
    let mut manifest: Manifest = serde_json::from_str(include_str!(
        "../../../../../packages/contracts/fixtures/manifest.valid.json"
    ))
    .expect("fixture manifest must deserialize");
    manifest.book_id = "manual:trash-test".to_string();
    manifest.title = "测试书目".to_string();
    fs::write(
        book.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest must serialize"),
    )
    .expect("manifest must write");
    fs::write(book.join("chapter.md"), b"content").expect("chapter must write");
    (root, book, manifest)
}

#[test]
fn move_records_original_path_and_restore_removes_metadata() {
    let (root, book, manifest) = fixture("restore");

    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    let items = list(&root).expect("trash must list");
    assert_eq!(items, vec![moved.clone()]);
    assert!(!book.exists());

    let restored = restore(&root, &moved.trash_id, 1).expect("book must restore");

    assert_eq!(restored.book_id, manifest.book_id);
    assert!(book.is_dir());
    assert!(!book.join("trash-entry.json").exists());
    assert!(list(&root)
        .expect("trash must list after restore")
        .is_empty());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn restore_refuses_to_overwrite_a_conflicting_destination() {
    let (root, book, manifest) = fixture("conflict");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    fs::create_dir_all(&book).expect("conflicting destination must exist");
    fs::write(book.join("sentinel"), b"keep").expect("sentinel must write");

    let error = restore(&root, &moved.trash_id, 1).expect_err("restore must not overwrite");

    assert_eq!(error, "CONFLICT");
    assert_eq!(
        fs::read(book.join("sentinel")).expect("sentinel must remain"),
        b"keep"
    );
    assert_eq!(list(&root).expect("trash item must remain").len(), 1);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn permanent_delete_removes_only_the_selected_trash_item() {
    let (root, book, manifest) = fixture("delete");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    let unrelated = root.join(".trash").join("legacy-unmanaged");
    fs::create_dir_all(&unrelated).expect("legacy directory must exist");
    fs::write(unrelated.join("sentinel"), b"keep").expect("legacy sentinel must write");

    let result = permanently_delete(&root, &moved.trash_id, 1).expect("item must delete");

    assert!(result.deleted_items >= 2);
    assert!(result.released_bytes > 0);
    assert!(unrelated.is_dir());
    assert!(list(&root).expect("managed trash must be empty").is_empty());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn restored_result_replays_after_database_reopen() {
    let (root, book, manifest) = fixture("idempotent");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    let control_path = root.join("control.db");
    let first = {
        let control = ControlDb::open(&control_path).expect("control database must open");
        restore_idempotent(&root, &control, &moved.trash_id, 1, "request-restore")
            .expect("first restore must succeed")
    };

    let replay = {
        let reopened = ControlDb::open(&control_path).expect("control database must reopen");
        restore_idempotent(&root, &reopened, &moved.trash_id, 1, "request-restore")
            .expect("restore result must replay")
    };

    assert_eq!(replay, first);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn reconcile_repairs_renamed_move_without_metadata() {
    let (root, book, manifest) = fixture("move-journal");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    let entry = root
        .join(".trash")
        .join(&moved.trash_id)
        .join("trash-entry.json");
    fs::remove_file(&entry).expect("metadata must be removed for crash simulation");
    super::write_journal(
        &root,
        &super::TrashJournal {
            schema_version: 1,
            operation: "move".to_string(),
            trash_id: moved.trash_id.clone(),
            phase: "renamed".to_string(),
            item: moved.clone(),
        },
    )
    .expect("journal must write");

    let items = list(&root).expect("reconciliation must complete the move");
    assert_eq!(items, vec![moved]);
    assert!(entry.exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn reconcile_restores_metadata_after_restore_crash() {
    let (root, book, manifest) = fixture("restore-journal");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    let item_root = root.join(".trash").join(&moved.trash_id);
    fs::remove_file(item_root.join("trash-entry.json"))
        .expect("metadata must be removed for crash simulation");
    super::write_journal(
        &root,
        &super::TrashJournal {
            schema_version: 1,
            operation: "restore".to_string(),
            trash_id: moved.trash_id.clone(),
            phase: "metadata_removed".to_string(),
            item: moved.clone(),
        },
    )
    .expect("journal must write");

    reconcile(&root).expect("reconciliation must restore metadata");
    assert!(item_root.join("trash-entry.json").exists());
    assert_eq!(list(&root).expect("trash must list").len(), 1);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn reconcile_removes_completed_delete_journal() {
    let (root, book, manifest) = fixture("delete-journal");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    let item_root = root.join(".trash").join(&moved.trash_id);
    fs::remove_dir_all(&item_root).expect("content must be removed for crash simulation");
    super::write_journal(
        &root,
        &super::TrashJournal {
            schema_version: 1,
            operation: "permanent_delete".to_string(),
            trash_id: moved.trash_id.clone(),
            phase: "prepared".to_string(),
            item: moved.clone(),
        },
    )
    .expect("journal must write");

    reconcile(&root).expect("reconciliation must remove completed journal");
    assert!(!root
        .join(".trash")
        .join(".journal")
        .join(format!("{}.json", moved.trash_id))
        .exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}
