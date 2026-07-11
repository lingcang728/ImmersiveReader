use crate::contracts::{validate_reading, Manifest, ReadingProgress};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_marker() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn progress_path(book_root: &Path) -> PathBuf {
    book_root.join(".reading.json")
}

fn backup_corrupt(path: &Path) -> Result<(), String> {
    let backup = path.with_file_name(format!(".reading.{}.corrupt", now_marker()));
    fs::rename(path, backup).map_err(|error| error.to_string())
}

fn default_progress(manifest: &Manifest) -> ReadingProgress {
    let first = manifest
        .chapters
        .first()
        .map(|chapter| chapter.id.as_str())
        .unwrap_or("");
    ReadingProgress::empty(first)
}

fn resolve_current(mut progress: ReadingProgress, manifest: &Manifest) -> ReadingProgress {
    let current_exists = manifest
        .chapters
        .iter()
        .any(|chapter| chapter.id == progress.current);
    if current_exists {
        return progress;
    }
    let first_unread = manifest
        .chapters
        .iter()
        .find(|chapter| !progress.read.contains(&chapter.id))
        .or_else(|| manifest.chapters.first());
    progress.current = first_unread
        .map(|chapter| chapter.id.clone())
        .unwrap_or_default();
    progress.position = 0.0;
    progress
}

pub fn load_progress(book_root: &Path, manifest: &Manifest) -> Result<ReadingProgress, String> {
    let path = progress_path(book_root);
    if !path.exists() {
        return Ok(default_progress(manifest));
    }
    let raw = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let parsed = serde_json::from_str::<ReadingProgress>(&raw);
    let mut progress = match parsed {
        Ok(value) => value,
        Err(_) => {
            backup_corrupt(&path)?;
            return Ok(default_progress(manifest));
        }
    };
    progress = resolve_current(progress, manifest);
    if let Err(error) = validate_reading(&progress, manifest) {
        backup_corrupt(&path)?;
        return Err(format!("Invalid reading state was backed up: {error}"));
    }
    Ok(progress)
}

pub fn save_progress(
    book_root: &Path,
    manifest: &Manifest,
    progress: &ReadingProgress,
) -> Result<(), String> {
    validate_reading(progress, manifest)?;
    let data = serde_json::to_vec_pretty(progress).map_err(|error| error.to_string())?;
    crate::atomic_write_file(&progress_path(book_root), &data)
}

#[cfg(test)]
mod tests {
    use super::{load_progress, save_progress};
    use crate::contracts::{Manifest, ReadingProgress};
    use std::fs;
    use std::path::PathBuf;

    fn fixture_manifest() -> Manifest {
        serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize")
    }

    fn temp_book(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("immersive-reader-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp book must be created");
        dir
    }

    #[test]
    fn round_trips_progress_atomically() {
        let root = temp_book("progress-roundtrip");
        let manifest = fixture_manifest();
        let progress: ReadingProgress = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/reading.valid.json"
        ))
        .expect("fixture must deserialize");
        save_progress(&root, &manifest, &progress).expect("progress must save");
        let loaded = load_progress(&root, &manifest).expect("progress must load");
        assert_eq!(loaded.current, progress.current);
        assert_eq!(loaded.position, 0.5);
        fs::remove_dir_all(root).expect("temp book must be removed");
    }

    #[test]
    fn backs_up_corrupt_progress() {
        let root = temp_book("progress-corrupt");
        fs::write(root.join(".reading.json"), "not json").expect("fixture must write");
        let loaded = load_progress(&root, &fixture_manifest()).expect("fallback must load");
        assert_eq!(loaded.position, 0.0);
        assert!(!root.join(".reading.json").exists());
        let backups = fs::read_dir(&root)
            .expect("temp book must list")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".corrupt"))
            .count();
        assert_eq!(backups, 1);
        fs::remove_dir_all(root).expect("temp book must be removed");
    }
}
