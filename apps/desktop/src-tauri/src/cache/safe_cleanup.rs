use super::{read_podcast_recovery, validate_task_id};
use crate::storage::StorageLocations;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheCategory {
    PodcastCompleted,
    PodcastTask,
    ZhihuBrowserCache,
    GeneralTemporary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheSkip {
    pub category: String,
    pub task_id: Option<String>,
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheClearResult {
    pub deleted_items: u64,
    pub released_bytes: u64,
    pub skipped: Vec<CacheSkip>,
    pub protected_roots_verified: bool,
}

/// Bound the cleanup walk — a junction loop would otherwise recurse forever.
const MAX_CLEANUP_WALK_DEPTH: usize = 64;

fn walk_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    walk_files_at(root, files, 0)
}

fn walk_files_at(root: &Path, files: &mut Vec<PathBuf>, depth: usize) -> Result<(), String> {
    if depth > MAX_CLEANUP_WALK_DEPTH {
        return Ok(());
    }
    if !root.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    // Junctions are reparse points, not symlinks — push the link itself so
    // the caller unlinks it instead of enumerating the target's children
    // (which would resolve outside the cache root).
    if metadata.file_type().is_symlink()
        || crate::atomic_file::is_reparse_point(&metadata)
        || metadata.is_file()
    {
        files.push(root.to_path_buf());
        return Ok(());
    }
    let mut entries = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        walk_files_at(&entry.path(), files, depth + 1)?;
    }
    Ok(())
}

fn managed_cache_path(locations: &StorageLocations, path: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(&locations.cache_root).map_err(|error| error.to_string())?;
    let cache_root = fs::canonicalize(&locations.cache_root).map_err(|error| error.to_string())?;
    if !path.exists() {
        return Ok(path.to_path_buf());
    }
    let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if canonical == cache_root || !canonical.starts_with(&cache_root) {
        return Err("Cache path is outside the managed Cache root".to_string());
    }
    Ok(canonical)
}

fn tree_metrics(path: &Path) -> Result<(u64, u64), String> {
    let mut files = Vec::new();
    walk_files(path, &mut files)?;
    let mut bytes = 0_u64;
    for file in &files {
        bytes = bytes.saturating_add(
            fs::symlink_metadata(file)
                .map_err(|error| error.to_string())?
                .len(),
        );
    }
    Ok((files.len() as u64 + u64::from(path.exists()), bytes))
}

fn remove_managed_path(locations: &StorageLocations, path: &Path) -> Result<(u64, u64), String> {
    let managed = managed_cache_path(locations, path)?;
    if !managed.exists() {
        return Ok((0, 0));
    }
    let metrics = tree_metrics(&managed)?;
    let metadata = fs::symlink_metadata(&managed).map_err(|error| error.to_string())?;
    if metadata.is_dir()
        && !metadata.file_type().is_symlink()
        && !crate::atomic_file::is_reparse_point(&metadata)
    {
        fs::remove_dir_all(managed).map_err(|error| error.to_string())?;
    } else if metadata.is_dir() {
        // A junction/mount point: unlink the link itself — never recurse.
        fs::remove_dir(managed).map_err(|error| error.to_string())?;
    } else {
        fs::remove_file(managed).map_err(|error| error.to_string())?;
    }
    Ok(metrics)
}

pub fn discard_podcast_task_at(
    locations: &StorageLocations,
    task_id: &str,
) -> Result<(u64, u64), String> {
    validate_task_id(task_id)?;
    let cache_path = locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    let metrics = remove_managed_path(locations, &cache_path)?;
    // Drop the whole per-task data dir — task.json/recovery.json are dead
    // weight once the task is discarded, and a lingering task.json contract
    // read like evidence of life to anything scanning `Data\Podcast\Tasks`.
    let task_data_dir = locations
        .data_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    if task_data_dir.exists() {
        let data_root =
            fs::canonicalize(&locations.data_root).map_err(|error| error.to_string())?;
        let canonical_dir = fs::canonicalize(&task_data_dir).map_err(|error| error.to_string())?;
        if canonical_dir == data_root || !canonical_dir.starts_with(&data_root) {
            return Err("Podcast task data path is outside the managed Data root".to_string());
        }
        let metadata = fs::symlink_metadata(&canonical_dir).map_err(|error| error.to_string())?;
        if metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && !crate::atomic_file::is_reparse_point(&metadata)
        {
            fs::remove_dir_all(&canonical_dir).map_err(|error| error.to_string())?;
        } else if metadata.is_dir() {
            // A junction/mount point: unlink the link itself — never recurse.
            fs::remove_dir(&canonical_dir).map_err(|error| error.to_string())?;
        } else {
            fs::remove_file(&canonical_dir).map_err(|error| error.to_string())?;
        }
    }
    Ok(metrics)
}

