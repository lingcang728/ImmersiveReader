use encoding_rs::{GB18030, UTF_16BE, UTF_16LE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(desktop)]
use tauri::menu::{MenuBuilder, MenuItem};
use tauri::{Emitter, Manager};
mod atomic_file;
pub mod cache;
mod contracts;
pub mod control;
mod importer;
#[cfg(windows)]
pub mod job_object;
mod library;
pub mod migration;
pub mod podcast;
mod progress;
pub mod publish;
mod reader_http;
mod reader_preferences;
mod reader_server;
mod secrets;
mod settings;
mod storage;
pub mod tasks;
mod temporary_content;
mod tools;
mod trash;
mod zhihu;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use tauri::RunEvent;

pub struct StandaloneReader {
    _state: reader_server::ReaderServiceState,
    url: String,
}

impl StandaloneReader {
    pub fn url(&self) -> &str {
        &self.url
    }
}

pub fn start_standalone_reader(book_id: &str) -> Result<StandaloneReader, String> {
    let state = reader_server::ReaderServiceState::default();
    let value = settings::load_settings()?;
    let descriptor = reader_server::start_session(&state, &value, book_id)?;
    Ok(StandaloneReader {
        _state: state,
        url: descriptor.url,
    })
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ReadingState {
    pub scroll_position: f64,
    pub bookmarks: Vec<usize>,
    /// Reading progress in [0, 1]; used by the welcome screen's continue entry.
    #[serde(default)]
    pub progress: f64,
}

#[derive(Serialize)]
struct ReadResult {
    content: String,
    encoding: String,
}

#[derive(Serialize)]
struct RecentFilesLoad {
    json: String,
    store_exists: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageUsage {
    library_bytes: u64,
    data_bytes: u64,
    cache_bytes: u64,
    logs_bytes: u64,
    backups_bytes: u64,
    runtime_state_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateBackupResult {
    backup_path: String,
    included: Vec<String>,
    skipped: Vec<String>,
}

fn state_dir() -> PathBuf {
    let dir = settings::app_state_dir();
    fs::create_dir_all(&dir).ok();
    dir
}

fn directory_size(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return Ok(metadata.len());
    }
    fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map_err(|error| error.to_string()))
        .try_fold(0_u64, |total, entry| {
            Ok(total.saturating_add(directory_size(&entry?.path())?))
        })
}

fn legacy_state_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mmbook")
}

fn state_path_for(file_path: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    state_dir().join(format!("{:x}.json", hash))
}

fn state_path_for_in_dir(dir: &Path, file_path: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    file_path.hash(&mut hasher);
    let hash = hasher.finish();
    dir.join(format!("{:x}.json", hash))
}

fn decode_markdown_bytes(mut bytes: Vec<u8>) -> Result<(String, String), String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(0..3);
        let content = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        return Ok((content, "utf-8-bom".to_string()));
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (text, _, had_errors) = UTF_16LE.decode(&bytes[2..]);
        if had_errors {
            return Err("Failed to decode UTF-16 LE markdown file".to_string());
        }
        return Ok((text.into_owned(), "utf-16le".to_string()));
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (text, _, had_errors) = UTF_16BE.decode(&bytes[2..]);
        if had_errors {
            return Err("Failed to decode UTF-16 BE markdown file".to_string());
        }
        return Ok((text.into_owned(), "utf-16be".to_string()));
    }

    match String::from_utf8(bytes) {
        Ok(text) => Ok((text, "utf-8".to_string())),
        Err(err) => {
            let bytes = err.into_bytes();
            let (text, _, _) = GB18030.decode(&bytes);
            Ok((text.into_owned(), "gb18030".to_string()))
        }
    }
}

fn encode_markdown(content: &str, encoding: &str) -> Result<Vec<u8>, String> {
    match encoding {
        "utf-8-bom" => {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(content.as_bytes());
            Ok(bytes)
        }
        "utf-16le" => {
            let mut bytes = vec![0xFF, 0xFE];
            for code_unit in content.encode_utf16() {
                bytes.extend_from_slice(&code_unit.to_le_bytes());
            }
            Ok(bytes)
        }
        "utf-16be" => {
            let mut bytes = vec![0xFE, 0xFF];
            for code_unit in content.encode_utf16() {
                bytes.extend_from_slice(&code_unit.to_be_bytes());
            }
            Ok(bytes)
        }
        "gb18030" => {
            let (encoded, _, had_errors) = GB18030.encode(content);
            if had_errors {
                return Err("Failed to encode as GB18030".to_string());
            }
            Ok(encoded.into_owned())
        }
        _ => Ok(content.as_bytes().to_vec()),
    }
}

fn is_markdown_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

fn initial_markdown_path(args: &[String]) -> Option<String> {
    if args.len() <= 1 {
        return None;
    }

    let exact = args[1].clone();
    if is_markdown_path(&exact) {
        return Some(exact);
    }

    // Some Windows launchers pass a path with spaces as multiple argv entries.
    // Reconstruct the tail when it forms a real Markdown path.
    let joined = args[1..].join(" ");
    if is_markdown_path(&joined) && std::path::Path::new(&joined).exists() {
        return Some(joined);
    }

    None
}

