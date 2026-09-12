use super::{execute_settings_migration, preview_for, LegacyLocations, MigrationScope};
use crate::control::ControlDb;
use crate::storage::StorageLocations;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> (PathBuf, LegacyLocations, StorageLocations) {
    let root = std::env::temp_dir().join(format!(
        "immersive-migration-execution-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("test root must exist");
    let legacy = LegacyLocations {
        settings: root.join(r"Legacy\settings.json"),
        immersive_state: root.join(r"Legacy\immersive-state"),
        mmbook_state: root.join(r"Legacy\mmbook-state"),
        podcast_root: root.join(r"Legacy\podcast"),
        zhihu_root: root.join(r"Legacy\zhihu"),
        library_root: root.join(r"Legacy\Library"),
    };
    let target = StorageLocations {
        channel: "test".to_string(),
        settings_path: root.join(r"Target\Settings\settings.json"),
        data_root: root.join(r"Target\Data"),
        cache_root: root.join(r"Target\Cache"),
        logs_root: root.join(r"Target\Logs"),
        runtime_state_root: root.join(r"Target\RuntimeState"),
        backups_root: root.join(r"Target\Backups"),
        library_root: root.join(r"Target\Library"),
        runtime_root: root.join(r"Target\Runtime"),
    };
    fs::create_dir_all(legacy.settings.parent().unwrap()).expect("legacy root must exist");
    fs::write(
        &legacy.settings,
        r#"{"schemaVersion":2,"libraryRoot":"D:\\Reader Library"}"#,
    )
    .expect("legacy settings must write");
    (root, legacy, target)
}

#[test]
fn settings_migration_is_verified_receipted_and_idempotent() {
    let (root, legacy, target) = fixture("success");
    let preview =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("preview must succeed");

    let first =
        execute_settings_migration(&legacy, &target, &preview.preview_id, "settings-request-1")
            .expect("settings migration must succeed");
    let second =
        execute_settings_migration(&legacy, &target, &preview.preview_id, "settings-request-1")
            .expect("same request must replay");

    assert_eq!(first, second);
    let migrated: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&target.settings_path).expect("target settings must exist"),
    )
    .expect("target settings must be valid json");
    assert_eq!(migrated["schemaVersion"], 3);
    assert_eq!(migrated["libraryRoot"], r"D:\Reader Library");
    let receipt: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&first.receipt_path).expect("receipt must exist"))
            .expect("receipt must be valid json");
    assert_eq!(receipt["status"], "success");
    assert!(receipt["nonSensitiveHashes"]["sourceSettingsSha256"].is_string());
    let control = ControlDb::open(&target.data_root.join(r"App\control.db"))
        .expect("control database must open");
    assert_eq!(
        control
            .migration_run(&first.migration_id)
            .expect("migration run must load")
            .expect("migration run must exist")
            .status,
        "success"
    );
    drop(control);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn settings_migration_rejects_a_stale_preview_without_writing_target() {
    let (root, legacy, target) = fixture("stale");
    let preview =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("preview must succeed");
    fs::write(
        &legacy.settings,
        r#"{"schemaVersion":2,"libraryRoot":"E:\\Changed"}"#,
    )
    .expect("legacy settings must change");

    let error = execute_settings_migration(
        &legacy,
        &target,
        &preview.preview_id,
        "settings-request-stale",
    )
    .expect_err("stale preview must fail");
    let replayed = execute_settings_migration(
        &legacy,
        &target,
        &preview.preview_id,
        "settings-request-stale",
    )
    .expect_err("same failed request must replay the failure");

    assert_eq!(error, "MIGRATION_PREVIEW_STALE");
    assert_eq!(replayed, error);
    assert!(!target.settings_path.exists());
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn settings_migration_replays_idempotently_on_a_migrated_install() {
    // P2-14: a used install already has a target settings.json, so the next
    // preview reports a conflict. When the target already holds exactly what
    // the migration would write, re-executing must succeed idempotently
    // instead of deadlocking on MIGRATION_CONFLICT forever.
    let (root, legacy, target) = fixture("replay-migrated");
    let preview =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("preview must succeed");
    execute_settings_migration(&legacy, &target, &preview.preview_id, "settings-request-a")
        .expect("first migration must succeed");

    let fresh =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("re-preview must succeed");
    assert_ne!(
        fresh.preview_id, preview.preview_id,
        "the conflict flag must change the preview identity"
    );
    assert!(fresh.conflict_count > 0);

    let second = execute_settings_migration(
        &legacy,
        &target,
        &fresh.preview_id,
        "settings-request-b",
    )
    .expect("re-run on an already-migrated install must succeed");
    assert_eq!(second.status, "success");
    let migrated: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&target.settings_path).expect("target settings must exist"),
    )
    .expect("target settings must be valid json");
    assert_eq!(migrated["schemaVersion"], 3);
    assert_eq!(migrated["libraryRoot"], r"D:\Reader Library");
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn settings_migration_still_rejects_a_genuine_conflict() {
    // The idempotent-replay gate must not weaken the real conflict case: a
    // target holding different content is not the migration result.
    let (root, legacy, target) = fixture("genuine-conflict");
    fs::create_dir_all(target.settings_path.parent().unwrap()).expect("target dir must exist");
    fs::write(
        &target.settings_path,
        r#"{"schemaVersion":3,"libraryRoot":"E:\\Other Library"}"#,
    )
    .expect("conflicting target must write");
    let fresh =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("preview must succeed");
    assert!(fresh.conflict_count > 0);

    let error = execute_settings_migration(
        &legacy,
        &target,
        &fresh.preview_id,
        "settings-request-conflict",
    )
    .expect_err("a genuinely different target must stay a conflict");

    assert_eq!(error, "MIGRATION_CONFLICT");
    assert_eq!(
        fs::read_to_string(&target.settings_path).expect("target must be untouched"),
        r#"{"schemaVersion":3,"libraryRoot":"E:\\Other Library"}"#
    );
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn internal_failures_still_settle_the_claim_for_replay() {
    // P2-14: a `?` after claim_command used to leak the claim, making every
    // retry replay COMMAND_IN_PROGRESS. The claim must now complete with the
    // failure so a replay returns the same error code deterministically.
    let (root, legacy, target) = fixture("claim-settled");
    // Unreadable source content fails `source_schema` — several `?`s after the
    // claim and past every early validation gate.
    fs::write(&legacy.settings, "not json").expect("broken legacy settings must write");
    let preview =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("preview must succeed");

    let first = execute_settings_migration(
        &legacy,
        &target,
        &preview.preview_id,
        "settings-request-fail",
    )
    .expect_err("unreadable source must fail");
    let replayed = execute_settings_migration(
        &legacy,
        &target,
        &preview.preview_id,
        "settings-request-fail",
    )
    .expect_err("the failed claim must replay the same failure");

    assert_eq!(first, "MIGRATION_FAILED");
    assert_eq!(replayed, first);
    fs::remove_dir_all(root).expect("fixture must be removed");
}

#[test]
fn settings_preview_detects_same_size_content_changes() {
    let (root, legacy, target) = fixture("same-size-stale");
    let preview =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("preview must succeed");
    fs::write(
        &legacy.settings,
        r#"{"schemaVersion":2,"libraryRoot":"E:\\Reader Library"}"#,
    )
    .expect("same-size settings must change");

    let changed =
        preview_for(&legacy, &target, MigrationScope::Settings).expect("preview must refresh");

    assert_ne!(changed.preview_id, preview.preview_id);
    fs::remove_dir_all(root).expect("fixture must be removed");
}
