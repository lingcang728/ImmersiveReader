use encoding_rs::{GB18030, UTF_16BE, UTF_16LE};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tauri::Manager;
mod atomic_file;
pub mod cache;
mod contracts;
mod importer;
mod library;
pub mod migration;
mod progress;
pub mod publish;
mod reader_http;
mod reader_server;
mod settings;
mod storage;
mod temporary_content;
mod tools;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use tauri::{Emitter, RunEvent};

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
    let url = reader_server::start_session(&state, &value, book_id)?;
    Ok(StandaloneReader { _state: state, url })
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

fn state_dir() -> PathBuf {
    let dir = settings::app_state_dir();
    fs::create_dir_all(&dir).ok();
    dir
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
fn scan_library() -> Result<library::LibraryScan, String> {
    let value = settings::load_settings()?;
    library::scan_library(Path::new(&value.library_root))
}

#[tauri::command]
fn open_book(book_id: String) -> Result<library::BookDetail, String> {
    let value = settings::load_settings()?;
    library::open_book(Path::new(&value.library_root), &book_id)
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
fn launch_companion_tool(tool: String) -> Result<tools::ToolLaunch, String> {
    let value = settings::load_settings()?;
    tools::launch(&tool, &value)
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
) -> Result<String, String> {
    let value = settings::load_settings()?;
    reader_server::start_session(&state, &value, &book_id)
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            get_file_mtime,
            read_markdown_file,
            save_markdown_file,
            load_reading_state,
            save_reading_state,
            load_recent_files,
            save_recent_files,
            delete_reading_state,
            markdown_file_exists,
            get_app_settings,
            get_storage_locations,
            update_app_settings,
            clear_safe_cache,
            get_publish_recovery_status,
            recover_publish_transactions,
            scan_library,
            open_book,
            get_book_chapter_path,
            save_book_progress,
            import_markdown_folder,
            remove_book,
            delete_book,
            launch_companion_tool,
            get_companion_status,
            list_temporary_content,
            start_reader_session,
            quit_app,
        ])
        .manage(reader_server::ReaderServiceState::default())
        .setup(|app| {
            // Windows: file path passed as CLI argument
            let window = app.get_webview_window("main").unwrap();
            let args: Vec<String> = std::env::args().collect();
            if let Some(file_path) = initial_markdown_path(&args) {
                let _ = window.eval(&format!(
                    "window.__INITIAL_FILE__ = {};",
                    serde_json::to_string(&file_path).unwrap()
                ));
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // macOS: file opened via Apple Event (double-click / Open With)
    app.run(|_app_handle, event| match event {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        RunEvent::Opened { urls } => {
            for url in urls {
                if let Ok(path) = url.to_file_path() {
                    let path_str = path.to_string_lossy().to_string();
                    if is_markdown_path(&path_str) {
                        let _ = _app_handle.emit("open-file", path_str);
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