/// File mtime in milliseconds since epoch — the frontend polls this to
/// auto-reload when the file is changed by an external editor.
#[tauri::command]
fn get_file_mtime(path: String) -> Result<u64, String> {
    let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
    let modified = meta.modified().map_err(|e| e.to_string())?;
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis() as u64;
    Ok(ms)
}

#[tauri::command]
fn read_markdown_file(app: tauri::AppHandle, path: String) -> Result<ReadResult, String> {
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let (content, encoding) = decode_markdown_bytes(bytes)?;
    if let Some(parent) = std::path::Path::new(&path).parent() {
        app.asset_protocol_scope()
            .allow_directory(parent, true)
            .map_err(|e| e.to_string())?;
    }
    Ok(ReadResult { content, encoding })
}

fn atomic_write_file(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    atomic_file::write(path, data)
}

#[tauri::command]
fn save_markdown_file(path: String, content: String, encoding: String) -> Result<(), String> {
    let bytes = encode_markdown(&content, &encoding)?;
    atomic_write_file(std::path::Path::new(&path), &bytes)
}

#[tauri::command]
fn load_reading_state(path: String) -> Result<ReadingState, String> {
    let sp = state_path_for(&path);
    if sp.exists() {
        let data = fs::read_to_string(&sp).map_err(|e| e.to_string())?;
        serde_json::from_str(&data).map_err(|e| e.to_string())
    } else {
        let legacy = state_path_for_in_dir(&legacy_state_dir(), &path);
        if !legacy.exists() {
            return Ok(ReadingState::default());
        }
        let data = fs::read_to_string(&legacy).map_err(|e| e.to_string())?;
        let state: ReadingState = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        let migrated = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
        atomic_write_file(&sp, &migrated)?;
        Ok(state)
    }
}

#[tauri::command]
fn save_reading_state(path: String, state: ReadingState) -> Result<(), String> {
    let sp = state_path_for(&path);
    let data = serde_json::to_string(&state).map_err(|e| e.to_string())?;
    atomic_write_file(&sp, data.as_bytes())
}

fn delete_reading_state_for_path(path: &str) -> Result<(), String> {
    let sp = state_path_for(path);
    match fs::remove_file(sp) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

fn path_exists(path: &str) -> bool {
    !path.is_empty() && Path::new(path).exists()
}

fn cleanup_recent_files_json(json: &str, state_base_dir: &Path) -> (String, bool) {
    let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return ("[]".to_string(), json.trim() != "[]");
    };

    let original_len = items.len();
    let mut changed = false;
    let mut kept = Vec::with_capacity(items.len());

    for item in items {
        let Some(path) = item.get("path").and_then(|value| value.as_str()) else {
            changed = true;
            continue;
        };

        if path_exists(path) {
            kept.push(item);
        } else {
            let _ = fs::remove_file(state_path_for_in_dir(state_base_dir, path));
            changed = true;
        }
    }

    if kept.len() != original_len {
        changed = true;
    }

    let cleaned = serde_json::to_string(&kept).unwrap_or_else(|_| "[]".to_string());
    if cleaned != json.trim() {
        changed = true;
    }

    (cleaned, changed)
}

/// Recent files list, stored as an opaque JSON string in the app state dir so
/// it survives WebView cache clears (unlike localStorage).
#[tauri::command]
fn load_recent_files() -> Result<RecentFilesLoad, String> {
    let dir = state_dir();
    let path = dir.join("recent-files.json");
    if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let (json, changed) = cleanup_recent_files_json(&raw, &dir);
        if changed {
            atomic_write_file(&path, json.as_bytes())?;
        }
        Ok(RecentFilesLoad {
            json,
            store_exists: true,
        })
    } else {
        let legacy_path = legacy_state_dir().join("recent-files.json");
        if legacy_path.exists() {
            let raw = fs::read_to_string(&legacy_path).map_err(|e| e.to_string())?;
            let (json, _) = cleanup_recent_files_json(&raw, &dir);
            atomic_write_file(&path, json.as_bytes())?;
            return Ok(RecentFilesLoad {
                json,
                store_exists: true,
            });
        }
        Ok(RecentFilesLoad {
            json: "[]".to_string(),
            store_exists: false,
        })
    }
}

#[tauri::command]
fn save_recent_files(json: String) -> Result<String, String> {
    let dir = state_dir();
    let path = dir.join("recent-files.json");
    let (cleaned, _) = cleanup_recent_files_json(&json, &dir);
    atomic_write_file(&path, cleaned.as_bytes())?;
    Ok(cleaned)
}

#[tauri::command]
fn load_reader_preferences() -> Result<reader_preferences::ReaderPreferencesLoad, String> {
    reader_preferences::load()
}

#[tauri::command]
fn save_reader_preferences(
    preferences: reader_preferences::ReaderPreferences,
) -> Result<(), String> {
    reader_preferences::save(&preferences)
}

