use crate::cache::{acquire_podcast_cache_lease, validate_task_id};
use crate::cache::{
    discard_podcast_task_at, set_podcast_recovery_compatibility, PodcastCompatibility,
};
use crate::storage::StorageLocations;
use crate::tasks::TaskSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

mod tasks;
pub use tasks::*;
mod publish;
mod task_contract;
mod task_request;
mod worker;
pub use publish::publish_task_result_at;
pub use task_request::{
    AddPodcastFilesRequest, DuplicatePolicy, PodcastAddResult, PodcastBudgetApproval,
    PodcastPreviewStore,
};
pub use worker::{cancel_task, pause_task, resume_task, start_task, stop_all as stop_workers};

const ESTIMATE_VERSION: &str = "podcast-budget-v1-deepseek-v4-2026-07-12";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastPreviewOptions {
    pub translate: bool,
    /// When missing (schema v1 recovery / older clients), default to true.
    #[serde(default = "default_true")]
    pub polish: bool,
    pub max_api_cost_cny: f64,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastFilePreview {
    pub path: String,
    pub file_name: String,
    pub bytes: u64,
    pub duration_seconds: f64,
    pub input_sha256: String,
    pub source_id: String,
    pub book_id: String,
    pub duplicate_book_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastBudgetPreview {
    pub estimated_disk_bytes: u64,
    pub estimated_translation_tokens: u64,
    pub estimated_api_cost_upper_cny: f64,
    pub available_disk_bytes: u64,
    pub estimate_version: String,
    pub confirmation_required: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastFilesPreview {
    pub preview_id: String,
    pub files: Vec<PodcastFilePreview>,
    pub budget: PodcastBudgetPreview,
}

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

fn probe_duration(ffprobe: &Path, source: &Path) -> Result<f64, String> {
    if !ffprobe.is_file() {
        return Err("RUNTIME_UNAVAILABLE".to_string());
    }
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(ffprobe);
    command.args([
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ]);
    command.arg(source);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = command
        .output()
        .map_err(|_| "RUNTIME_UNAVAILABLE".to_string())?;
    if !output.status.success() {
        return Err("INVALID_ARGUMENT".to_string());
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "INVALID_ARGUMENT".to_string())?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err("INVALID_ARGUMENT".to_string());
    }
    Ok(duration)
}

#[cfg(windows)]
fn available_space(path: &Path) -> Result<u64, String> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let existing = path
        .ancestors()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| "INSUFFICIENT_DISK".to_string())?;
    let wide = existing
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    // SAFETY: wide is a live NUL-terminated path buffer and available is a
    // valid out-pointer; the two unused optional output pointers are null.
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if succeeded == 0 {
        return Err(format!(
            "Disk space lookup failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(available)
}

#[cfg(not(windows))]
fn available_space(_path: &Path) -> Result<u64, String> {
    Err("Disk space lookup is unsupported".to_string())
}

fn estimate_budget(
    files: &[PodcastFilePreview],
    translate: bool,
    max_api_cost_cny: f64,
    available_disk_bytes: u64,
) -> PodcastBudgetPreview {
    let duration = files.iter().map(|file| file.duration_seconds).sum::<f64>();
    let source_bytes = files.iter().map(|file| file.bytes).sum::<u64>();
    let normalized_and_chunks = (duration * 32_000.0 * 2.0).ceil() as u64;
    let estimated_disk_bytes = source_bytes
        .saturating_add(normalized_and_chunks)
        .saturating_add(256 * 1024 * 1024);
    let estimated_translation_tokens = if translate {
        (duration * 12.0).ceil() as u64
    } else {
        0
    };
    let estimated_api_cost_upper_cny = if translate {
        estimated_translation_tokens as f64 * 6.0 / 1_000_000.0
    } else {
        0.0
    };
    PodcastBudgetPreview {
        estimated_disk_bytes,
        estimated_translation_tokens,
        estimated_api_cost_upper_cny,
        available_disk_bytes,
        estimate_version: ESTIMATE_VERSION.to_string(),
        confirmation_required: estimated_disk_bytes > available_disk_bytes
            || estimated_api_cost_upper_cny > max_api_cost_cny,
    }
}

pub fn preview_podcast_files_at(
    paths: &[String],
    options: &PodcastPreviewOptions,
    locations: &StorageLocations,
) -> Result<PodcastFilesPreview, String> {
    if paths.is_empty() || !options.max_api_cost_cny.is_finite() || options.max_api_cost_cny < 0.0 {
        return Err("INVALID_ARGUMENT".to_string());
    }
    let ffprobe = locations.runtime_root.join(r"podcast\ffmpeg\ffprobe.exe");
    let mut files = Vec::new();
    for raw in paths {
        let path = PathBuf::from(raw);
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "mp3" | "m4a" | "wav") {
            return Err("INVALID_ARGUMENT".to_string());
        }
        let metadata = fs::metadata(&path).map_err(|_| "INPUT_CHANGED".to_string())?;
        if !metadata.is_file() {
            return Err("INVALID_ARGUMENT".to_string());
        }
        let input_sha256 = crate::publish::hash_file(&path)?;
        let book_id = format!("podcast:{input_sha256}");
        let duplicate_book_id =
            crate::library::find_book_by_source_id(&locations.library_root, &input_sha256)?
                .map(|manifest| manifest.book_id);
        files.push(PodcastFilePreview {
            path: raw.clone(),
            file_name: path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .ok_or_else(|| "INVALID_ARGUMENT".to_string())?,
            bytes: metadata.len(),
            duration_seconds: probe_duration(&ffprobe, &path)?,
            source_id: input_sha256.clone(),
            book_id,
            input_sha256,
            duplicate_book_id,
        });
    }
    let budget = estimate_budget(
        &files,
        options.translate,
        options.max_api_cost_cny,
        available_space(&locations.cache_root)?,
    );
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&files).map_err(|error| error.to_string())?);
    hasher.update([u8::from(options.translate)]);
    hasher.update([u8::from(options.polish)]);
    hasher.update(options.max_api_cost_cny.to_le_bytes());
    hasher.update(ESTIMATE_VERSION.as_bytes());
    Ok(PodcastFilesPreview {
        preview_id: format!("{:x}", hasher.finalize()),
        files,
        budget,
    })
}