fn cleanup_podcast_task(
    locations: &StorageLocations,
    task_id: &str,
    category: &str,
    result: &mut CacheClearResult,
) -> Result<(), String> {
    validate_task_id(task_id)?;
    let path = locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    match read_podcast_recovery(locations, task_id) {
        Ok(Some(recovery)) if recovery.lease_held => {
            result.skipped.push(CacheSkip {
                category: category.to_string(),
                task_id: Some(task_id.to_string()),
                path: path.to_string_lossy().into_owned(),
                reason: format!("cache lease held: {}", recovery.lease_reason),
            });
            return Ok(());
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            result.skipped.push(CacheSkip {
                category: category.to_string(),
                task_id: Some(task_id.to_string()),
                path: path.to_string_lossy().into_owned(),
                reason: "recovery metadata is missing".to_string(),
            });
            return Ok(());
        }
        Err(error) => {
            result.skipped.push(CacheSkip {
                category: category.to_string(),
                task_id: Some(task_id.to_string()),
                path: path.to_string_lossy().into_owned(),
                reason: format!("recovery metadata is invalid: {error}"),
            });
            return Ok(());
        }
    }
    let (items, bytes) = remove_managed_path(locations, &path)?;
    result.deleted_items = result.deleted_items.saturating_add(items);
    result.released_bytes = result.released_bytes.saturating_add(bytes);
    Ok(())
}

fn podcast_task_ids(locations: &StorageLocations) -> Result<Vec<String>, String> {
    let root = locations.cache_root.join(r"Podcast\Tasks");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut ids = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

pub fn clear_safe_cache_at(
    locations: &StorageLocations,
    categories: &[CacheCategory],
    task_ids: &[String],
) -> Result<CacheClearResult, String> {
    let mut result = CacheClearResult {
        deleted_items: 0,
        released_bytes: 0,
        skipped: Vec::new(),
        protected_roots_verified: false,
    };
    for category in categories {
        match category {
            CacheCategory::PodcastCompleted => {
                for task_id in podcast_task_ids(locations)? {
                    cleanup_podcast_task(locations, &task_id, "podcast_completed", &mut result)?;
                }
            }
            CacheCategory::PodcastTask => {
                for task_id in task_ids {
                    cleanup_podcast_task(locations, task_id, "podcast_task", &mut result)?;
                }
            }
            CacheCategory::ZhihuBrowserCache => {
                let (items, bytes) = remove_managed_path(
                    locations,
                    &locations.cache_root.join(r"Zhihu\BrowserCache"),
                )?;
                result.deleted_items = result.deleted_items.saturating_add(items);
                result.released_bytes = result.released_bytes.saturating_add(bytes);
            }
            CacheCategory::GeneralTemporary => {
                let (items, bytes) =
                    remove_managed_path(locations, &locations.cache_root.join("GeneralTemporary"))?;
                result.deleted_items = result.deleted_items.saturating_add(items);
                result.released_bytes = result.released_bytes.saturating_add(bytes);
                // Android: SAF pick staging lives in the private app cache
                // (`import-*`/`mobile-import-*`/`临时`), outside `Cache\` —
                // a stalled mobile import must still be cleanable here.
                #[cfg(target_os = "android")]
                if let Ok(base) = crate::storage::android_base_dirs() {
                    for dir in crate::temporary_content::android_staging_dirs(&base.app_cache) {
                        // Containment guard before any removal: only trees
                        // that still resolve inside the app cache go away.
                        if !crate::storage::path_within(&base.app_cache, &dir) {
                            continue;
                        }
                        if let Ok((items, bytes)) = tree_metrics(&dir) {
                            if fs::remove_dir_all(&dir).is_ok() {
                                result.deleted_items = result.deleted_items.saturating_add(items);
                                result.released_bytes = result.released_bytes.saturating_add(bytes);
                            }
                        }
                    }
                }
            }
        }
    }
    result.protected_roots_verified = true;
    Ok(result)
}