#[tauri::command]
fn delete_reading_state(path: String) -> Result<(), String> {
    delete_reading_state_for_path(&path)
}

#[tauri::command]
fn markdown_file_exists(path: String) -> bool {
    path_exists(&path)
}

#[tauri::command]
fn get_app_settings() -> Result<settings::AppSettings, String> {
    settings::load_settings()
}

#[tauri::command]
fn get_storage_locations() -> Result<storage::StorageLocations, String> {
    let mut locations = storage::StorageLocations::current()?;
    locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
    Ok(locations)
}

#[tauri::command]
fn get_storage_usage() -> Result<StorageUsage, String> {
    let mut locations = storage::StorageLocations::current()?;
    locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
    Ok(StorageUsage {
        library_bytes: directory_size(&locations.library_root)?,
        data_bytes: directory_size(&locations.data_root)?,
        cache_bytes: directory_size(&locations.cache_root)?,
        logs_bytes: directory_size(&locations.logs_root)?,
        backups_bytes: directory_size(&locations.backups_root)?,
        runtime_state_bytes: directory_size(&locations.runtime_state_root)?,
    })
}

#[tauri::command]
fn create_state_backup() -> Result<StateBackupResult, String> {
    let locations = storage::StorageLocations::current()?;
    fs::create_dir_all(&locations.backups_root).map_err(|error| error.to_string())?;
    let backup_root = locations
        .backups_root
        .join(format!("state-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
    let mut included = Vec::new();
    let mut skipped = vec![
        "library".to_string(),
        "cache".to_string(),
        "logs".to_string(),
        "credentials".to_string(),
        "browser_profiles".to_string(),
    ];
    for (label, source, name) in [
        ("settings", locations.settings_path, "settings.json"),
        (
            "control_db",
            locations.data_root.join(r"App\control.db"),
            "control.db",
        ),
    ] {
        if source.is_file() {
            fs::copy(&source, backup_root.join(name)).map_err(|error| error.to_string())?;
            included.push(label.to_string());
        } else {
            skipped.push(label.to_string());
        }
    }
    skipped.sort();
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "createdAt": chrono::Utc::now().to_rfc3339(),
        "channel": locations.channel,
        "included": included,
        "skipped": skipped,
        "sensitiveData": "excluded",
    });
    atomic_file::write(
        &backup_root.join("backup-manifest.json"),
        &serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )?;
    let included = manifest
        .get("included")
        .and_then(serde_json::Value::as_array)
        .map(|values| values.iter().filter_map(|value| value.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let skipped = manifest
        .get("skipped")
        .and_then(serde_json::Value::as_array)
        .map(|values| values.iter().filter_map(|value| value.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(StateBackupResult {
        backup_path: backup_root.to_string_lossy().into_owned(),
        included,
        skipped,
    })
}

#[tauri::command]
fn reveal_storage_directory(kind: String) -> Result<(), String> {
    let mut locations = storage::StorageLocations::current()?;
    locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
    let path = match kind.as_str() {
        "library" => locations.library_root,
        "data" => locations.data_root,
        "cache" => locations.cache_root,
        "logs" => locations.logs_root,
        "backups" => locations.backups_root,
        "runtime_state" => locations.runtime_state_root,
        _ => return Err("Unknown storage directory".to_string()),
    };
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| error.to_string())
}

#[tauri::command]
fn update_app_settings(value: settings::AppSettings) -> Result<(), String> {
    settings::save_settings(&value)
}

#[tauri::command]
fn clear_safe_cache(
    categories: Vec<cache::CacheCategory>,
    task_ids: Option<Vec<String>>,
) -> Result<cache::CacheClearResult, String> {
    cache::clear_safe_cache_at(
        &storage::StorageLocations::current()?,
        &categories,
        task_ids.as_deref().unwrap_or_default(),
    )
}

#[tauri::command]
fn get_secret_status() -> Result<secrets::SecretStatus, String> {
    secrets::deepseek_status(&settings::AppChannel::current())
}

#[tauri::command]
fn set_deepseek_api_key(api_key: String) -> Result<secrets::SecretStatus, String> {
    secrets::set_deepseek_api_key(&settings::AppChannel::current(), &api_key)
}

#[tauri::command]
fn delete_deepseek_api_key() -> Result<secrets::SecretStatus, String> {
    secrets::delete_deepseek_api_key(&settings::AppChannel::current())
}

#[tauri::command]
fn get_publish_recovery_status() -> Result<Vec<publish::PublishTransaction>, String> {
    let value = settings::load_settings()?;
    publish::list_transactions(Path::new(&value.library_root)).map(|transactions| {
        transactions
            .into_iter()
            .filter(|transaction| {
                !matches!(
                    transaction.phase,
                    publish::PublishPhase::Committed | publish::PublishPhase::RolledBack
                )
            })
            .collect()
    })
}

#[tauri::command]
fn recover_publish_transactions(
    transaction_ids: Option<Vec<String>>,
) -> Result<Vec<publish::PublishTransaction>, String> {
    let value = settings::load_settings()?;
    let library_root = Path::new(&value.library_root);
    let ids = match transaction_ids {
        Some(ids) => ids,
        None => publish::list_transactions(library_root)?
            .into_iter()
            .filter(|transaction| {
                !matches!(
                    transaction.phase,
                    publish::PublishPhase::Committed | publish::PublishPhase::RolledBack
                )
            })
            .map(|transaction| transaction.transaction_id)
            .collect(),
    };
    ids.into_iter()
        .map(|id| publish::recover_transaction(library_root, &id))
        .collect()
}

#[tauri::command]
fn preview_legacy_migration(
    scope: migration::MigrationScope,
) -> Result<migration::MigrationPreview, String> {
    let mut target = storage::StorageLocations::current()?;
    let settings = settings::load_settings()?;
    target.library_root = PathBuf::from(&settings.library_root);
    let legacy = migration::current_legacy_locations(PathBuf::from(settings.library_root))?;
    migration::preview_for(&legacy, &target, scope)
}

#[tauri::command]
fn get_migration_runs() -> Result<Vec<control::MigrationRunRecord>, String> {
    control::ControlDb::open_current()?.migration_runs()
}

#[tauri::command]
fn get_acquisition_snapshot(
    kind: Option<tasks::TaskKind>,
    app: tauri::AppHandle,
) -> Result<tasks::AcquisitionSnapshot, String> {
    tools::recover_stale_engine_instances()?;
    // Connect/start Zhihu sidecar when needed, then reconcile sidecar truth over
    // stale desktop mirrors (including false interrupted/crashed terminals).
    if matches!(kind, None | Some(tasks::TaskKind::Zhihu)) {
        let settings = settings::load_settings()?;
        if let Ok(()) = tools::ensure_zhihu_ready(&settings) {
            let _ = zhihu::reconcile_active_tasks(&settings, Some(&app));
        }
    }
    control::repair_orphaned_podcast_tasks()?;
    let mut control = control::ControlDb::open_current()?;
    let locations = storage::StorageLocations::current_with_library_settings()?;
    reconcile_cancel_and_discard(&locations, &control)?;
    // Keep the queue lean: drop terminal history older than a week.
    let _ = control.prune_terminal_tasks_older_than(7);
    let mut tasks = control.task_snapshots(kind)?;
    // Backfill titles for older snapshots that predate displayName.
    if let Ok(locations) = storage::StorageLocations::current_with_library_settings() {
        enrich_task_display_names(&locations, &mut tasks);
    }
    Ok(tasks::AcquisitionSnapshot {
        recoverable_cache_bytes: tasks
            .iter()
            .filter(|task| task.recoverable)
            .map(|task| task.cache_lease_bytes)
            .sum(),
        tasks,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn enrich_task_display_names(
    locations: &storage::StorageLocations,
    tasks: &mut [tasks::TaskSnapshot],
) {
    for task in tasks.iter_mut() {
        if task
            .display_name
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            continue;
        }
        if !matches!(task.kind, tasks::TaskKind::Podcast) {
            continue;
        }
        let spec_path = locations
            .data_root
            .join("Podcast")
            .join("Tasks")
            .join(&task.id)
            .join("task.json");
        let Ok(raw) = std::fs::read_to_string(&spec_path) else {
            continue;
        };
        let Ok(spec) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let stem = spec
            .pointer("/input/relativePath")
            .and_then(|value| value.as_str())
            .and_then(|path| std::path::Path::new(path).file_stem())
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty());
        if let Some(stem) = stem {
            task.display_name = Some(stem.to_string());
        }
    }
}

fn reconcile_cancel_and_discard(
    locations: &storage::StorageLocations,
    control: &control::ControlDb,
) -> Result<(), String> {
    let pending = control.pending_cancel_discard()?;
    if pending.is_empty() {
        return Ok(());
    }
    let active: BTreeSet<String> = control
        .task_snapshots(None)?
        .into_iter()
        .filter(|snapshot| {
            matches!(
                snapshot.lifecycle_state,
                tasks::LifecycleState::Queued
                    | tasks::LifecycleState::Starting
                    | tasks::LifecycleState::Running
                    | tasks::LifecycleState::Pausing
                    | tasks::LifecycleState::Paused
                    | tasks::LifecycleState::Stopping
            )
        })
        .map(|snapshot| snapshot.id)
        .collect();
    for task_id in pending {
        if active.contains(&task_id) {
            continue;
        }
        cache::discard_podcast_task_at(locations, &task_id)?;
        control.complete_cancel_discard(&task_id)?;
    }
    Ok(())
}

#[tauri::command]
fn get_task_events(
    task_id: String,
    after_sequence: u64,
    limit: u32,
) -> Result<Vec<tasks::TaskEvent>, String> {
    control::ControlDb::open_current()?.task_events(&task_id, after_sequence, limit)
}

#[tauri::command]
fn preview_podcast_files(
    paths: Vec<String>,
    options: podcast::PodcastPreviewOptions,
    state: tauri::State<'_, podcast::PodcastPreviewStore>,
) -> Result<podcast::PodcastFilesPreview, String> {
    let mut locations = storage::StorageLocations::current()?;
    locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
    let preview = podcast::preview_podcast_files_at(&paths, &options, &locations)?;
    state.insert(preview.clone(), options)?;
    Ok(preview)
}

#[tauri::command]
async fn add_podcast_files(
    preview_id: String,
    duplicate_policy: podcast::DuplicatePolicy,
    budget_approval: Option<podcast::PodcastBudgetApproval>,
    request_id: String,
    state: tauri::State<'_, podcast::PodcastPreviewStore>,
    app: tauri::AppHandle,
) -> Result<podcast::PodcastAddResult, String> {
    let store = state.inner().clone();
    let app_for_work = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut locations = storage::StorageLocations::current()?;
        locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
        let mut control = control::ControlDb::open_current()?;
        let request = podcast::AddPodcastFilesRequest {
            preview_id: &preview_id,
            duplicate_policy,
            budget_approval: budget_approval.as_ref(),
            request_id: &request_id,
        };
        podcast::add_podcast_files_at(&store, &mut control, &locations, &request, |event| {
            if let Err(error) = app_for_work.emit(podcast::TASK_EVENT_NAME, event) {
                eprintln!("Task event broadcast failed after persistence: {error}");
            }
        })
    })
    .await
    .map_err(|error| error.to_string())??;

    // Input prep finished: auto-start transcription without a second "开始" click.
    for task in &result.tasks {
        if let Err(error) = podcast::start_task(task.id.clone(), app.clone()) {
            eprintln!("Auto-start podcast task {} failed: {error}", task.id);
        }
    }
    Ok(result)
}

#[tauri::command]
fn scan_library() -> Result<library::LibraryScan, String> {
    let value = settings::load_settings()?;
    library::scan_library(Path::new(&value.library_root))
}

#[tauri::command]
fn open_book(book_id: String) -> Result<library::BookDetail, String> {
    open_book_detail(&book_id)
}

fn open_book_detail(book_id: &str) -> Result<library::BookDetail, String> {
    let value = settings::load_settings()?;
    let mut detail = library::open_book(Path::new(&value.library_root), book_id)?;
    detail.task_records =
        control::ControlDb::open_current()?.task_snapshots_for_book(&detail.manifest.book_id)?;
    Ok(detail)
}

#[tauri::command]
fn get_book_chapter_path(book_id: String, chapter_id: String) -> Result<String, String> {
    let value = settings::load_settings()?;
    let path = library::chapter_path(Path::new(&value.library_root), &book_id, &chapter_id)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn save_book_progress(book_id: String, progress: contracts::ReadingProgress) -> Result<(), String> {
    let value = settings::load_settings()?;
    library::save_book_progress(Path::new(&value.library_root), &book_id, &progress)
}

#[tauri::command]
fn import_markdown_folder(path: String) -> Result<contracts::Manifest, String> {
    let value = settings::load_settings()?;
    importer::import_markdown_folder(Path::new(&path), Path::new(&value.library_root))
}

#[tauri::command]
fn remove_book(book_id: String) -> Result<String, String> {
    let value = settings::load_settings()?;
    library::remove_book(Path::new(&value.library_root), &book_id)
}

#[tauri::command]
fn delete_book(book_id: String) -> Result<String, String> {
    let value = settings::load_settings()?;
    library::delete_book(Path::new(&value.library_root), &book_id)
}

#[tauri::command]
fn list_trash() -> Result<Vec<trash::TrashItem>, String> {
    let value = settings::load_settings()?;
    trash::list(Path::new(&value.library_root))
}

#[tauri::command]
fn restore_trash_item(
    trash_id: String,
    expected_revision: u64,
    request_id: String,
) -> Result<trash::TrashRestoreResult, String> {
    let value = settings::load_settings()?;
    let control = control::ControlDb::open_current()?;
    trash::restore_idempotent(
        Path::new(&value.library_root),
        &control,
        &trash_id,
        expected_revision,
        &request_id,
    )
}

#[tauri::command]
fn permanently_delete_trash_item(
    trash_id: String,
    expected_revision: u64,
    request_id: String,
) -> Result<trash::TrashDeleteResult, String> {
    let value = settings::load_settings()?;
    let control = control::ControlDb::open_current()?;
    trash::delete_idempotent(
        Path::new(&value.library_root),
        &control,
        &trash_id,
        expected_revision,
        &request_id,
    )
}

#[tauri::command]
fn list_temporary_content() -> Result<Vec<temporary_content::TemporaryItem>, String> {
    temporary_content::items()
}

#[tauri::command]
fn get_companion_status(tool: String) -> Result<tools::ToolStatus, String> {
    tools::status(&tool)
}

#[tauri::command]
fn start_reader_session(
    book_id: String,
    state: tauri::State<'_, reader_server::ReaderServiceState>,
) -> Result<reader_server::ReaderSessionDescriptor, String> {
    let value = settings::load_settings()?;
    reader_server::start_session(&state, &value, &book_id)
}

#[tauri::command]
fn close_reader_session(
    session_id: String,
    state: tauri::State<'_, reader_server::ReaderServiceState>,
) -> Result<bool, String> {
    reader_server::close_session(&state, &session_id)
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg(desktop)]
fn schedule_tray_exit_fallback(app: &tauri::AppHandle, delay: Duration) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        app.exit(0);
    });
}