pub fn copy_verified_input(
    source: &Path,
    locations: &StorageLocations,
    task_id: &str,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<VerifiedPodcastInput, String> {
    copy_verified_input_with_progress(
        source,
        locations,
        task_id,
        expected_sha256,
        expected_bytes,
        None,
    )
}

/// Copy source audio into the managed task cache with optional progress callbacks.
/// Progress is throttled to at most once per 250ms or every +1% of total bytes.
pub fn copy_verified_input_with_progress(
    source: &Path,
    locations: &StorageLocations,
    task_id: &str,
    expected_sha256: &str,
    expected_bytes: u64,
    mut on_progress: Option<&mut dyn FnMut(u64, u64)>,
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
        let mut last_report = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_millis(250))
            .unwrap_or_else(std::time::Instant::now);
        let mut last_percent = 0_u64;
        if let Some(callback) = on_progress.as_mut() {
            callback(0, expected_bytes);
        }
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
            if let Some(callback) = on_progress.as_mut() {
                let percent = if expected_bytes == 0 {
                    100
                } else {
                    ((copied_bytes.saturating_mul(100)) / expected_bytes).min(100)
                };
                if last_report.elapsed() >= std::time::Duration::from_millis(250)
                    || percent >= last_percent.saturating_add(1)
                    || copied_bytes >= expected_bytes
                {
                    callback(copied_bytes, expected_bytes);
                    last_report = std::time::Instant::now();
                    last_percent = percent;
                }
            }
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
        if let Some(callback) = on_progress.as_mut() {
            callback(copied_bytes, expected_bytes);
        }
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

fn task_has_markdown_artifacts(locations: &StorageLocations, task_id: &str) -> bool {
    let cache_root = locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    for relative in [
        "output",
        "work/internal/markdown_bilingual",
        "work/internal/markdown_raw",
    ] {
        let dir = cache_root.join(relative);
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_md = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| matches!(value.to_ascii_lowercase().as_str(), "md" | "markdown"))
                    .unwrap_or(false);
                if is_md && path.is_file() {
                    return true;
                }
            }
        }
    }
    false
}

