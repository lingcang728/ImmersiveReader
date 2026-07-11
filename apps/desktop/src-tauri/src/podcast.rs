use crate::cache::{acquire_podcast_cache_lease, validate_task_id};
use crate::storage::StorageLocations;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedPodcastInput {
    pub relative_path: String,
    pub input_sha256: String,
    pub bytes: u64,
}

fn validate_sha256(value: &str) -> Result<String, String> {
    let normalized = value.to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("INPUT_CHANGED".to_string());
    }
    Ok(normalized)
}

fn task_cache_root(locations: &StorageLocations, task_id: &str) -> PathBuf {
    locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id)
}

pub fn copy_verified_input(
    source: &Path,
    locations: &StorageLocations,
    task_id: &str,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<VerifiedPodcastInput, String> {
    validate_task_id(task_id)?;
    let expected_sha256 = validate_sha256(expected_sha256)?;
    let before = fs::metadata(source).map_err(|_| "INPUT_CHANGED".to_string())?;
    if !before.is_file() || before.len() != expected_bytes {
        return Err("INPUT_CHANGED".to_string());
    }
    let file_name = source
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "INVALID_ARGUMENT".to_string())?;
    acquire_podcast_cache_lease(locations, task_id, "queued", expected_bytes)?;
    let task_root = task_cache_root(locations, task_id);
    let input_root = task_root.join("input");
    fs::create_dir_all(&input_root).map_err(|error| error.to_string())?;
    let partial = task_root.join("input.partial");
    if partial.exists() {
        fs::remove_file(&partial).map_err(|error| error.to_string())?;
    }
    let final_path = input_root.join(file_name);
    if final_path.exists() {
        return Err("CONFLICT".to_string());
    }
    let copied = (|| {
        let mut reader = fs::File::open(source).map_err(|error| error.to_string())?;
        let mut writer = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut copied_bytes = 0_u64;
        let mut buffer = [0_u8; 1024 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            writer
                .write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            hasher.update(&buffer[..read]);
            copied_bytes = copied_bytes.saturating_add(read as u64);
        }
        writer.sync_all().map_err(|error| error.to_string())?;
        let actual_sha256 = format!("{:x}", hasher.finalize());
        let after = fs::metadata(source).map_err(|_| "INPUT_CHANGED".to_string())?;
        if copied_bytes != expected_bytes
            || actual_sha256 != expected_sha256
            || after.len() != before.len()
            || after.modified().ok() != before.modified().ok()
        {
            return Err("INPUT_CHANGED".to_string());
        }
        fs::rename(&partial, &final_path).map_err(|error| error.to_string())?;
        Ok(VerifiedPodcastInput {
            relative_path: format!("input/{}", file_name.to_string_lossy()),
            input_sha256: actual_sha256,
            bytes: copied_bytes,
        })
    })();
    if copied.is_err() {
        let _ = fs::remove_file(&partial);
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::copy_verified_input;
    use crate::cache::read_podcast_recovery;
    use crate::storage::StorageLocations;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::PathBuf;

    fn fixture(name: &str) -> (PathBuf, StorageLocations) {
        let root = std::env::temp_dir().join(format!(
            "immersive-podcast-input-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture root must exist");
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

    #[test]
    fn verified_copy_promotes_partial_without_changing_source() {
        let (root, locations) = fixture("success");
        let source = root.join("sample.mp3");
        let bytes = b"immutable-audio";
        fs::write(&source, bytes).expect("source must write");
        let sha256 = format!("{:x}", Sha256::digest(bytes));

        let copied =
            copy_verified_input(&source, &locations, "task-1", &sha256, bytes.len() as u64)
                .expect("verified copy must succeed");

        assert_eq!(copied.relative_path, "input/sample.mp3");
        assert_eq!(fs::read(&source).expect("source must remain"), bytes);
        assert_eq!(
            fs::read(
                locations
                    .cache_root
                    .join(r"Podcast\Tasks\task-1\input\sample.mp3")
            )
            .expect("managed input must exist"),
            bytes
        );
        assert!(!locations
            .cache_root
            .join(r"Podcast\Tasks\task-1\input.partial")
            .exists());
        fs::remove_dir_all(root).expect("fixture must be removed");
    }

    #[test]
    fn hash_mismatch_never_promotes_partial_and_keeps_lease() {
        let (root, locations) = fixture("mismatch");
        let source = root.join("sample.m4a");
        fs::write(&source, b"audio").expect("source must write");

        let error = copy_verified_input(&source, &locations, "task-2", &"0".repeat(64), 5)
            .expect_err("hash mismatch must fail");

        assert_eq!(error, "INPUT_CHANGED");
        let task_root = locations.cache_root.join(r"Podcast\Tasks\task-2");
        assert!(!task_root.join("input.partial").exists());
        assert!(!task_root.join(r"input\sample.m4a").exists());
        assert!(
            read_podcast_recovery(&locations, "task-2")
                .expect("recovery metadata must load")
                .expect("recovery metadata must exist")
                .lease_held
        );
        fs::remove_dir_all(root).expect("fixture must be removed");
    }
}