#[tauri::command]
fn cancel_and_discard(app: tauri::AppHandle) -> Result<(), String> {
    let locations = storage::StorageLocations::current()?;
    let mut control = control::ControlDb::open_current()?;
    control.capture_cancel_discard()?;
    tools::stop_all()?;
    control.cancel_active_tasks()?;
    reconcile_cancel_and_discard(&locations, &control)?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn start_podcast_task(task_id: String, app: tauri::AppHandle) -> Result<(), String> {
    podcast::start_task(task_id, app)
}

#[tauri::command]
fn create_zhihu_task(
    request: zhihu::CreateZhihuTaskRequest,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    let settings = settings::load_settings()?;
    let snapshot = zhihu::create_task(&settings, &request)?;
    let event = control::ControlDb::open_current()?
        .task_events(&snapshot.id, 0, 1)?
        .into_iter()
        .next()
        .ok_or_else(|| "TASK_EVENT_MISSING".to_string())?;
    app.emit("acquisition://task-event", event)
        .map_err(|error| error.to_string())?;
    Ok(snapshot)
}

#[tauri::command]
fn get_zhihu_login_status() -> Result<zhihu::ZhihuLoginStatus, String> {
    let settings = settings::load_settings()?;
    zhihu::login_status(&settings)
}

#[tauri::command]
fn start_zhihu_login() -> Result<(), String> {
    let settings = settings::load_settings()?;
    zhihu::start_login(&settings)
}

#[tauri::command]
fn start_zhihu_task(
    task_id: String,
    expected_revision: u64,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    let settings = settings::load_settings()?;
    zhihu::start_task(&task_id, expected_revision, &settings, &app)
}

#[tauri::command]
fn control_zhihu_task(
    task_id: String,
    action: String,
    expected_revision: u64,
    request_id: String,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    let settings = settings::load_settings()?;
    zhihu::control_task(
        &task_id,
        &action,
        expected_revision,
        &request_id,
        &settings,
        &app,
    )
}

#[tauri::command]
async fn restart_podcast_task(
    task_id: String,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    // Heavy copy / publish must not block the UI thread (was causing hard freezes / perceived crashes).
    let app_for_work = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut locations = storage::StorageLocations::current()?;
        locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
        let mut control = control::ControlDb::open_current()?;
        podcast::retry_task_at(&mut control, &locations, &task_id)
    })
    .await
    .map_err(|error| format!("RETRY_JOIN_FAILED: {error}"))??;

    let (snapshot, kind) = result;
    // Emit the latest event for the returned snapshot (republish or new queued task).
    if let Ok(control) = control::ControlDb::open_current() {
        let after = snapshot.last_sequence.saturating_sub(1);
        if let Ok(events) = control.task_events(&snapshot.id, after, 1) {
            if let Some(event) = events.into_iter().next() {
                let _ = app_for_work.emit(podcast::TASK_EVENT_NAME, event);
            }
        }
    }
    // Full restart creates a queued task — auto-start transcription immediately.
    if matches!(kind, podcast::RetryKind::Restarted) {
        if let Err(error) = podcast::start_task(snapshot.id.clone(), app_for_work.clone()) {
            // Surface as soft error string rather than panicking the command.
            return Err(format!(
                "已创建新任务但自动开始失败：{error}。请在任务列表点击「开始」。"
            ));
        }
    }
    Ok(snapshot)
}

