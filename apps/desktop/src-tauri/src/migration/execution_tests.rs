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

    let first = execute_settings_migration(
        &legacy,
        &target,
        &preview.preview_id,
        "settings-request-1",
    )
    .expect("settings migration must succeed");
    let second = execute_settings_migration(
        &legacy,
        &target,
        &preview.preview_id,
        "settings-request-1",
    )
    .expect("same request must replay");

    assert_eq!(first, second);
    let migrated: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&target.settings_path).expect("target settings must exist"),
    )
    .expect("target settings must be valid json");
    assert_eq!(migrated["schemaVersion"], 3);
    assert_eq!(migrated["libraryRoot"], r"D:\Reader Library");
    let receipt: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&first.receipt_path).expect("receipt must exist"),
    )
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