/// User-facing retry: prefer re-publish of existing markdown (fast, no re-transcribe);
/// otherwise clone a fresh task revision for a full re-run.
pub fn retry_task_at(
    control: &mut crate::control::ControlDb,
    locations: &StorageLocations,
    task_id: &str,
) -> Result<(TaskSnapshot, RetryKind), String> {
    validate_task_id(task_id)?;
    let snapshot = control
        .task_snapshot(task_id)?
        .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
    if !snapshot.can_retry
        || !matches!(
            snapshot.lifecycle_state,
            crate::tasks::LifecycleState::Terminal
        )
    {
        return Err("TASK_NOT_RETRYABLE".to_string());
    }

    // Fast path: publish already-produced transcript without re-running Whisper.
    let publish_failed = matches!(
        snapshot.error_code,
        Some(crate::tasks::TaskErrorCode::PublishFailed)
            | Some(crate::tasks::TaskErrorCode::PublishRecoveryRequired)
    ) || snapshot
        .error_message
        .as_deref()
        .is_some_and(|message| message.to_ascii_uppercase().contains("PUBLISH"));
    if (publish_failed || task_has_markdown_artifacts(locations, task_id))
        && locations
            .data_root
            .join("Podcast")
            .join("Tasks")
            .join(task_id)
            .join("task.json")
            .is_file()
    {
        match publish_task_result_at(control, locations, task_id) {
            Ok(transaction) => {
                if !matches!(transaction.phase, crate::publish::PublishPhase::Committed) {
                    return Err(format!(
                        "PUBLISH_FAILED: re-publish ended in {:?}",
                        transaction.phase
                    ));
                }
                let event = control
                    .mark_terminal_task_success(
                        task_id,
                        "已保存到 书库/播客",
                        Some(transaction.book_id),
                    )?
                    .ok_or_else(|| "TASK_ALREADY_SUCCEEDED".to_string())?;
                return Ok((event.snapshot, RetryKind::Republished));
            }
            Err(error) => {
                // If there is no markdown left, fall through to full restart.
                if !error.contains("no Markdown") && !error.contains("output directory is missing")
                {
                    return Err(error);
                }
            }
        }
    }

    let restarted = restart_incompatible_task_at(control, locations, task_id)?;
    Ok((restarted, RetryKind::Restarted))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryKind {
    Republished,
    Restarted,
}

pub fn restart_incompatible_task_at(
    control: &mut crate::control::ControlDb,
    locations: &StorageLocations,
    task_id: &str,
) -> Result<TaskSnapshot, String> {
    validate_task_id(task_id)?;
    let snapshot = control
        .task_snapshot(task_id)?
        .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
    if !snapshot.can_retry
        || !matches!(
            snapshot.lifecycle_state,
            crate::tasks::LifecycleState::Terminal
        )
    {
        return Err("TASK_NOT_RETRYABLE".to_string());
    }
    let old_task_root = locations
        .data_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    let old_spec_path = old_task_root.join("task.json");
    if !old_spec_path.is_file() {
        return Err(
            "TASK_CONTRACT_MISSING: 任务合同已丢失，请重新添加原音频文件。".to_string(),
        );
    }
    let spec: Value = serde_json::from_str(
        &fs::read_to_string(&old_spec_path).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let input = spec
        .get("input")
        .and_then(Value::as_object)
        .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?;
    let relative_path = input
        .get("relativePath")
        .and_then(Value::as_str)
        .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?;
    let input_sha256 = input
        .get("inputSha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?;
    let bytes = input
        .get("bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?;
    let old_cache_root = locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    let relative = Path::new(relative_path);
    // Accept both "input/foo.m4a" and nested Normal components (Unicode filenames).
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    let old_input = old_cache_root.join(relative);
    if !old_input.is_file() {
        return Err(
            "INPUT_MISSING: 缓存中的原音频已不存在，请重新选择文件添加。".to_string(),
        );
    }
    let new_task_id = uuid::Uuid::new_v4().simple().to_string();
    let publish = spec
        .get("publish")
        .and_then(Value::as_object)
        .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?;
    let revision = publish
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?
        .checked_add(1)
        .ok_or_else(|| "INVALID_TASK_REVISION".to_string())?;
    let compatibility_obj = spec
        .get("compatibility")
        .and_then(Value::as_object)
        .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?;
    let compatibility = PodcastCompatibility {
        input_sha256: input_sha256.to_string(),
        pipeline_version: compatibility_obj
            .get("pipelineVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?
            .to_string(),
        engine_version: compatibility_obj
            .get("engineVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?
            .to_string(),
        config_hash: compatibility_obj
            .get("configHash")
            .and_then(Value::as_str)
            .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?
            .to_string(),
        model_hash: compatibility_obj
            .get("modelHash")
            .and_then(Value::as_str)
            .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?
            .to_string(),
    };
    let result = (|| {
        let verified =
            copy_verified_input(&old_input, locations, &new_task_id, input_sha256, bytes)?;
        let mut new_spec = spec.clone();
        new_spec["taskId"] = Value::String(new_task_id.clone());
        if let Some(input_obj) = new_spec.get_mut("input") {
            input_obj["relativePath"] = Value::String(verified.relative_path);
        }
        if let Some(publish_obj) = new_spec.get_mut("publish") {
            publish_obj["revision"] = json!(revision);
            publish_obj["incomingRelativePath"] =
                Value::String(format!(".incoming/{new_task_id}"));
        }
        let new_task_root = locations
            .data_root
            .join("Podcast")
            .join("Tasks")
            .join(&new_task_id);
        fs::create_dir_all(&new_task_root).map_err(|error| error.to_string())?;
        let data = serde_json::to_vec_pretty(&new_spec).map_err(|error| error.to_string())?;
        crate::atomic_file::write(&new_task_root.join("task.json"), &data)?;
        set_podcast_recovery_compatibility(locations, &new_task_id, compatibility)?;
        let file_name = old_input
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .ok_or_else(|| "INVALID_TASK_SPEC".to_string())?;
        let file = PodcastFilePreview {
            path: old_input.to_string_lossy().into_owned(),
            file_name,
            bytes,
            duration_seconds: input
                .get("durationSeconds")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            input_sha256: input_sha256.to_string(),
            source_id: snapshot.source_id.clone().unwrap_or_default(),
            book_id: snapshot.book_id.clone().unwrap_or_default(),
            duplicate_book_id: None,
        };
        let event = tasks::queued_event(new_task_id.clone(), &file, bytes);
        control.persist_task_event(&event)?;
        Ok(event.snapshot)
    })();
    if result.is_err() {
        let _ = discard_podcast_task_at(locations, &new_task_id);
        let new_task_root = locations
            .data_root
            .join("Podcast")
            .join("Tasks")
            .join(&new_task_id);
        let _ = fs::remove_dir_all(new_task_root);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        copy_verified_input, preview_podcast_files_at, restart_incompatible_task_at, retry_task_at,
        PodcastPreviewOptions, RetryKind,
    };
    use crate::cache::read_podcast_recovery;
    use crate::control::ControlDb;
    use crate::storage::StorageLocations;
    use crate::tasks::{
        LifecycleState, ProgressMode, RequiredAction, TaskErrorCode, TaskEvent, TaskKind,
        TaskOutcome, TaskProgress, TaskSnapshot,
    };
    use serde_json::json;
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
            runtime_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(r"..\..\..\runtime"),
        };
        (root, locations)
    }

    fn write_one_second_wav(path: &std::path::Path) {
        let data_size = 16_000_u32 * 2;
        let mut bytes = Vec::with_capacity(44 + data_size as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&16_000_u32.to_le_bytes());
        bytes.extend_from_slice(&32_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        bytes.resize(44 + data_size as usize, 0);
        fs::write(path, bytes).expect("wav fixture must write");
    }

    #[test]
    fn preview_is_read_only_deterministic_and_requires_zero_budget_confirmation() {
        let (root, locations) = fixture("preview");
        let source = root.join("sample.wav");
        write_one_second_wav(&source);
        let options = PodcastPreviewOptions {
            translate: true,
            polish: true,
            max_api_cost_cny: 0.0,
        };

        let first = preview_podcast_files_at(
            &[source.to_string_lossy().into_owned()],
            &options,
            &locations,
        )
        .expect("preview must succeed");
        let second = preview_podcast_files_at(
            &[source.to_string_lossy().into_owned()],
            &options,
            &locations,
        )
        .expect("repeated preview must succeed");

        assert_eq!(first.preview_id, second.preview_id);
        assert!((first.files[0].duration_seconds - 1.0).abs() < 0.01);
        assert_eq!(first.files[0].source_id, first.files[0].input_sha256);
        assert!(first.budget.estimated_disk_bytes > first.files[0].bytes);
        assert!(first.budget.estimated_translation_tokens > 0);
        assert!(first.budget.estimated_api_cost_upper_cny > 0.0);
        assert!(first.budget.confirmation_required);
        assert!(!locations.data_root.exists());
        assert!(!locations.cache_root.exists());
        fs::remove_dir_all(root).expect("fixture must be removed");
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

    #[test]
    fn incompatible_restart_creates_fresh_cache_and_next_revision() {
        let (root, locations) = fixture("restart");
        let old_task_root = locations
            .data_root
            .join("Podcast")
            .join("Tasks")
            .join("task-old");
        let old_cache_root = locations
            .cache_root
            .join("Podcast")
            .join("Tasks")
            .join("task-old");
        fs::create_dir_all(&old_task_root).expect("old task root must exist");
        fs::create_dir_all(old_cache_root.join("input")).expect("old input root must exist");
        fs::create_dir_all(old_cache_root.join("chunks")).expect("old chunks root must exist");
        let input = b"restart-audio";
        fs::write(old_cache_root.join("input").join("sample.mp3"), input)
            .expect("old input must write");
        fs::write(old_cache_root.join("chunks").join("old.bin"), b"old chunk")
            .expect("old chunk must write");
        let input_sha256 = format!("{:x}", Sha256::digest(input));
        fs::write(
            old_task_root.join("task.json"),
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": 1,
                "taskId": "task-old",
                "input": {"relativePath": "input/sample.mp3", "inputSha256": input_sha256, "bytes": input.len(), "durationSeconds": 2.0},
                "compatibility": {"pipelineVersion": "pipeline-1", "engineVersion": "engine-1", "configHash": "config-1", "modelHash": "model-1"},
                "publish": {"bookId": "podcast:book", "sourceId": "source", "revision": 1, "incomingRelativePath": ".incoming/task-old"}
            }))
            .expect("task spec must serialize"),
        )
        .expect("task spec must write");
        let now = chrono::Utc::now().to_rfc3339();
        let snapshot = TaskSnapshot {
            id: "task-old".to_string(),
            kind: TaskKind::Podcast,
            revision: 1,
            last_sequence: 1,
            lifecycle_state: LifecycleState::Terminal,
            outcome: TaskOutcome::Failed,
            required_action: RequiredAction::None,
            progress: TaskProgress {
                mode: ProgressMode::Determinate,
                percent: Some(12.0),
                completed_units: None,
                total_units: None,
                label: None,
                unit: None,
                source_total_units: None,
                skipped_units: None,
            },
            error_code: Some(TaskErrorCode::PipelineIncompatible),
            error_message: Some("incompatible".to_string()),
            retry_after_seconds: None,
            engine_stage: "recovery_check".to_string(),
            engine_status: "exited".to_string(),
            recoverable: true,
            can_pause: false,
            can_resume: false,
            can_retry: true,
            can_cancel: false,
            book_id: Some("podcast:book".to_string()),
            source_id: Some("source".to_string()),
            display_name: None,
            cache_lease_bytes: input.len() as u64,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_heartbeat_at: None,
            checkpoint_at: None,
        };
        let event = TaskEvent {
            schema_version: 1,
            task_id: snapshot.id.clone(),
            sequence: 1,
            revision: 1,
            event_type: "worker_failed".to_string(),
            snapshot,
            created_at: now,
        };
        let mut control =
            ControlDb::open(&root.join("control.db")).expect("control database must open");
        control
            .persist_task_event(&event)
            .expect("failed task must persist");

        let restarted = restart_incompatible_task_at(&mut control, &locations, "task-old")
            .expect("restart must create a new revision");
        assert_ne!(restarted.id, "task-old");
        assert_eq!(restarted.revision, 1);
        assert_eq!(restarted.lifecycle_state, LifecycleState::Queued);
        let new_root = locations
            .cache_root
            .join("Podcast")
            .join("Tasks")
            .join(&restarted.id);
        assert!(new_root.join("input").join("sample.mp3").is_file());
        assert!(!new_root.join("chunks").join("old.bin").exists());
        let new_spec: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(
                locations
                    .data_root
                    .join("Podcast")
                    .join("Tasks")
                    .join(&restarted.id)
                    .join("task.json"),
            )
            .expect("new task spec must exist"),
        )
        .expect("new task spec must parse");
        assert_eq!(new_spec["publish"]["revision"], 2);
        assert_eq!(new_spec["publish"]["bookId"], "podcast:book");
        assert_eq!(new_spec["publish"]["sourceId"], "source");
        drop(control);
        fs::remove_dir_all(root).expect("fixture must be removed");
    }

    #[test]
    fn retry_publish_failed_republishes_without_new_task() {
        let (root, locations) = fixture("retry-republish");
        let task_id = "task-retry";
        let source_id = "b".repeat(64);
        let book_id = format!("podcast:{source_id}");
        let task_root = locations
            .data_root
            .join("Podcast")
            .join("Tasks")
            .join(task_id);
        let cache_root = locations
            .cache_root
            .join("Podcast")
            .join("Tasks")
            .join(task_id);
        fs::create_dir_all(&task_root).expect("task root");
        fs::create_dir_all(cache_root.join("input")).expect("input root");
        let final_output = cache_root.join("output");
        fs::create_dir_all(&final_output).expect("final output root");
        fs::write(
            final_output.join("episode.md"),
            "# Huberman\n\nTranscript body",
        )
        .expect("final markdown");
        let input = b"retry-audio-bytes";
        fs::write(cache_root.join("input").join("episode.mp3"), input).expect("input");
        let input_sha256 = format!("{:x}", Sha256::digest(input));
        fs::write(
            task_root.join("task.json"),
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": 1,
                "taskId": task_id,
                "input": {
                    "relativePath": "input/episode.mp3",
                    "inputSha256": input_sha256,
                    "bytes": input.len(),
                    "durationSeconds": 1.0
                },
                "compatibility": {
                    "pipelineVersion": "pipeline-1",
                    "engineVersion": "engine-1",
                    "configHash": "config-1",
                    "modelHash": "model-1"
                },
                "publish": {
                    "bookId": book_id,
                    "sourceId": source_id,
                    "revision": 1,
                    "incomingRelativePath": format!(".incoming/{task_id}")
                }
            }))
            .expect("serialize"),
        )
        .expect("task.json");

        let now = chrono::Utc::now().to_rfc3339();
        let snapshot = TaskSnapshot {
            id: task_id.to_string(),
            kind: TaskKind::Podcast,
            revision: 1,
            last_sequence: 1,
            lifecycle_state: LifecycleState::Terminal,
            outcome: TaskOutcome::Failed,
            required_action: RequiredAction::None,
            progress: TaskProgress {
                mode: ProgressMode::Determinate,
                percent: Some(99.0),
                completed_units: None,
                total_units: None,
                label: Some("发布失败".to_string()),
                unit: None,
                source_total_units: None,
                skipped_units: None,
            },
            error_code: Some(TaskErrorCode::PublishFailed),
            error_message: Some(
                r#"{"errorCode":"PUBLISH_FAILED","message":"PUBLISH_FAILED: worker produced no Markdown output","type":"fatal"}"#.to_string(),
            ),
            retry_after_seconds: None,
            engine_stage: "failed".to_string(),
            engine_status: "exited".to_string(),
            recoverable: true,
            can_pause: false,
            can_resume: false,
            can_retry: true,
            can_cancel: false,
            book_id: Some(book_id.clone()),
            source_id: Some(source_id),
            display_name: None,
            cache_lease_bytes: 0,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_heartbeat_at: Some(now.clone()),
            checkpoint_at: None,
        };
        let event = TaskEvent {
            schema_version: 1,
            task_id: task_id.to_string(),
            sequence: 1,
            revision: 1,
            event_type: "worker_failed".to_string(),
            snapshot,
            created_at: now,
        };
        let mut control = ControlDb::open(&locations.data_root.join("App").join("control.db"))
            .expect("control");
        control.persist_task_event(&event).expect("persist failed task");

        let (result, kind) =
            retry_task_at(&mut control, &locations, task_id).expect("retry must republish");
        assert_eq!(kind, RetryKind::Republished);
        assert_eq!(result.id, task_id);
        assert_eq!(result.outcome, TaskOutcome::Success);
        assert_eq!(result.lifecycle_state, LifecycleState::Terminal);
        assert!(!result.can_retry);
        assert_eq!(result.book_id.as_deref(), Some(book_id.as_str()));
        assert!(locations
            .library_root
            .join("播客")
            .join("episode")
            .join("manifest.json")
            .is_file());

        drop(control);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
