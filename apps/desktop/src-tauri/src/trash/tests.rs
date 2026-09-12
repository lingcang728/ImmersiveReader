use super::{list, move_book, permanently_delete, reconcile, restore, restore_idempotent};
use crate::contracts::Manifest;
use crate::control::ControlDb;
use std::fs;
use std::path::{Path, PathBuf};

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

#[test]
fn reconcile_finishes_a_half_completed_permanent_delete() {
    // P3-15: a crash inside `remove_dir_all` leaves `.trash/<id>` partially
    // deleted — entry gone, content left — plus the delete journal. Reconcile
    // must finish the delete instead of leaving an unloadable orphan.
    let (root, book, manifest) = fixture("partial-delete");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    let item_root = root.join(".trash").join(&moved.trash_id);
    fs::remove_file(item_root.join("trash-entry.json"))
        .expect("entry must be removed for crash simulation");
    fs::remove_file(item_root.join("manifest.json")).expect("partial delete simulation");
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

    reconcile(&root).expect("reconciliation must finish the delete");

    assert!(!item_root.exists(), "half-deleted item must be removed");
    assert!(!root
        .join(".trash")
        .join(".journal")
        .join(format!("{}.json", moved.trash_id))
        .exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn reconcile_cleans_stray_entry_after_pre_rename_move_crash() {
    // P3-15: the entry is written into the book dir before the rename so it
    // travels into `.trash`. A crash in that window leaves the book on the
    // shelf with a stray `trash-entry.json` and the prepared journal —
    // reconcile must remove both and keep the book listed.
    let (root, book, manifest) = fixture("pre-rename-crash");
    let moved = super::TrashItem {
        schema_version: 1,
        trash_id: "orphan-move1".to_string(),
        book_id: manifest.book_id.clone(),
        title: manifest.title.clone(),
        original_relative_path: "手动/测试书目".to_string(),
        trash_relative_path: ".trash/orphan-move1".to_string(),
        deleted_at: "2026-07-10T00:00:00Z".to_string(),
        revision: 1,
    };
    fs::write(
        book.join("trash-entry.json"),
        serde_json::to_vec_pretty(&moved).expect("entry must serialize"),
    )
    .expect("stray entry must be written");
    super::write_journal(
        &root,
        &super::TrashJournal {
            schema_version: 1,
            operation: "move".to_string(),
            trash_id: moved.trash_id.clone(),
            phase: "prepared".to_string(),
            item: moved,
        },
    )
    .expect("journal must write");

    reconcile(&root).expect("reconciliation must clean the stray entry");

    assert!(!book.join("trash-entry.json").exists());
    assert!(book.join("manifest.json").exists());
    assert!(!root
        .join(".trash")
        .join(".journal")
        .join("orphan-move1.json")
        .exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

fn write_raw_journal(root: &Path, name: &str, value: serde_json::Value) {
    let journal_dir = root.join(".trash").join(".journal");
    fs::create_dir_all(&journal_dir).expect("journal dir must exist");
    fs::write(
        journal_dir.join(name),
        serde_json::to_vec_pretty(&value).expect("journal must serialize"),
    )
    .expect("journal must write");
}

fn journal_item(trash_id: &str, original_relative_path: &str) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "trashId": trash_id,
        "bookId": "manual:planted",
        "title": "planted",
        "originalRelativePath": original_relative_path,
        "trashRelativePath": format!(".trash/{trash_id}"),
        "deletedAt": "2026-07-10T00:00:00Z",
        "revision": 1,
    })
}

#[test]
fn reconcile_skips_corrupt_journals_and_keeps_processing() {
    let (root, book, manifest) = fixture("bad-journal");
    let moved = move_book(&root, &book, &manifest).expect("book must move to trash");
    // A journal that is not valid JSON, a valid journal whose operation can
    // never resolve, and a good crash-recovery journal coexist in one pass.
    let journal_dir = root.join(".trash").join(".journal");
    fs::create_dir_all(&journal_dir).expect("journal dir must exist");
    fs::write(journal_dir.join("garbage.json"), b"not json").expect("garbage must write");
    write_raw_journal(
        &root,
        "unsafe-path.json",
        serde_json::json!({
            "schemaVersion": 1,
            "operation": "restore",
            "trashId": "abad1dea",
            "phase": "prepared",
            "item": journal_item("abad1dea", "../outside"),
        }),
    );
    let item_root = root.join(".trash").join(&moved.trash_id);
    fs::remove_file(item_root.join("trash-entry.json"))
        .expect("metadata must be removed for crash simulation");
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

    // The corrupt entries are skipped, the recoverable one still completes.
    reconcile(&root).expect("reconciliation must survive bad journals");
    let items = list(&root).expect("trash must still list");
    assert_eq!(items, vec![moved]);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn reconcile_ignores_planted_journals_that_escape_the_trash_root() {
    let (root, _book, _manifest) = fixture("planted-journal");
    // A planted journal whose trashId does not pass validate_id must never be
    // used to build a path — ".." would resolve item_root to the library root.
    write_raw_journal(
        &root,
        "planted.json",
        serde_json::json!({
            "schemaVersion": 1,
            "operation": "move",
            "trashId": "..",
            "phase": "renamed",
            "item": journal_item("..", "manual/book"),
        }),
    );
    reconcile(&root).expect("reconciliation must skip the planted journal");
    assert!(!root.join("trash-entry.json").exists());

    // An inconsistent item payload is rejected before any entry is written.
    write_raw_journal(
        &root,
        "mismatched.json",
        serde_json::json!({
            "schemaVersion": 1,
            "operation": "move",
            "trashId": "mismatch1",
            "phase": "renamed",
            "item": journal_item("other-id9", "manual/book"),
        }),
    );
    let ghost = root.join(".trash").join("mismatch1");
    fs::create_dir_all(&ghost).expect("ghost item dir must exist");
    reconcile(&root).expect("reconciliation must skip the mismatched journal");
    assert!(!ghost.join("trash-entry.json").exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn parse_relative_rejects_everything_the_contract_rule_rejects() {
    // Dual check with contracts::is_safe_relative_path: every string the shared
    // contract rejects must also be rejected for trash-managed paths.
    for value in [
        "",
        "   ",
        "/abs.md",
        "C:abs.md",
        "C:/abs.md",
        "a\\b.md",
        "a\0b.md",
        "a//b.md",
        "a/",
        "./a.md",
        "a/./b.md",
        "..",
        "../a.md",
        "a/../b.md",
    ] {
        assert!(
            !crate::contracts::is_safe_relative_path(value),
            "contract rule must reject {value:?}"
        );
        assert!(
            super::parse_relative(value).is_err(),
            "trash rule must reject {value:?}"
        );
    }
    // Deliberately stricter than the contract: dot-leading segments are barred
    // so restored paths can never reach managed directories.
    for value in [".trash/x", ".journal/y.json", ".hidden/a.md"] {
        assert!(crate::contracts::is_safe_relative_path(value));
        assert!(super::parse_relative(value).is_err());
    }
    for value in ["手动/测试书目", "a/b/c.md"] {
        assert!(crate::contracts::is_safe_relative_path(value));
        assert!(super::parse_relative(value).is_ok());
    }
}
