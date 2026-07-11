use crate::storage::StorageLocations;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

mod safe_cleanup;
pub use safe_cleanup::{clear_safe_cache_at, CacheCategory, CacheClearResult};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastRecovery {
    pub schema_version: u32,
    pub task_id: String,
    pub lease_held: bool,
    pub lease_reason: String,
    pub cache_relative_path: String,
    pub resumable: bool,
    pub last_compatible_checkpoint: Option<String>,
    pub bytes: u64,
    pub updated_at: String,
}

impl PodcastRecovery {
    fn new(task_id: &str, lease_held: bool, reason: &str, resumable: bool, bytes: u64) -> Self {
        Self {
            schema_version: 1,
            task_id: task_id.to_string(),
            lease_held,
            lease_reason: reason.to_string(),
            cache_relative_path: format!("Podcast/Tasks/{task_id}"),
            resumable,
            last_compatible_checkpoint: None,
            bytes,
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

pub fn acquire_podcast_cache_lease(
    locations: &StorageLocations,
    task_id: &str,
    reason: &str,
    bytes: u64,
) -> Result<PodcastRecovery, String> {
    validate_task_id(task_id)?;
    let recovery = PodcastRecovery::new(task_id, true, reason, true, bytes);
    write_podcast_recovery(locations, &recovery)?;
    Ok(recovery)
}

pub fn release_podcast_cache_lease(
    locations: &StorageLocations,
    task_id: &str,
    bytes: u64,
) -> Result<PodcastRecovery, String> {
    validate_task_id(task_id)?;
    let recovery = PodcastRecovery::new(task_id, false, "completed", false, bytes);
    write_podcast_recovery(locations, &recovery)?;
    Ok(recovery)
}

pub(crate) fn validate_task_id(task_id: &str) -> Result<(), String> {
    if task_id.is_empty()
        || !task_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
    {
        return Err("Podcast task id must contain only letters, digits, '-' or '_'".to_string());
    }
    Ok(())
}

pub(crate) fn recovery_path(locations: &StorageLocations, task_id: &str) -> PathBuf {
    locations
        .data_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id)
        .join("recovery.json")
}

pub fn write_podcast_recovery(
    locations: &StorageLocations,
    recovery: &PodcastRecovery,
) -> Result<(), String> {
    validate_task_id(&recovery.task_id)?;
    let expected_cache = format!("Podcast/Tasks/{}", recovery.task_id);
    if recovery.schema_version != 1 || recovery.cache_relative_path != expected_cache {
        return Err("Invalid Podcast recovery metadata".to_string());
    }
    let path = recovery_path(locations, &recovery.task_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(recovery).map_err(|error| error.to_string())?;
    crate::atomic_file::write(&path, &data)
}

pub(crate) fn read_podcast_recovery(
    locations: &StorageLocations,
    task_id: &str,
) -> Result<Option<PodcastRecovery>, String> {
    validate_task_id(task_id)?;
    let path = recovery_path(locations, task_id);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let recovery: PodcastRecovery =
        serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    if recovery.schema_version != 1 || recovery.task_id != task_id {
        return Err("Podcast recovery metadata does not match the task".to_string());
    }
    Ok(Some(recovery))
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_podcast_cache_lease, clear_safe_cache_at, release_podcast_cache_lease,
        CacheCategory,
    };
    use crate::storage::StorageLocations;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn test_locations(name: &str) -> (PathBuf, StorageLocations) {
        let root =
            std::env::temp_dir().join(format!("immersive-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let locations = StorageLocations {
            channel: "test".to_string(),
            settings_path: root.join(r"Settings\settings.json"),
            data_root: root.join("Data"),
            cache_root: root.join("Cache"),
            logs_root: root.join("Logs"),
            runtime_state_root: root.join("RuntimeState"),
            backups_root: root.join("Backups"),
            library_root: root.join("Library"),
            runtime_root: root.join("Runtime"),
        };
        (root, locations)
    }

    fn write_sentinel(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().expect("sentinel parent must exist"))
            .expect("sentinel parent must be created");
        fs::write(path, bytes).expect("sentinel must write");
    }

    #[test]
    fn safe_cleanup_skips_leased_tasks_and_preserves_protected_roots() {
        let (root, locations) = test_locations("leased");
        let leased_cache = locations.cache_root.join(r"Podcast\Tasks\leased-task");
        let completed_cache = locations.cache_root.join(r"Podcast\Tasks\completed-task");
        write_sentinel(&leased_cache.join(r"chunks\one.bin"), b"leased");
        write_sentinel(&completed_cache.join(r"chunks\two.bin"), b"completed");
        acquire_podcast_cache_lease(&locations, "leased-task", "paused", 6)
            .expect("lease metadata must persist");
        release_podcast_cache_lease(&locations, "completed-task", 9)
            .expect("released metadata must persist");
        let data_sentinel = locations.data_root.join("database.sentinel");
        let library_sentinel = locations.library_root.join("book.sentinel");
        let backup_sentinel = locations.backups_root.join("backup.sentinel");
        write_sentinel(&data_sentinel, b"data");
        write_sentinel(&library_sentinel, b"library");
        write_sentinel(&backup_sentinel, b"backup");

        let result = clear_safe_cache_at(&locations, &[CacheCategory::PodcastCompleted], &[])
            .expect("safe cleanup must succeed");

        assert!(leased_cache.exists());
        assert!(!completed_cache.exists());
        assert_eq!(fs::read(data_sentinel).expect("data must remain"), b"data");
        assert_eq!(
            fs::read(library_sentinel).expect("library must remain"),
            b"library"
        );
        assert_eq!(
            fs::read(backup_sentinel).expect("backup must remain"),
            b"backup"
        );
        assert!(result.protected_roots_verified);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].task_id.as_deref(), Some("leased-task"));
        fs::remove_dir_all(root).expect("fixture must be removed");
    }

    #[test]
    fn task_cleanup_rejects_path_traversal_identifiers() {
        let (root, locations) = test_locations("traversal");

        let error = clear_safe_cache_at(
            &locations,
            &[CacheCategory::PodcastTask],
            &[r"..\Data".to_string()],
        )
        .expect_err("unsafe task id must be rejected");

        assert!(error.contains("task id"));
        let _ = fs::remove_dir_all(root);
    }
}