#[tauri::command]
fn open_task_result(task_id: String) -> Result<library::BookDetail, String> {
    crate::cache::validate_task_id(&task_id)?;
    let snapshot = control::ControlDb::open_current()?
        .task_snapshot(&task_id)?
        .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
    if !matches!(snapshot.outcome, tasks::TaskOutcome::Success) {
        return Err("TASK_RESULT_NOT_READY".to_string());
    }
    let book_id = snapshot
        .book_id
        .clone()
        .ok_or_else(|| "TASK_RESULT_BOOK_MISSING".to_string())?;
    match open_book_detail(&book_id) {
        Ok(detail) => Ok(detail),
        Err(error) if error.starts_with("Book not found:") => {
            // Recover: worker may have published to the wrong library root historically,
            // or the shelf folder was removed. Re-publish from managed task output.
            let locations = storage::StorageLocations::current_with_library_settings()?;
            let mut control = control::ControlDb::open_current()?;
            let transaction =
                podcast::publish_task_result_at(&mut control, &locations, &task_id).map_err(
                    |publish_error| {
                        format!(
                            "书架中找不到已完成播客。已尝试从任务输出重新发布但失败：{publish_error}"
                        )
                    },
                )?;
            if !matches!(transaction.phase, publish::PublishPhase::Committed) {
                return Err(format!("重新发布未完成（{:?}）", transaction.phase));
            }
            open_book_detail(&transaction.book_id)
                .map_err(|open_error| format!("重新发布后仍无法打开播客：{open_error}"))
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
fn control_podcast_task(
    task_id: String,
    action: String,
    expected_revision: u64,
    request_id: String,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    if request_id.trim().is_empty() {
        return Err("INVALID_REQUEST_ID".to_string());
    }
    let input = serde_json::json!({
        "taskId": task_id,
        "action": action,
        "expectedRevision": expected_revision,
    });
    let input_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&input).map_err(|error| error.to_string())?)
    );
    let mut control = control::ControlDb::open_current()?;
    match control.claim_command(&request_id, "control_podcast_task", &input_hash)? {
        control::CommandClaim::Existing(record) => {
            if let Some(error) = record.error_code {
                return Err(error);
            }
            serde_json::from_str(
                record
                    .result_json
                    .as_deref()
                    .ok_or_else(|| "COMMAND_RESULT_MISSING".to_string())?,
            )
            .map_err(|error| error.to_string())
        }
        control::CommandClaim::New => {
            let result = (|| {
                control.validate_task_control(
                    &task_id,
                    tasks::TaskKind::Podcast,
                    expected_revision,
                )?;
                match action.as_str() {
                    "pause" => podcast::pause_task(&task_id)?,
                    "resume" => podcast::resume_task(&task_id)?,
                    "cancel" | "cancel_and_discard" => {
                        if let Err(error) = podcast::cancel_task(&task_id) {
                            if error != "WORKER_NOT_RUNNING" {
                                return Err(error);
                            }
                        }
                    }
                    _ => return Err("INVALID_TASK_CONTROL".to_string()),
                }
                let event = control.control_task(&task_id, &action, expected_revision)?;
                if action == "cancel_and_discard" {
                    let locations = storage::StorageLocations::current()?;
                    cache::discard_podcast_task_at(&locations, &task_id)?;
                }
                app.emit(podcast::TASK_EVENT_NAME, &event)
                    .map_err(|error| error.to_string())?;
                Ok(event.snapshot)
            })();
            match result {
                Ok(snapshot) => {
                    let json =
                        serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
                    control.complete_command(
                        &request_id,
                        &json,
                        None,
                        i64::try_from(snapshot.revision).ok(),
                    )?;
                    Ok(snapshot)
                }
                Err(error) => {
                    control.complete_command(&request_id, "{}", Some(&error), None)?;
                    Err(error)
                }
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        if let Some(file_path) = initial_markdown_path(&args) {
            let _ = app.emit("open-file", file_path);
        }
    }));
    let app = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_file_mtime,
            read_markdown_file,
            save_markdown_file,
            load_reading_state,
            save_reading_state,
            load_recent_files,
            save_recent_files,
            load_reader_preferences,
            save_reader_preferences,
            delete_reading_state,
            markdown_file_exists,
            get_app_settings,
            get_storage_locations,
            get_storage_usage,
            reveal_storage_directory,
            create_state_backup,
            update_app_settings,
            clear_safe_cache,
            get_secret_status,
            set_deepseek_api_key,
            delete_deepseek_api_key,
            get_publish_recovery_status,
            recover_publish_transactions,
            preview_legacy_migration,
            get_migration_runs,
            get_acquisition_snapshot,
            get_task_events,
            preview_podcast_files,
            add_podcast_files,
            scan_library,
            open_book,
            get_book_chapter_path,
            save_book_progress,
            import_markdown_folder,
            remove_book,
            delete_book,
            list_trash,
            restore_trash_item,
            permanently_delete_trash_item,
            get_companion_status,
            list_temporary_content,
            start_reader_session,
            close_reader_session,
            quit_app,
            cancel_and_discard,
            start_podcast_task,
            create_zhihu_task,
            get_zhihu_login_status,
            start_zhihu_login,
            start_zhihu_task,
            control_zhihu_task,
            restart_podcast_task,
            open_task_result,
            control_podcast_task,
        ])
        .manage(podcast::PodcastPreviewStore::default())
        .manage(reader_server::ReaderServiceState::default())
        .setup(|app| {
            // Windows: file path passed as CLI argument
            let window = app.get_webview_window("main").unwrap();
            let args: Vec<String> = std::env::args().collect();
            if let Some(file_path) = initial_markdown_path(&args) {
                let _ = window.eval(format!(
                    "window.__INITIAL_FILE__ = {};",
                    serde_json::to_string(&file_path).unwrap()
                ));
            }
            #[cfg(desktop)]
            {
                let handle = app.handle();
                let show = MenuItem::with_id(handle, "tray_show", "显示窗口", true, None::<&str>)?;
                let hide = MenuItem::with_id(handle, "tray_hide", "隐藏窗口", true, None::<&str>)?;
                let exit = MenuItem::with_id(
                    handle,
                    "tray_exit_safe",
                    "退出（保留任务）",
                    true,
                    None::<&str>,
                )?;
                let cleanup = MenuItem::with_id(
                    handle,
                    "tray_exit_cleanup",
                    "退出并清理（取消任务）",
                    true,
                    None::<&str>,
                )?;
                let menu = MenuBuilder::new(handle)
                    .items(&[&show, &hide])
                    .separator()
                    .items(&[&exit, &cleanup])
                    .build()?;
                if let Some(tray) = handle.tray_by_id("main") {
                    tray.set_menu(Some(menu))?;
                }
                app.on_menu_event(|app, event| match event.id().as_ref() {
                    "tray_show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "tray_hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "tray_exit_safe" => {
                        let _ = app.emit(
                            "request-app-exit",
                            serde_json::json!({
                                "mode": "preserve"
                            }),
                        );
                        schedule_tray_exit_fallback(app, Duration::from_secs(2));
                    }
                    "tray_exit_cleanup" => {
                        let _ = app.emit(
                            "request-app-exit",
                            serde_json::json!({
                                "mode": "cancel_and_discard"
                            }),
                        );
                        schedule_tray_exit_fallback(app, Duration::from_secs(4));
                    }
                    _ => {}
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // macOS: file opened via Apple Event (double-click / Open With)
    app.run(|app_handle, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            api.prevent_close();
            if let Some(window) = app_handle.get_webview_window(&label) {
                let _ = window.hide();
            }
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        RunEvent::Opened { urls } => {
            for url in urls {
                if let Ok(path) = url.to_file_path() {
                    let path_str = path.to_string_lossy().to_string();
                    if is_markdown_path(&path_str) {
                        let _ = app_handle.emit("open-file", path_str);
                    }
                }
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod recent_file_tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mmbook-recent-test-{}-{}-{}",
            name,
            std::process::id(),
            stamp
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recent_cleanup_keeps_existing_files_only() {
        let dir = temp_test_dir("keeps-existing");
        let existing = dir.join("existing.md");
        let missing = dir.join("missing.md");
        fs::write(&existing, "# Existing").unwrap();

        let raw = serde_json::to_string(&vec![
            json!({
                "path": existing.to_string_lossy(),
                "name": "existing.md",
                "openedAt": 10
            }),
            json!({
                "path": missing.to_string_lossy(),
                "name": "missing.md",
                "openedAt": 9
            }),
        ])
        .unwrap();

        let (cleaned, changed) = cleanup_recent_files_json(&raw, &dir);
        let items: Vec<serde_json::Value> = serde_json::from_str(&cleaned).unwrap();

        assert!(changed);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("path").and_then(|value| value.as_str()),
            Some(existing.to_string_lossy().as_ref())
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recent_cleanup_removes_state_for_missing_files() {
        let dir = temp_test_dir("removes-state");
        let missing_path = dir.join("gone.md").to_string_lossy().to_string();
        let state_path = state_path_for_in_dir(&dir, &missing_path);
        fs::write(
            &state_path,
            r#"{"scroll_position":42,"bookmarks":[],"progress":0.5}"#,
        )
        .unwrap();

        let raw = serde_json::to_string(&vec![json!({
            "path": missing_path,
            "name": "gone.md",
            "openedAt": 1
        })])
        .unwrap();

        let (cleaned, changed) = cleanup_recent_files_json(&raw, &dir);

        assert!(changed);
        assert_eq!(cleaned, "[]");
        assert!(!state_path.exists());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recent_cleanup_treats_malformed_json_as_empty() {
        let dir = temp_test_dir("malformed");
        let (cleaned, changed) = cleanup_recent_files_json("{not json", &dir);

        assert!(changed);
        assert_eq!(cleaned, "[]");

        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::{initial_markdown_path, is_markdown_path};

    #[test]
    fn markdown_extension_check_is_case_insensitive() {
        assert!(is_markdown_path("C:\\docs\\README.MD"));
        assert!(is_markdown_path("/tmp/notes.MarkDown"));
    }

    #[test]
    fn non_markdown_extensions_are_rejected() {
        assert!(!is_markdown_path("C:\\docs\\README.txt"));
        assert!(!is_markdown_path("/tmp/readme.md.bak"));
    }

    #[test]
    fn initial_markdown_path_uses_quoted_path_argument() {
        let args = vec![
            "mmbook.exe".to_string(),
            "C:\\docs\\space name\\README.md".to_string(),
        ];

        assert_eq!(
            initial_markdown_path(&args),
            Some("C:\\docs\\space name\\README.md".to_string())
        );
    }

    #[test]
    fn initial_markdown_path_rejects_non_markdown_argument() {
        let args = vec!["mmbook.exe".to_string(), "C:\\docs\\README.txt".to_string()];

        assert_eq!(initial_markdown_path(&args), None);
    }
}
