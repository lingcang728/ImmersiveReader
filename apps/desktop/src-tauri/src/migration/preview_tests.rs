use super::{preview_for, LegacyLocations, MigrationScope};
use crate::storage::StorageLocations;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    let path = std::env::temp_dir().join(format!("immersive-preview-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("test root must exist");
    path
}

#[test]
fn preview_is_read_only_deterministic_and_marks_sensitive_profile() {
    let root = root();
    let legacy = LegacyLocations {
        settings: root.join(r"Legacy\settings.json"),
        immersive_state: root.join(r"Legacy\immersive-state"),
        mmbook_state: root.join(r"Legacy\mmbook-state"),
        podcast_root: root.join(r"Legacy\podcast"),
        zhihu_root: root.join(r"Legacy\zhihu"),
        library_root: root.join(r"Legacy\Library"),
    };
    fs::create_dir_all(legacy.zhihu_root.join("browser-profile")).expect("profile must exist");
    fs::write(
        legacy.zhihu_root.join(r"browser-profile\Cookies"),
        b"private",
    )
    .expect("profile fixture must write");
    fs::create_dir_all(&legacy.podcast_root).expect("podcast root must exist");
    fs::write(legacy.podcast_root.join("config.json"), b"{}").expect("podcast fixture must write");
    fs::create_dir_all(&legacy.mmbook_state).expect("MMbook state must exist");
    fs::write(
        legacy.mmbook_state.join("recent-files.json"),
        br#"[{"path":"C:\\Books\\one.md"}]"#,
    )
    .expect("recent-files fixture must write");
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

    let first =
        preview_for(&legacy, &target, MigrationScope::All).expect("migration preview must succeed");
    let second =
        preview_for(&legacy, &target, MigrationScope::All).expect("repeated preview must succeed");

    assert_eq!(first.preview_id, second.preview_id);
    assert!(!target.data_root.exists());
    let profile = first
        .items
        .iter()
        .find(|item| item.kind == "zhihu_profile")
        .expect("profile must be listed");
    assert!(profile.sensitive);
    assert_eq!(profile.bytes, 7);
    let recent_files = first
        .items
        .iter()
        .find(|item| item.kind == "mmbook_recent_files")
        .expect("recent-files must be listed");
    assert!(recent_files.exists);
    assert!(!recent_files.sensitive);
    assert!(recent_files.target_path.ends_with(r"Target\Settings\recent-files.json"));
    fs::remove_dir_all(root).expect("fixture must be removed");
}
