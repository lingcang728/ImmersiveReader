use encoding_rs::{GB18030, UTF_16BE, UTF_16LE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
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
mod tls;
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

/// P2-20: bound the usage walk — managed roots only hold app-created trees,
/// but a pathological (or previously corrupted) layout must not recurse
/// without limit.
const MAX_DIRECTORY_SIZE_DEPTH: usize = 64;

fn directory_size(path: &Path) -> Result<u64, String> {
    directory_size_at(path, 0)
}

fn directory_size_at(path: &Path, depth: usize) -> Result<u64, String> {
    if depth > MAX_DIRECTORY_SIZE_DEPTH {
        return Ok(0);
    }
    let normalized = atomic_file::long_path(path);
    if !normalized.exists() {
        return Ok(0);
    }
    let metadata = fs::symlink_metadata(&normalized).map_err(|error| error.to_string())?;
    // Junctions are reparse points, not symlinks — `is_symlink()` misses
    // them. A junction points outside the managed root, so count the link
    // itself (≈0) instead of following it into an unrelated tree (or loop).
    if metadata.file_type().is_symlink() || atomic_file::is_reparse_point(&metadata) {
        return Ok(metadata.len());
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    fs::read_dir(&normalized)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map_err(|error| error.to_string()))
        .try_fold(0_u64, |total, entry| {
            Ok(total.saturating_add(directory_size_at(&entry?.path(), depth + 1)?))
        })
}

fn legacy_state_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mmbook")
}

/// State file names need a hash whose output is stable across Rust versions —
/// `DefaultHasher` explicitly does not guarantee that (F17). FNV-1a 64-bit is
/// fixed forever by our own implementation. New files use the `v2-` prefix so
/// they can never collide with legacy SipHash-named files, which are still
/// read and migrated below.
fn stable_state_hash(file_path: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in file_path.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn state_path_for(file_path: &str) -> PathBuf {
    state_dir().join(format!("v2-{:x}.json", stable_state_hash(file_path)))
}

fn state_path_for_in_dir(dir: &Path, file_path: &str) -> PathBuf {
    dir.join(format!("v2-{:x}.json", stable_state_hash(file_path)))
}

/// Pre-F17 name: SipHash via `DefaultHasher`, `{:x}.json`. Only used to find
/// state files written before the switch — never written anymore.
fn legacy_hash_state_path_in_dir(dir: &Path, file_path: &str) -> PathBuf {
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

/// P2-18: never slurp an unbounded file into the WebView — metadata pre-check.
const MAX_MARKDOWN_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Stale-worker watchdog threshold. Must sit clearly above the longest
/// bounded silent stretch a healthy worker can have — ffmpeg invocations run
/// with timeouts up to 600s during which the Python side cannot refresh its
/// heartbeat file — while still converging a truly wedged worker to
/// Interrupted in minutes, not hours (17-F3).
const PODCAST_WORKER_STALE_AFTER: Duration = Duration::from_secs(900);
/// How often the background sweep runs — the snapshot-path reap alone meant
/// a wedged worker survived indefinitely whenever the user never touched the
/// task list.
const PODCAST_WORKER_REAP_INTERVAL: Duration = Duration::from_secs(120);

/// P1-18 whitelist state: Markdown paths this process has actually served to
/// the UI (successful reads, OS open-file hand-offs, recent-files entries).
/// `save_markdown_file` may only write inside managed roots or to a path in
/// this set — the renderer cannot mint a fresh writable target out of thin
/// air without reading it first.
fn opened_markdown_files() -> &'static Mutex<BTreeSet<String>> {
    static OPENED: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    OPENED.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn markdown_set_key(path: &Path) -> String {
    path.canonicalize()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_else(|_| path.to_string_lossy().to_lowercase())
}

fn register_opened_markdown(path: &Path) {
    let Ok(mut set) = opened_markdown_files().lock() else {
        return;
    };
    set.insert(path.to_string_lossy().to_lowercase());
    if let Ok(canonical) = path.canonicalize() {
        set.insert(canonical.to_string_lossy().to_lowercase());
    }
}

/// Shared shape check for every Markdown-path command: absolute path with a
/// `.md`/`.markdown` extension. Without this the renderer could point the
/// commands at any file on disk (settings.json, *.db, keys...).
fn markdown_path_allowed(path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("MARKDOWN_PATH_EMPTY".to_string());
    }
    if !is_markdown_path(path) {
        return Err("MARKDOWN_PATH_TYPE_NOT_ALLOWED".to_string());
    }
    let candidate = PathBuf::from(path);
    if !candidate.is_absolute() {
        return Err("MARKDOWN_PATH_NOT_ABSOLUTE".to_string());
    }
    Ok(candidate)
}

/// Read-side whitelist: any absolute `.md`/`.markdown` file that exists and is
/// small enough. The explicit-open cases this can't enumerate in lib.rs alone
/// (file dialog selection, drag-drop) still flow through here — but they are
/// bounded to Markdown files, and writes stay behind `markdown_write_permitted`.
fn markdown_read_target(path: &str) -> Result<PathBuf, String> {
    let candidate = markdown_path_allowed(path)?;
    // P2-20: normalize before metadata/read — a >MAX_PATH Markdown must be
    // readable via the `\\?\` spelling instead of failing as "not found".
    let normalized = atomic_file::long_path(&candidate);
    let metadata = fs::metadata(&normalized).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("MARKDOWN_PATH_NOT_FILE".to_string());
    }
    if metadata.len() > MAX_MARKDOWN_FILE_BYTES {
        return Err("MARKDOWN_FILE_TOO_LARGE".to_string());
    }
    Ok(normalized)
}

/// Write-side whitelist: managed roots (Library/Data/Cache) always qualify;
/// anything else must be a Markdown file this process already opened.
fn markdown_write_permitted(path: &Path) -> Result<(), String> {
    if let Ok(locations) = storage::StorageLocations::current_with_library_settings() {
        if let Ok(canonical) = path.canonicalize() {
            for root in [
                &locations.library_root,
                &locations.data_root,
                &locations.cache_root,
            ] {
                if let Ok(root) = root.canonicalize() {
                    if let Ok(relative) = canonical.strip_prefix(&root) {
                        // P-11-F17: "inside a managed root" is not enough —
                        // `.trash/<id>/x.md`, `.incoming/<tx>/x.md`,
                        // `.revisions/…`, `.transactions/…` are control areas
                        // a renderer must not write. Every control dir the
                        // app creates is dot-prefixed, so the rule is simple.
                        if relative.components().any(|component| {
                            component.as_os_str().to_string_lossy().starts_with('.')
                        }) {
                            return Err("MARKDOWN_PATH_NOT_ALLOWED".to_string());
                        }
                        return Ok(());
                    }
                }
            }
        }
    }
    let key = markdown_set_key(path);
    let raw = path.to_string_lossy().to_lowercase();
    let opened = opened_markdown_files()
        .lock()
        .map(|set| set.contains(&key) || set.contains(&raw))
        .unwrap_or(false);
    if opened {
        return Ok(());
    }
    Err("MARKDOWN_PATH_NOT_ALLOWED".to_string())
}

/// Chapter images resolve through `convertFileSrc`, which needs an asset://
/// scope grant. The old code recursively granted the file's whole parent
/// subtree — every read permanently widened the exfiltration surface. Now:
/// the Markdown file itself always gets `allow_file`; inside the Library the
/// owning book directory (the ancestor holding manifest.json) gets a
/// recursive `allow_directory` so `assets/` siblings keep rendering. Trade-off:
/// standalone `.md` files outside the Library lose relative-image loading —
/// granting arbitrary user directories recursively is exactly what P1-18
/// removes.
/// P2-44: the ancestor directory holding `manifest.json` — i.e. the book
/// root — is what gets the recursive asset grant, so `chapters/*.md` can
/// reach `assets/` siblings and `../img.png`-style references that stay
/// inside the book. Anything outside the Library (or escaping the book dir,
/// e.g. `../../x.png`) intentionally stays ungranted.
fn book_asset_root(canonical_file: &Path, canonical_library: &Path) -> Option<PathBuf> {
    if !canonical_file.starts_with(canonical_library) {
        return None;
    }
    let mut dir = canonical_file.parent();
    while let Some(current) = dir {
        if current == canonical_library {
            break;
        }
        if current.join("manifest.json").is_file() {
            return Some(current.to_path_buf());
        }
        dir = current.parent();
    }
    None
}

fn grant_markdown_asset_scope(app: &tauri::AppHandle, path: &Path) {
    let scope = app.asset_protocol_scope();
    let _ = scope.allow_file(path);
    let Ok(locations) = storage::StorageLocations::current_with_library_settings() else {
        return;
    };
    let (Ok(canonical_file), Ok(canonical_library)) =
        (path.canonicalize(), locations.library_root.canonicalize())
    else {
        return;
    };
    if let Some(book_root) = book_asset_root(&canonical_file, &canonical_library) {
        let _ = scope.allow_directory(book_root, true);
    }
}

/// P1-17: `fs::copy` on a live WAL database silently skips the -wal file, so
/// the "backup" is a stale/torn snapshot. Same recipe as
/// `migration/sqlite.rs::execute` (checkpoint → integrity_check → VACUUM INTO
/// → verify the copy), but through bundled rusqlite instead of a sqlite3 CLI.
fn backup_sqlite_verified(source: &Path, target: &Path) -> Result<(), String> {
    use rusqlite::{Connection, OpenFlags};
    // Read-write (without CREATE) because wal_checkpoint must fold the live
    // WAL into the main file; VACUUM INTO itself is a consistent committed
    // snapshot regardless of checkpoint busyness.
    let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|error| error.to_string())?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_row| Ok(()))
        .map_err(|error| error.to_string())?;
    let source_integrity: String = db
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if source_integrity != "ok" {
        return Err(format!(
            "control.db integrity check failed: {source_integrity}"
        ));
    }
    let source_version: u32 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let temporary = target.with_extension(format!("backup.{}.db", uuid::Uuid::new_v4()));
    let escaped = temporary.to_string_lossy().replace('\'', "''");
    if let Err(error) = db.execute_batch(&format!("VACUUM INTO '{escaped}'")) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    drop(db);
    let verified = (|| -> Result<(), String> {
        let copy = Connection::open_with_flags(&temporary, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| error.to_string())?;
        let copy_integrity: String = copy
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if copy_integrity != "ok" {
            return Err(format!(
                "backup copy failed integrity check: {copy_integrity}"
            ));
        }
        let copy_version: u32 = copy
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if copy_version != source_version {
            return Err("backup copy user_version differs from source".to_string());
        }
        drop(copy);
        fs::rename(&temporary, target).map_err(|error| error.to_string())
    })();
    if verified.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    verified
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

/// P3-23: the bootstrap `window.__INITIAL_FILE__` injection. The path goes
/// through `serde_json::to_string` so quotes/backslashes can never break out
/// of the JS string literal — interpolating the raw path into eval'd script
/// would be an injection hole.
fn initial_file_eval_script(file_path: &str) -> Option<String> {
    serde_json::to_string(file_path)
        .ok()
        .map(|encoded| format!("window.__INITIAL_FILE__ = {encoded};"))
}

/// File mtime in milliseconds since epoch — the frontend polls this to
/// auto-reload when the file is changed by an external editor.
#[tauri::command]
async fn get_file_mtime(path: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<u64, String> {
        let path = markdown_path_allowed(&path)?;
        let meta = fs::metadata(atomic_file::long_path(&path)).map_err(|e| e.to_string())?;
        let modified = meta.modified().map_err(|e| e.to_string())?;
        let ms = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis() as u64;
        Ok(ms)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn read_markdown_file(app: tauri::AppHandle, path: String) -> Result<ReadResult, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<ReadResult, String> {
        let path = markdown_read_target(&path)?;
        // P2-18: the metadata check above can be raced — cap the read itself
        // so a swapped-in file can never slurp more than the limit + 1 byte.
        let bytes = {
            let file = fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(MAX_MARKDOWN_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_MARKDOWN_FILE_BYTES {
                return Err("MARKDOWN_FILE_TOO_LARGE".to_string());
            }
            bytes
        };
        let (content, encoding) = decode_markdown_bytes(bytes)?;
        register_opened_markdown(&path);
        grant_markdown_asset_scope(&app, &path);
        Ok(ReadResult { content, encoding })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn atomic_write_file(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    atomic_file::write(path, data)
}

/// 12-F3: `markdown_write_permitted` canonicalizes once, but between that
/// check and the atomic rename a parent directory can be swapped for a
/// junction — the write would then land outside the approved root. Right
/// before writing, walk every existing ancestor and refuse reparse points,
/// then repeat the permission decision on the freshly-resolved path.
fn markdown_write_recheck(path: &Path) -> Result<(), String> {
    let mut ancestor = path.parent();
    while let Some(dir) = ancestor {
        match fs::symlink_metadata(dir) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || atomic_file::is_reparse_point(&metadata) =>
            {
                return Err("MARKDOWN_PATH_NOT_ALLOWED".to_string());
            }
            Ok(_) => {}
            // A missing ancestor is normal (create_dir_all will make it);
            // deeper ancestors above it still get checked.
            Err(_) => {}
        }
        ancestor = dir.parent();
    }
    markdown_write_permitted(path)
}

#[tauri::command]
async fn save_markdown_file(path: String, content: String, encoding: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let path = markdown_path_allowed(&path)?;
        markdown_write_permitted(&path)?;
        let bytes = encode_markdown(&content, &encoding)?;
        if bytes.len() as u64 > MAX_MARKDOWN_FILE_BYTES {
            return Err("MARKDOWN_FILE_TOO_LARGE".to_string());
        }
        markdown_write_recheck(&path)?;
        atomic_write_file(&path, &bytes)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// P2-18: reading-state blobs are app-generated and tiny — anything bigger
/// is corrupt or hostile, so read through a hard cap instead of slurping.
fn read_state_json_capped(path: &Path) -> Result<String, String> {
    let normalized = atomic_file::long_path(path);
    let metadata = fs::metadata(&normalized).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_MARKDOWN_FILE_BYTES {
        return Err("STATE_FILE_TOO_LARGE".to_string());
    }
    fs::read_to_string(&normalized).map_err(|error| error.to_string())
}

#[tauri::command]
async fn load_reading_state(path: String) -> Result<ReadingState, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<ReadingState, String> {
        let sp = state_path_for(&path);
        if sp.exists() {
            let data = read_state_json_capped(&sp)?;
            serde_json::from_str(&data).map_err(|e| e.to_string())
        } else {
            // Fallback chain: legacy SipHash name in the current dir, then the
            // pre-move mmbook dir (also SipHash-named). Either hit migrates to
            // the stable v2 name and drops the stale file.
            let old_named = legacy_hash_state_path_in_dir(&state_dir(), &path);
            let legacy = legacy_hash_state_path_in_dir(&legacy_state_dir(), &path);
            let source = if old_named.exists() {
                old_named
            } else if legacy.exists() {
                legacy
            } else {
                return Ok(ReadingState::default());
            };
            let data = read_state_json_capped(&source)?;
            let state: ReadingState = serde_json::from_str(&data).map_err(|e| e.to_string())?;
            let migrated = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
            atomic_write_file(&sp, &migrated)?;
            let _ = fs::remove_file(&source);
            Ok(state)
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn save_reading_state(path: String, state: ReadingState) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let sp = state_path_for(&path);
        let data = serde_json::to_string(&state).map_err(|e| e.to_string())?;
        atomic_write_file(&sp, data.as_bytes())?;
        // Drop a lingering SipHash-named file for this path, if any survived.
        let _ = fs::remove_file(legacy_hash_state_path_in_dir(&state_dir(), &path));
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
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
            let _ = fs::remove_file(legacy_hash_state_path_in_dir(state_base_dir, path));
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
async fn load_recent_files() -> Result<RecentFilesLoad, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<RecentFilesLoad, String> {
        // The recent-files list is pure UI data — it must NOT feed the
        // markdown write whitelist. Registering it here let `save_recent_files`
        // mint write permission for any existing .md on disk without a read.
        let dir = state_dir();
        let path = dir.join("recent-files.json");
        if path.exists() {
            let raw = read_state_json_capped(&path)?;
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
                let raw = read_state_json_capped(&legacy_path)?;
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
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn save_recent_files(json: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let dir = state_dir();
        let path = dir.join("recent-files.json");
        let (cleaned, _) = cleanup_recent_files_json(&json, &dir);
        atomic_write_file(&path, cleaned.as_bytes())?;
        Ok(cleaned)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn load_reader_preferences() -> Result<reader_preferences::ReaderPreferencesLoad, String> {
    tauri::async_runtime::spawn_blocking(reader_preferences::load)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn save_reader_preferences(
    preferences: reader_preferences::ReaderPreferences,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || reader_preferences::save(&preferences))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_app_settings() -> Result<settings::AppSettings, String> {
    tauri::async_runtime::spawn_blocking(settings::load_settings)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_storage_locations() -> Result<storage::StorageLocations, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<storage::StorageLocations, String> {
        let mut locations = storage::StorageLocations::current()?;
        locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
        Ok(locations)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_storage_usage() -> Result<StorageUsage, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<StorageUsage, String> {
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
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn create_state_backup() -> Result<StateBackupResult, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<StateBackupResult, String> {
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
        // settings.json is a plain file — a straight copy is fine.
        let settings_source = locations.settings_path;
        if settings_source.is_file() {
            fs::copy(&settings_source, backup_root.join("settings.json"))
                .map_err(|error| error.to_string())?;
            included.push("settings".to_string());
        } else {
            skipped.push("settings".to_string());
        }
        // control.db is a live WAL database: fs::copy drops the -wal contents
        // and can tear mid-write (P1-17). Snapshot it with the verified
        // checkpoint + VACUUM INTO + integrity_check recipe instead.
        let control_source = locations.data_root.join(r"App\control.db");
        if control_source.is_file() {
            backup_sqlite_verified(&control_source, &backup_root.join("control.db"))?;
            included.push("control_db".to_string());
        } else {
            skipped.push("control_db".to_string());
        }
        skipped.sort();
        // P-10-F7: the manifest carries the version triple — a restore (or a
        // human diffing snapshots) can tell which build/schema produced the
        // backup instead of trusting "state-<uuid>" blindly.
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "createdAt": chrono::Utc::now().to_rfc3339(),
            "channel": locations.channel,
            "appVersion": env!("CARGO_PKG_VERSION"),
            "controlDbSchemaVersion": control::CONTROL_SCHEMA_VERSION,
            "included": included,
            "skipped": skipped,
            "sensitiveData": "excluded",
        });
        atomic_file::write(
            &backup_root.join("backup-manifest.json"),
            &serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
        )?;
        // P-11-F13: bound the snapshot pile — one directory per backup would
        // otherwise accumulate a full control.db copy forever.
        prune_backup_dirs(&locations.backups_root, "state-", STATE_BACKUPS_KEPT);
        let included = manifest
            .get("included")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let skipped = manifest
            .get("skipped")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(StateBackupResult {
            backup_path: backup_root.to_string_lossy().into_owned(),
            included,
            skipped,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// P-11-F13: keep only the newest `keep` `<prefix>-*` snapshot dirs under
/// `backups_root`. Ordering uses the manifest `createdAt` (directory mtime as
/// fallback); best-effort — a directory that cannot be removed is skipped.
fn prune_backup_dirs(backups_root: &Path, prefix: &str, keep: usize) {
    let entries = match fs::read_dir(backups_root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut snapshots: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(prefix) {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let stamp = fs::read_to_string(path.join("backup-manifest.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|manifest| {
                manifest
                    .get("createdAt")
                    .and_then(|value| value.as_str().map(str::to_string))
            })
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(&text).ok())
            .map(|stamp| std::time::SystemTime::from(stamp.with_timezone(&chrono::Utc)))
            .or_else(|| {
                entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
            })
            .unwrap_or(std::time::UNIX_EPOCH);
        snapshots.push((path, stamp));
    }
    snapshots.sort_by_key(|(_, stamp)| *stamp);
    while snapshots.len() > keep {
        if let Some((path, _)) = snapshots.first().cloned() {
            let _ = fs::remove_dir_all(path);
            snapshots.remove(0);
        } else {
            break;
        }
    }
}

/// P-10-F7 / P-11-F13: snapshot retention — `state-*` backups keep the newest
/// few; `pre-restore-*` auto snapshots get a tighter cap.
const STATE_BACKUPS_KEPT: usize = 8;
const PRE_RESTORE_SNAPSHOTS_KEPT: usize = 4;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateBackupInfo {
    path: String,
    created_at: Option<String>,
    app_version: Option<String>,
    channel: Option<String>,
    included: Vec<String>,
}

fn scan_state_backups(backups_root: &Path) -> Vec<StateBackupInfo> {
    let mut backups = Vec::new();
    let entries = match fs::read_dir(backups_root) {
        Ok(entries) => entries,
        Err(_) => return backups,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("state-") {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = fs::read_to_string(path.join("backup-manifest.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
        // A dir without a readable manifest is a half-written backup — not
        // restorable, not listed.
        let Some(manifest) = manifest else { continue };
        if manifest.get("schemaVersion").and_then(|v| v.as_u64()) != Some(1) {
            continue;
        }
        let text = |key: &str| {
            manifest
                .get(key)
                .and_then(|value| value.as_str().map(str::to_string))
        };
        backups.push(StateBackupInfo {
            path: path.to_string_lossy().into_owned(),
            created_at: text("createdAt"),
            app_version: text("appVersion"),
            channel: text("channel"),
            included: manifest
                .get("included")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    backups.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    backups
}

#[tauri::command]
async fn list_state_backups() -> Result<Vec<StateBackupInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let locations = storage::StorageLocations::current()?;
        Ok(scan_state_backups(&locations.backups_root))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateRestoreResult {
    restored: Vec<String>,
    pre_restore_path: String,
}

/// Database filename sidecars (`db-wal`, `db-shm`, …) — appends the suffix
/// to the full file name, mirroring `control.rs::database_sidecar_path`.
fn db_sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// P-10-F7: restore a `create_state_backup` snapshot (settings.json +
/// control.db). Safety rails:
/// - the backup must be a `state-*` dir directly under the managed
///   `Backups\` root with a readable schema-1 manifest;
/// - the incoming control.db is staged, integrity-checked and schema-gated
///   (a snapshot from a NEWER app is refused — the same "too new" rule as
///   `ControlDb::open`);
/// - the current state is snapshotted to `Backups\pre-restore-<uuid>` first,
///   and the live db set is moved aside rather than deleted;
/// - a crash between "live set moved" and "staged rename" is converged by
///   the `control.db.restore-*` pickup in `ControlDb::open_inner`.
fn restore_state_backup_at(
    locations: &storage::StorageLocations,
    backup_dir: &Path,
) -> Result<StateRestoreResult, String> {
    let backups_root = locations
        .backups_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let backup_dir = backup_dir
        .canonicalize()
        .map_err(|_| "BACKUP_NOT_FOUND".to_string())?;
    let is_backup_dir = backup_dir.parent() == Some(backups_root.as_path())
        && backup_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("state-"));
    if !is_backup_dir {
        return Err("BACKUP_OUTSIDE_MANAGED_ROOT".to_string());
    }
    let manifest_raw = fs::read_to_string(backup_dir.join("backup-manifest.json"))
        .map_err(|_| "BACKUP_MANIFEST_MISSING".to_string())?;
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_raw).map_err(|error| error.to_string())?;
    if manifest.get("schemaVersion").and_then(|v| v.as_u64()) != Some(1) {
        return Err("BACKUP_MANIFEST_UNSUPPORTED".to_string());
    }
    let included: std::collections::BTreeSet<String> = manifest
        .get("included")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let live_db = locations.data_root.join(r"App\control.db");
    // Stage + verify the incoming database BEFORE touching live state — the
    // risky I/O runs while rollback is still trivial.
    let staged_db = if included.contains("control_db") {
        let incoming = backup_dir.join("control.db");
        if !incoming.is_file() {
            return Err(
                "BACKUP_INCOMPLETE: manifest lists control_db but the file is missing".to_string(),
            );
        }
        let staged = live_db.with_file_name(format!("control.db.restore-{}", uuid::Uuid::new_v4()));
        crate::atomic_file::copy_file_synced(&incoming, &staged)?;
        let staged_check = (|| -> Result<(), String> {
            use rusqlite::{Connection, OpenFlags};
            let check = Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|error| error.to_string())?;
            check
                .busy_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            let integrity: String = check
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .map_err(|error| error.to_string())?;
            if integrity != "ok" {
                return Err(format!("backup control.db failed integrity: {integrity}"));
            }
            let version: i64 = check
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .map_err(|error| error.to_string())?;
            if version > control::CONTROL_SCHEMA_VERSION {
                return Err(format!(
                    "BACKUP_SCHEMA_TOO_NEW: backup is control schema {version}, newer than this build's {} — update the app instead of restoring",
                    control::CONTROL_SCHEMA_VERSION
                ));
            }
            Ok(())
        })();
        if let Err(error) = staged_check {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        Some(staged)
    } else {
        None
    };

    // Pre-restore snapshot: the current settings + a verified control.db
    // copy land under Backups\pre-restore-<uuid> before any live file moves.
    let pre_root = locations
        .backups_root
        .join(format!("pre-restore-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&pre_root).map_err(|error| error.to_string())?;
    if locations.settings_path.is_file() {
        let _ = crate::atomic_file::copy_file_synced(
            &locations.settings_path,
            &pre_root.join("settings.json"),
        );
    }
    if live_db.is_file() {
        backup_sqlite_verified(&live_db, &pre_root.join("control.db"))?;
    }
    let _ = atomic_file::write(
        &pre_root.join("backup-manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "kind": "pre-restore",
            "createdAt": chrono::Utc::now().to_rfc3339(),
            "appVersion": env!("CARGO_PKG_VERSION"),
            "restoredFrom": backup_dir.to_string_lossy(),
        }))
        .map_err(|error| error.to_string())?
        .as_bytes(),
    );

    let mut restored = Vec::new();
    if let Some(staged) = staged_db {
        // Fold the live WAL so no committed transaction hides in a sidecar
        // we are about to move. Best-effort: a checkpoint failure still
        // leaves a consistent (un-checkpointed) db to preserve.
        if live_db.is_file() {
            if let Ok(db) = rusqlite::Connection::open(&live_db) {
                let _ = db.busy_timeout(Duration::from_secs(5));
                let _ = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
            }
        }
        let previous_dir = pre_root.join("previous-live");
        fs::create_dir_all(&previous_dir).map_err(|error| error.to_string())?;
        for suffix in ["-wal", "-shm", "-journal", ""] {
            let from = db_sidecar(&live_db, suffix);
            if from.exists() {
                let name = format!("control.db{suffix}");
                fs::rename(&from, previous_dir.join(name))
                    .map_err(|error| format!("control.db is in use; restore aborted: {error}"))?;
            }
        }
        // Live set is parked; the staged copy takes over. A crash here is
        // converged by the `.restore-*` pickup at next open.
        fs::rename(&staged, &live_db).map_err(|error| error.to_string())?;
        restored.push("control_db".to_string());
    }
    if included.contains("settings") {
        let incoming = backup_dir.join("settings.json");
        if incoming.is_file() {
            // Parse through the migration-aware reader so a v1/v2 backup
            // restores as current-shape settings rather than legacy JSON.
            let mut parsed = settings::load_compatible_from(&incoming)
                .map_err(|error| format!("BACKUP_SETTINGS_UNREADABLE: {error}"))?;
            // Same production pin as save_settings — a restored root must not
            // smuggle a custom library path into the production channel.
            if locations.channel == "production" {
                parsed.library_root = locations.library_root.to_string_lossy().into_owned();
            }
            settings::save_compatible_to(&locations.settings_path, &parsed)
                .map_err(|error| error.to_string())?;
            restored.push("settings".to_string());
        }
    }
    prune_backup_dirs(
        &locations.backups_root,
        "pre-restore-",
        PRE_RESTORE_SNAPSHOTS_KEPT,
    );
    prune_backup_dirs(&locations.backups_root, "state-", STATE_BACKUPS_KEPT);
    Ok(StateRestoreResult {
        restored,
        pre_restore_path: pre_root.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
async fn restore_state_backup(backup_path: String) -> Result<StateRestoreResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let locations = storage::StorageLocations::current()?;
        restore_state_backup_at(&locations, Path::new(&backup_path))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// P-10-F6: one-shot "control.db was rebuilt empty" notice — the flag is set
/// the moment a corrupt database is quarantined and consumed by the UI's
/// startup poll, so an empty task list is explained instead of silent.
#[tauri::command]
async fn take_control_db_recovery_notice() -> Result<bool, String> {
    Ok(control::take_recovery_pending())
}

#[tauri::command]
async fn reveal_storage_directory(kind: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
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
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn update_app_settings(value: settings::AppSettings) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Production pins the library to Documents/沉浸阅读/Library (see
        // settings::save_settings). Reject divergent roots here instead of
        // silently discarding the user's choice after a "已更新" notice.
        let locations = storage::StorageLocations::current()?;
        if locations.channel == "production" {
            // Canonical compare — a raw string match would let `..\`
            // segments or 8.3 spellings slip a divergent root past the pin.
            let pinned = &locations.library_root;
            let requested = Path::new(&value.library_root);
            let same =
                storage::path_within(pinned, requested) && storage::path_within(requested, pinned);
            if !same {
                let canonical = pinned.to_string_lossy().replace('/', "\\");
                return Err(format!("正式版书库位置固定为 {canonical}，不支持自定义"));
            }
        }
        settings::save_settings(&value)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn clear_safe_cache(
    categories: Vec<cache::CacheCategory>,
    task_ids: Option<Vec<String>>,
) -> Result<cache::CacheClearResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        cache::clear_safe_cache_at(
            &storage::StorageLocations::current()?,
            &categories,
            task_ids.as_deref().unwrap_or_default(),
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_secret_status() -> Result<secrets::SecretStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        secrets::deepseek_status(&settings::AppChannel::current())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn set_deepseek_api_key(api_key: String) -> Result<secrets::SecretStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        secrets::set_deepseek_api_key(&settings::AppChannel::current(), &api_key)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn delete_deepseek_api_key() -> Result<secrets::SecretStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        secrets::delete_deepseek_api_key(&settings::AppChannel::current())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_publish_recovery_status() -> Result<Vec<publish::PublishTransaction>, String> {
    tauri::async_runtime::spawn_blocking(|| {
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
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn recover_publish_transactions(
    transaction_ids: Option<Vec<String>>,
) -> Result<Vec<publish::PublishTransaction>, String> {
    tauri::async_runtime::spawn_blocking(
        move || -> Result<Vec<publish::PublishTransaction>, String> {
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
            // P-11-F02: batch tolerance — one wedged journal must not abort
            // recovery for every transaction after it. Collect what
            // converged; failures stay logged and their journals remain for
            // the next pass (or manual salvage).
            let mut recovered = Vec::new();
            for id in ids {
                match publish::recover_transaction(library_root, &id) {
                    Ok(transaction) => recovered.push(transaction),
                    Err(error) => {
                        eprintln!("publish recovery: transaction {id} failed: {error}")
                    }
                }
            }
            Ok(recovered)
        },
    )
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn preview_legacy_migration(
    scope: migration::MigrationScope,
) -> Result<migration::MigrationPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut target = storage::StorageLocations::current()?;
        let settings = settings::load_settings()?;
        target.library_root = PathBuf::from(&settings.library_root);
        let legacy = migration::current_legacy_locations(PathBuf::from(settings.library_root))?;
        migration::preview_for(&legacy, &target, scope)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_migration_runs() -> Result<Vec<control::MigrationRunRecord>, String> {
    tauri::async_runtime::spawn_blocking(|| control::ControlDb::open_current()?.migration_runs())
        .await
        .map_err(|error| error.to_string())?
}

// P2-14 registration half: the three migration executors existed but were
// unreachable — nothing registered them as Tauri commands. Idempotency
// (claim settlement, stale-preview checks, conflict gate) lives in the
// execution layer; these wrappers only derive managed locations and hop off
// the IPC thread.

#[tauri::command]
async fn execute_settings_migration(
    preview_id: String,
    request_id: String,
) -> Result<migration::MigrationExecutionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut target = storage::StorageLocations::current()?;
        let settings = settings::load_settings()?;
        target.library_root = PathBuf::from(&settings.library_root);
        let legacy = migration::current_legacy_locations(PathBuf::from(settings.library_root))?;
        migration::execute_settings_migration(&legacy, &target, &preview_id, &request_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// P-11-F04 / P-10-F17: a renderer-supplied path is only writable when it
/// resolves under a managed root. The target may not exist yet (the whole
/// point of a migration target), so canonicalize the deepest ancestor that
/// does and compare that.
fn managed_write_target(path: &Path, locations: &storage::StorageLocations) -> Result<(), String> {
    // Canonicalize through the deepest ancestor that exists, then re-attach
    // the not-yet-existing tail — the target (or even a managed root) may be
    // absent on a fresh install, but its resolved location still compares.
    fn canonical_through_existing(path: &Path) -> Option<PathBuf> {
        let mut probe = path;
        loop {
            match probe.canonicalize() {
                Ok(value) => {
                    let tail = path.strip_prefix(probe).ok()?;
                    return Some(value.join(tail));
                }
                Err(_) => probe = probe.parent()?,
            }
        }
    }
    if !path.is_absolute() {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    let canonical = canonical_through_existing(path).ok_or("PATH_OUTSIDE_MANAGED_ROOT")?;
    for root in [
        &locations.data_root,
        &locations.backups_root,
        &locations.library_root,
        &locations.cache_root,
    ] {
        if let Some(canonical_root) = canonical_through_existing(root) {
            if canonical.starts_with(&canonical_root) {
                return Ok(());
            }
        }
    }
    Err("PATH_OUTSIDE_MANAGED_ROOT".to_string())
}

/// Read-side counterpart of `managed_write_target` for the migration escape
/// hatches: the file must exist and resolve under a managed root or one of
/// the legacy state dirs these commands exist to migrate. Bounding the read
/// matters because the source is byte-copied into managed dirs (rollback /
/// work set) and its row counts + SHA-256 land in the receipt the caller
/// sees — unconstrained, it is an arbitrary-file exfiltration primitive.
fn migration_read_target(path: &Path, locations: &storage::StorageLocations) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    let canonical = path.canonicalize().map_err(|error| error.to_string())?;
    let mut roots = vec![
        locations.data_root.clone(),
        locations.cache_root.clone(),
        locations.backups_root.clone(),
        locations.library_root.clone(),
    ];
    if let Ok(legacy) = migration::current_legacy_locations(locations.library_root.clone()) {
        roots.extend([
            legacy.immersive_state,
            legacy.mmbook_state,
            legacy.podcast_root,
            legacy.zhihu_root,
        ]);
    }
    for root in roots {
        if let Ok(root) = root.canonicalize() {
            if canonical.starts_with(&root) {
                return Ok(());
            }
        }
    }
    Err("PATH_OUTSIDE_MANAGED_ROOT".to_string())
}

#[tauri::command]
async fn migrate_sqlite_verified(
    source: String,
    target: String,
) -> Result<migration::MigrationReceipt, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let locations = storage::StorageLocations::current_with_library_settings()?;
        // P-11-F04: the target is an arbitrary-file-create primitive (its
        // content is a copy of the source DB) — it must land under a managed
        // root. The source is bounded too: it is byte-copied into managed
        // dirs and its hash/row counts reach the caller via the receipt.
        managed_write_target(Path::new(&target), &locations)?;
        migration_read_target(Path::new(&source), &locations)?;
        // Rollback + receipt stay under the managed Data root — the caller
        // picks source/target only, never where safety copies land.
        let run_root = locations
            .data_root
            .join("Migrations")
            .join(format!("sqlite-{}", uuid::Uuid::new_v4()));
        let rollback = run_root.join("rollback");
        let receipt_path = run_root.join("receipt.json");
        migration::migrate_sqlite_verified(
            Path::new(&source),
            Path::new(&target),
            &rollback,
            &receipt_path,
            env!("CARGO_PKG_VERSION"),
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn reconcile_zhihu_archive(
    database: String,
    output_root: String,
) -> Result<migration::ReconciliationReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let locations = storage::StorageLocations::current_with_library_settings()?;
        let database = PathBuf::from(&database);
        let output_root = PathBuf::from(&output_root);
        migration_read_target(&database, &locations)?;
        // The output tree is only scanned, never written — but the scan is
        // still a filesystem read primitive, so it is bounded to the
        // Library plus the legacy content roots it exists to reconcile.
        let output_allowed = output_root.is_absolute()
            && output_root.is_dir()
            && ([
                locations.library_root.as_path(),
                locations.data_root.as_path(),
            ]
            .iter()
            .any(|&root| storage::path_within(root, &output_root))
                || migration::current_legacy_locations(locations.library_root.clone())
                    .map(|legacy| {
                        [legacy.zhihu_root.as_path(), legacy.podcast_root.as_path()]
                            .iter()
                            .any(|&root| storage::path_within(root, &output_root))
                    })
                    .unwrap_or(false));
        if !output_allowed {
            return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
        }
        migration::reconcile_zhihu_archive(&database, &output_root)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_acquisition_snapshot(
    kind: Option<tasks::TaskKind>,
    app: tauri::AppHandle,
) -> Result<tasks::AcquisitionSnapshot, String> {
    // Warm up the Zhihu engine on a dedicated background worker (deduped
    // internally) instead of launching it inline here — the snapshot path must
    // never spawn the engine or wait on its readiness/health probes itself.
    if matches!(kind, None | Some(tasks::TaskKind::Zhihu)) {
        crate::tools::request_engine_warmup(crate::tools::ToolKind::Zhihu);
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<tasks::AcquisitionSnapshot, String> {
        tools::recover_stale_engine_instances()?;
        // Reconcile sidecar truth over stale desktop mirrors (including
        // false interrupted/crashed terminals) — but only when the engine
        // is already running: zhihu_* HTTP calls would otherwise launch it
        // inline and block this path on spawn + readiness waits.
        if matches!(kind, None | Some(tasks::TaskKind::Zhihu))
            && tools::status("zhihu")
                .map(|status| status.state == "running")
                .unwrap_or(false)
        {
            let settings = settings::load_settings()?;
            let _ = zhihu::reconcile_active_tasks(&settings, Some(&app));
        }
        control::repair_orphaned_podcast_tasks()?;
        let control = control::ControlDb::open_current()?;
        let locations = storage::StorageLocations::current_with_library_settings()?;
        // P1-15 watchdog: the Python worker refreshes work/state/*.json
        // heartbeats every ~15s; when that file goes silent the worker is
        // wedged/killed/suspended and its task must not stay "Running"
        // forever — mark it Interrupted so checkpoint resume can take over.
        // work_root = Cache\Podcast\Tasks (each <task_id>\work\state lives
        // underneath, per transcribe_task.py/common.py).
        if matches!(kind, None | Some(tasks::TaskKind::Podcast)) {
            let work_root = locations.cache_root.join("Podcast").join("Tasks");
            match control.reap_stale_workers(&work_root, PODCAST_WORKER_STALE_AFTER) {
                Ok(events) => {
                    for event in events {
                        let _ = app.emit(podcast::TASK_EVENT_NAME, event);
                    }
                }
                Err(error) => eprintln!("stale podcast worker reap failed: {error}"),
            }
        }
        reconcile_cancel_and_discard(&locations, &control)?;
        // Keep the queue lean: drop terminal history older than a week.
        let _ = control.prune_terminal_tasks_older_than(7);
        let mut tasks = control.task_snapshots(kind)?;
        // Backfill titles for older snapshots that predate displayName.
        if let Ok(locations) = storage::StorageLocations::current_with_library_settings() {
            enrich_task_display_names(&control, &locations, &mut tasks);
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
    })
    .await
    .map_err(|error| error.to_string())?
}

fn enrich_task_display_names(
    control: &control::ControlDb,
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
        let Ok(raw) = read_state_json_capped(&spec_path) else {
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
            // 16-F6: write the backfill back once — otherwise legacy rows
            // re-read task.json on every focus/refresh snapshot forever.
            // Best effort: a failed write just re-enriches next time.
            if let Err(error) = control.set_task_display_name(&task.id, stem) {
                eprintln!(
                    "displayName backfill persist failed for {}: {error}",
                    task.id
                );
            }
        }
    }
}

/// P2-11: per-task `cancel_and_discard` cannot reuse
/// `control.capture_cancel_discard` — it snapshots *every* active podcast
/// task, so it would plant discard intents for tasks the user never
/// cancelled. A sibling `<task_id>.discard-pending` marker under
/// `Cache\Podcast\Tasks` is the durable intent instead: it is written before
/// the fallible discard, survives a mid-discard crash (it lives outside the
/// directory being deleted), and `reconcile_cancel_and_discard` retries any
/// leftover marker at startup / on every acquisition snapshot.
const DISCARD_INTENT_SUFFIX: &str = ".discard-pending";

fn discard_intent_path(locations: &storage::StorageLocations, task_id: &str) -> PathBuf {
    locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(format!("{task_id}{DISCARD_INTENT_SUFFIX}"))
}

fn reconcile_discard_markers(locations: &storage::StorageLocations, control: &control::ControlDb) {
    let tasks_dir = atomic_file::long_path(&locations.cache_root.join("Podcast").join("Tasks"));
    let Ok(entries) = fs::read_dir(&tasks_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(task_id) = name
            .to_str()
            .and_then(|name| name.strip_suffix(DISCARD_INTENT_SUFFIX))
        else {
            continue;
        };
        if task_id.is_empty() {
            continue;
        }
        // 02-F8: the marker is a durable intent, but only for a task whose
        // discard actually committed (or whose row is gone). A marker
        // stranded by a crash between write and the failed-control rollback
        // may belong to a still-live task — deleting its cache mid-run is
        // worse than retrying later, so skip the delete and keep the marker.
        match control.task_snapshot(task_id) {
            Ok(None) => {}
            Ok(Some(snapshot))
                if matches!(snapshot.lifecycle_state, tasks::LifecycleState::Terminal) => {}
            _ => continue,
        }
        // Best effort per marker: a task whose cache is already gone reports
        // Ok, a genuinely stuck discard keeps its marker for the next sweep.
        if cache::discard_podcast_task_at(locations, task_id).is_ok() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn reconcile_cancel_and_discard(
    locations: &storage::StorageLocations,
    control: &control::ControlDb,
) -> Result<(), String> {
    reconcile_discard_markers(locations, control);
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
        // Per-intent tolerance: one poisoned marker must not stall the sweep
        // for every later pending discard (was `?` on the first error).
        if let Err(error) = cache::discard_podcast_task_at(locations, &task_id) {
            eprintln!("cancel_and_discard reconcile: discard {task_id} failed: {error}");
            continue;
        }
        if let Err(error) = control.complete_cancel_discard(&task_id) {
            eprintln!("cancel_and_discard reconcile: complete {task_id} failed: {error}");
        }
    }
    Ok(())
}

#[tauri::command]
async fn preview_podcast_files(
    paths: Vec<String>,
    options: podcast::PodcastPreviewOptions,
    state: tauri::State<'_, podcast::PodcastPreviewStore>,
) -> Result<podcast::PodcastFilesPreview, String> {
    // SHA-256 hashing + ffprobe per file must not run on the IPC thread.
    let store = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut locations = storage::StorageLocations::current()?;
        locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
        let preview = podcast::preview_podcast_files_at(&paths, &options, &locations)?;
        store.insert(preview.clone(), options)?;
        Ok(preview)
    })
    .await
    .map_err(|error| error.to_string())?
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
    // Worker spawn + DB writes stay off the async runtime's worker threads.
    let task_ids: Vec<String> = result.tasks.iter().map(|task| task.id.clone()).collect();
    tauri::async_runtime::spawn_blocking(move || {
        for task_id in task_ids {
            if let Err(error) = podcast::start_task(task_id.clone(), app.clone()) {
                eprintln!("Auto-start podcast task {task_id} failed: {error}");
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(result)
}

#[tauri::command]
async fn scan_library() -> Result<library::LibraryScan, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let value = settings::load_settings()?;
        library::scan_library(Path::new(&value.library_root))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn open_book(book_id: String) -> Result<library::BookDetail, String> {
    tauri::async_runtime::spawn_blocking(move || open_book_detail(&book_id))
        .await
        .map_err(|error| error.to_string())?
}

fn open_book_detail(book_id: &str) -> Result<library::BookDetail, String> {
    let value = settings::load_settings()?;
    let mut detail = library::open_book(Path::new(&value.library_root), book_id)?;
    detail.task_records =
        control::ControlDb::open_current()?.task_snapshots_for_book(&detail.manifest.book_id)?;
    Ok(detail)
}

#[tauri::command]
async fn get_book_chapter_path(book_id: String, chapter_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let value = settings::load_settings()?;
        let path = library::chapter_path(Path::new(&value.library_root), &book_id, &chapter_id)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn save_book_progress(
    book_id: String,
    progress: contracts::ReadingProgress,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        library::save_book_progress(Path::new(&value.library_root), &book_id, &progress)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn import_markdown_folder(path: String) -> Result<importer::ImportOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        importer::import_markdown_folder(Path::new(&path), Path::new(&value.library_root))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn remove_book(
    book_id: String,
    state: tauri::State<'_, std::sync::Arc<reader_server::ReaderServiceState>>,
) -> Result<String, String> {
    // P2-21: close every 连读 session on this book first — a live session's
    // PUT /progress would otherwise re-create .reading.json (and the book
    // directory) underneath a remove in flight.
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        // 12-F4: session sweep + book removal are one critical section, so a
        // racing start_reader_session cannot register past the sweep.
        let _lifecycle = reader_book_lifecycle_lock().lock().ok();
        close_reader_sessions_for_book(&state, &book_id);
        library::remove_book(Path::new(&value.library_root), &book_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn delete_book(
    book_id: String,
    state: tauri::State<'_, std::sync::Arc<reader_server::ReaderServiceState>>,
) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        let _lifecycle = reader_book_lifecycle_lock().lock().ok();
        close_reader_sessions_for_book(&state, &book_id);
        library::delete_book(Path::new(&value.library_root), &book_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn list_trash() -> Result<Vec<trash::TrashItem>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let value = settings::load_settings()?;
        trash::list(Path::new(&value.library_root))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn restore_trash_item(
    trash_id: String,
    expected_revision: u64,
    request_id: String,
) -> Result<trash::TrashRestoreResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        let control = control::ControlDb::open_current()?;
        trash::restore_idempotent(
            Path::new(&value.library_root),
            &control,
            &trash_id,
            expected_revision,
            &request_id,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn permanently_delete_trash_item(
    trash_id: String,
    expected_revision: u64,
    request_id: String,
) -> Result<trash::TrashDeleteResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        let control = control::ControlDb::open_current()?;
        trash::delete_idempotent(
            Path::new(&value.library_root),
            &control,
            &trash_id,
            expected_revision,
            &request_id,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn list_temporary_content() -> Result<Vec<temporary_content::TemporaryItem>, String> {
    tauri::async_runtime::spawn_blocking(temporary_content::items)
        .await
        .map_err(|error| error.to_string())?
}

// P3-24: diagnostics surface, intentionally registered — no frontend caller
// today; `tools::status` documents why the command stays wired (health-gate
// visibility for support).
#[tauri::command]
async fn get_companion_status(tool: String) -> Result<tools::ToolStatus, String> {
    // Locks TOOL_MANAGER + touches the control DB — never on the IPC thread.
    tauri::async_runtime::spawn_blocking(move || tools::status(&tool))
        .await
        .map_err(|error| error.to_string())?
}

/// P2-21: `reader_server` keeps its Sessions map private, so lib.rs tracks
/// book_id → session_id alongside `start_reader_session`. `remove_book`/
/// `delete_book` close every tracked session first so the tiny_http reader
/// cannot recreate files (e.g. .reading.json via PUT /progress) inside a
/// directory that is being removed.
fn reader_sessions_by_book() -> &'static Mutex<BTreeMap<String, Vec<String>>> {
    static MAP: OnceLock<Mutex<BTreeMap<String, Vec<String>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Serializes "open a reader session for a book" against "delete that book".
/// `remove_book`/`delete_book` sweep tracked sessions *before* the directory
/// removal — without a shared critical section a session could be created in
/// between and never closed, letting PUT /progress recreate `.reading.json`
/// inside the removed tree (12-F4). Held only across the bookkeeping; the
/// session/book IO under it is rare and bounded.
fn reader_book_lifecycle_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn track_reader_session(
    state: &reader_server::ReaderServiceState,
    book_id: &str,
    session_id: &str,
) {
    if let Ok(mut map) = reader_sessions_by_book().lock() {
        let ids = map.entry(book_id.to_string()).or_default();
        // Sweep ids the server already retired (TTL expiry or a service
        // restart) — otherwise a long-lived app accumulates dead ids per
        // book and delete_book wastes close attempts on ghosts.
        ids.retain(|id| state.session_alive(id));
        ids.push(session_id.to_string());
    }
}

fn untrack_reader_session(session_id: &str) {
    if let Ok(mut map) = reader_sessions_by_book().lock() {
        for ids in map.values_mut() {
            ids.retain(|id| id != session_id);
        }
        map.retain(|_, ids| !ids.is_empty());
    }
}

fn close_reader_sessions_for_book(state: &reader_server::ReaderServiceState, book_id: &str) {
    let ids = reader_sessions_by_book()
        .lock()
        .map(|mut map| map.remove(book_id).unwrap_or_default())
        .unwrap_or_default();
    for id in ids {
        // A TTL-expired or already-closed session reports Ok(false)/Err —
        // either way it no longer serves the book; keep closing the rest.
        if let Err(error) = reader_server::close_session(state, &id) {
            eprintln!("reader session {id} close before book removal failed: {error}");
        }
    }
}

#[tauri::command]
async fn start_reader_session(
    book_id: String,
    state: tauri::State<'_, std::sync::Arc<reader_server::ReaderServiceState>>,
) -> Result<reader_server::ReaderSessionDescriptor, String> {
    // Server bind + reader template read are blocking IO — keep off IPC thread.
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        // 12-F4: track atomically against remove_book/delete_book — a session
        // created during their sweep gap would escape closure and keep
        // writing into the removed book directory.
        let _lifecycle = reader_book_lifecycle_lock().lock().ok();
        let descriptor = reader_server::start_session(&state, &value, &book_id)?;
        track_reader_session(&state, &book_id, &descriptor.session_id);
        Ok(descriptor)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn close_reader_session(
    session_id: String,
    state: tauri::State<'_, std::sync::Arc<reader_server::ReaderServiceState>>,
) -> Result<bool, String> {
    // Session teardown joins the accept thread — keep off IPC thread.
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let closed = reader_server::close_session(&state, &session_id)?;
        untrack_reader_session(&session_id);
        Ok(closed)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn quit_app(app: tauri::AppHandle) {
    // Exit immediately without touching tool/engine locks: Job Objects reap the
    // sidecars at process exit, and the frontend has already flushed its state.
    app.exit(0);
}

/// Arms/disarms the tray-exit hard fallback. Each arming bumps the epoch; the
/// armed thread only exits if the epoch is still its own when it wakes — a
/// `cancel_exit_fallback` (or a newer arming) retires older fallbacks.
static TRAY_EXIT_EPOCH: AtomicU64 = AtomicU64::new(0);

#[cfg(desktop)]
fn schedule_tray_exit_fallback(app: &tauri::AppHandle, delay: Duration) {
    let epoch = TRAY_EXIT_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if TRAY_EXIT_EPOCH.load(Ordering::SeqCst) == epoch {
            app.exit(0);
        }
    });
}

#[tauri::command]
fn cancel_exit_fallback() {
    TRAY_EXIT_EPOCH.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
async fn cancel_and_discard(app: tauri::AppHandle) -> Result<(), String> {
    // Graceful flush (worker/engine stop + DB intent) happens on a blocking
    // worker so the event loop stays responsive; the tray fallback still bounds
    // total shutdown time if the tools lock is held elsewhere.
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let locations = storage::StorageLocations::current()?;
        let mut control = control::ControlDb::open_current()?;
        control.capture_cancel_discard()?;
        tools::stop_all()?;
        control.cancel_active_tasks()?;
        reconcile_cancel_and_discard(&locations, &control)?;
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn start_podcast_task(task_id: String, app: tauri::AppHandle) -> Result<bool, String> {
    // Returns whether a worker actually spawned: when the single concurrency
    // slot is taken, `start_task` deliberately leaves the task Queued and the
    // UI must say "已排队" rather than "已启动" (05-F19).
    tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        podcast::start_task(task_id.clone(), app)?;
        let still_queued = control::ControlDb::open_current()?
            .task_snapshot(&task_id)?
            .map(|snapshot| snapshot.lifecycle_state == tasks::LifecycleState::Queued)
            .unwrap_or(false);
        Ok(!still_queued)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn create_zhihu_task(
    request: zhihu::CreateZhihuTaskRequest,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<tasks::TaskSnapshot, String> {
        // P3-26: this command had no idempotency claim — a retried click
        // or relaunch could ask the sidecar for a second archive task.
        // No request_id reaches us from the frontend, so the claim key is
        // derived from the request itself: an identical retry replays the
        // stored snapshot instead of creating a duplicate task.
        let input_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&request).map_err(|error| error.to_string())?)
        );
        let request_id = format!("create-zhihu-task-{input_hash}");
        let control = control::ControlDb::open_current()?;
        match control.claim_command(&request_id, "create_zhihu_task", &input_hash)? {
            control::CommandClaim::Existing(record) => {
                match replay_task_snapshot(&record) {
                    Ok(snapshot) if snapshot.lifecycle_state != tasks::LifecycleState::Terminal => {
                        Ok(snapshot)
                    }
                    // A terminal stored snapshot is not a duplicate —
                    // re-adding the same person after the old task
                    // finished must create a fresh task, not replay the
                    // dead one forever. Reopen the completed claim first:
                    // `complete_command` is first-wins on `completed_at`,
                    // so without the CAS the re-run could never settle.
                    Ok(_) => {
                        if control.reopen_completed_command(&request_id)? {
                            run_new_zhihu_task(control, request, app, request_id)
                        } else {
                            // Lost the reopen race — replay whatever the
                            // winning caller produces/stored.
                            match control.claim_command(
                                &request_id,
                                "create_zhihu_task",
                                &input_hash,
                            )? {
                                control::CommandClaim::Existing(record) => {
                                    replay_task_snapshot(&record)
                                }
                                control::CommandClaim::New => {
                                    run_new_zhihu_task(control, request, app, request_id)
                                }
                            }
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            control::CommandClaim::New => run_new_zhihu_task(control, request, app, request_id),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Decode a stored `command_results` row back into a task snapshot. An
/// in-progress row (claimed but not settled) reports `COMMAND_IN_PROGRESS`
/// instead of the old opaque `COMMAND_RESULT_MISSING`.
fn replay_task_snapshot(record: &control::CommandRecord) -> Result<tasks::TaskSnapshot, String> {
    if let Some(error) = &record.error_code {
        return Err(error.clone());
    }
    serde_json::from_str(
        record
            .result_json
            .as_deref()
            .ok_or_else(|| "COMMAND_IN_PROGRESS".to_string())?,
    )
    .map_err(|error| error.to_string())
}

/// Request-shape errors a Zhihu create can fail with forever — the only ones
/// safe to cache as the claim's terminal result. Everything else (sidecar
/// HTTP/timeout, opaque `response.error` strings, settings/DB/emit failures)
/// is environmental and must release the claim so a retry re-executes.
fn is_deterministic_zhihu_create_error(error: &str) -> bool {
    matches!(error, "INVALID_ZHIHU_PEOPLE_ID" | "INVALID_ZHIHU_TOP_N")
}

fn run_new_zhihu_task(
    control: control::ControlDb,
    request: zhihu::CreateZhihuTaskRequest,
    app: tauri::AppHandle,
    request_id: String,
) -> Result<tasks::TaskSnapshot, String> {
    // Tracks the moment the task actually exists (sidecar row + local event):
    // a failure after that point (emit/DB readback) must still settle the
    // claim with the created snapshot — caching the error would replay it
    // forever, and releasing would make a retry create a duplicate task.
    let mut created: Option<tasks::TaskSnapshot> = None;
    let result = (|| -> Result<tasks::TaskSnapshot, String> {
        let settings = settings::load_settings()?;
        let snapshot = zhihu::create_task(&settings, &request)?;
        created = Some(snapshot.clone());
        let event = control
            .task_events(&snapshot.id, 0, 1)?
            .into_iter()
            .next()
            .ok_or_else(|| "TASK_EVENT_MISSING".to_string())?;
        // Best-effort: the task row already committed — a broadcast failure
        // must not become the command result (the UI poll converges anyway).
        if let Err(error) = app.emit("acquisition://task-event", event) {
            eprintln!("Task event broadcast failed after persistence: {error}");
        }
        Ok(snapshot)
    })();
    match result {
        Ok(snapshot) => {
            let json = serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
            control.complete_command(
                &request_id,
                &json,
                None,
                i64::try_from(snapshot.revision).ok(),
            )?;
            Ok(snapshot)
        }
        Err(error) => {
            if let Some(snapshot) = created {
                let json = serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
                control.complete_command(
                    &request_id,
                    &json,
                    None,
                    i64::try_from(snapshot.revision).ok(),
                )?;
            } else if is_deterministic_zhihu_create_error(&error) {
                control.complete_command(&request_id, "{}", Some(&error), None)?;
            } else {
                control.release_command(&request_id)?;
            }
            Err(error)
        }
    }
}

#[tauri::command]
async fn get_zhihu_login_status() -> Result<zhihu::ZhihuLoginStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let settings = settings::load_settings()?;
        zhihu::login_status(&settings)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn start_zhihu_login() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let settings = settings::load_settings()?;
        zhihu::start_login(&settings)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn clear_zhihu_login() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let settings = settings::load_settings()?;
        zhihu::clear_login(&settings)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn start_zhihu_task(
    task_id: String,
    expected_revision: u64,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = settings::load_settings()?;
        zhihu::start_task(&task_id, expected_revision, &settings, &app)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn control_zhihu_task(
    task_id: String,
    action: String,
    expected_revision: u64,
    request_id: String,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = settings::load_settings()?;
        zhihu::control_task(
            &task_id,
            &action,
            expected_revision,
            &request_id,
            &settings,
            &app,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn restart_podcast_task(
    task_id: String,
    budget_limit_cny: Option<f64>,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    // Heavy copy / publish must not block the UI thread (was causing hard freezes / perceived crashes).
    // No request_id reaches us from the frontend, so the idempotency key is
    // derived from the inputs: a retried click on the same source task
    // replays the stored result instead of cloning a second task (02-F2).
    cache::validate_task_id(&task_id)?;
    let input_hash = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::json!({
                "taskId": task_id,
                "budgetLimitCny": budget_limit_cny,
            }))
            .map_err(|error| error.to_string())?
        )
    );
    let request_id = format!("restart-podcast-task-{input_hash}");
    let (snapshot, kind, superseded_event) = tauri::async_runtime::spawn_blocking(
        move || -> Result<
            (
                tasks::TaskSnapshot,
                Option<podcast::RetryKind>,
                Option<tasks::TaskEvent>,
            ),
            String,
        > {
            let mut locations = storage::StorageLocations::current()?;
            locations.library_root = PathBuf::from(settings::load_settings()?.library_root);
            let mut control = control::ControlDb::open_current()?;
            // Claimed execution path: run the retry, settle the claim at the
            // commit boundary, and report the superseded-source event so the
            // caller can emit it after the claim is recorded.
            let execute = |
                control: &mut control::ControlDb,
                locations: &storage::StorageLocations,
            | -> Result<
                (
                    tasks::TaskSnapshot,
                    Option<podcast::RetryKind>,
                    Option<tasks::TaskEvent>,
                ),
                String,
            > {
                match podcast::retry_task_at(control, locations, &task_id, budget_limit_cny) {
                    Ok((snapshot, kind)) => {
                        // A full restart minted a successor for the same
                        // input — flip the old row's retry affordance off
                        // so repeated clicks cannot spawn parallel re-runs
                        // publishing to the same book id (05-F3).
                        let superseded_event =
                            if matches!(kind, podcast::RetryKind::Restarted) {
                                control.mark_task_superseded(&task_id, &snapshot.id)?
                            } else {
                                None
                            };
                        // Settle at the commit boundary: emit/auto-start
                        // failures after this point must neither wedge the
                        // claim nor make a retry clone a second task.
                        let json =
                            serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
                        control.complete_command(
                            &request_id,
                            &json,
                            None,
                            i64::try_from(snapshot.revision).ok(),
                        )?;
                        Ok((snapshot, Some(kind), superseded_event))
                    }
                    Err(error) => {
                        // TASK_NOT_FOUND is request-stable (task ids are
                        // never recreated). Everything else describes the
                        // *current* state (TASK_NOT_RETRYABLE can clear
                        // once the task goes terminal) or the environment
                        // (copy/publish IO) — release so a retry executes.
                        if error == "TASK_NOT_FOUND" {
                            control.complete_command(
                                &request_id,
                                "{}",
                                Some(&error),
                                None,
                            )?;
                        } else {
                            control.release_command(&request_id)?;
                        }
                        Err(error)
                    }
                }
            };
            match control.claim_command(&request_id, "restart_podcast_task", &input_hash)? {
                control::CommandClaim::Existing(record) => {
                    match replay_task_snapshot(&record) {
                        Ok(snapshot) => {
                            // A Resumed retry keeps the same task id, so a
                            // later failure puts the source row back into a
                            // retryable terminal state under the SAME derived
                            // request id — replaying the stored Queued
                            // snapshot then would swallow the new retry
                            // without running anything. Reopen the settled
                            // claim and re-execute (the create_zhihu_task
                            // pattern); while the revived task is still live
                            // or was superseded by a successor, the stored
                            // result is the honest replay.
                            let retryable_again = control
                                .task_snapshot(&task_id)?
                                .map(|current| {
                                    current.lifecycle_state
                                        == tasks::LifecycleState::Terminal
                                        && current.can_retry
                                })
                                .unwrap_or(false);
                            if !retryable_again {
                                Ok((snapshot, None, None))
                            } else if control.reopen_completed_command(&request_id)? {
                                execute(&mut control, &locations)
                            } else {
                                // Lost the reopen race — replay whatever the
                                // winning caller is producing/stored.
                                match control.claim_command(
                                    &request_id,
                                    "restart_podcast_task",
                                    &input_hash,
                                )? {
                                    control::CommandClaim::Existing(record) => {
                                        replay_task_snapshot(&record)
                                            .map(|snapshot| (snapshot, None, None))
                                    }
                                    control::CommandClaim::New => {
                                        execute(&mut control, &locations)
                                    }
                                }
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
                control::CommandClaim::New => execute(&mut control, &locations),
            }
        },
    )
    .await
    .map_err(|error| format!("RETRY_JOIN_FAILED: {error}"))??;

    let Some(kind) = kind else {
        // Claim replay — the original run already emitted and auto-started.
        return Ok(snapshot);
    };
    // The superseded source row changed too — push its event so the task list
    // drops the stale "重试" affordance without waiting for the next refresh.
    if let Some(event) = superseded_event {
        let _ = app.emit(podcast::TASK_EVENT_NAME, event);
    }
    // Emit the latest event for the returned snapshot (republish or new queued
    // task), then auto-start — DB reads and worker spawn stay on a blocking
    // worker, not the async runtime's IPC-facing threads.
    let app_for_work = app.clone();
    let snapshot_for_work = snapshot.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        if let Ok(control) = control::ControlDb::open_current() {
            let after = snapshot_for_work.last_sequence.saturating_sub(1);
            if let Ok(events) = control.task_events(&snapshot_for_work.id, after, 1) {
                if let Some(event) = events.into_iter().next() {
                    let _ = app_for_work.emit(podcast::TASK_EVENT_NAME, event);
                }
            }
        }
        // Restart/requeue leaves a queued task — auto-start transcription
        // immediately (a Resumed task continues from its own checkpoint).
        if matches!(
            kind,
            podcast::RetryKind::Restarted | podcast::RetryKind::Resumed
        ) {
            if let Err(error) =
                podcast::start_task(snapshot_for_work.id.clone(), app_for_work.clone())
            {
                // Surface as soft error string rather than panicking the command.
                return Err(format!(
                    "任务已排队但自动开始失败：{error}。请在任务列表点击「开始」。"
                ));
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(snapshot)
}

#[tauri::command]
async fn open_task_result(task_id: String) -> Result<library::BookDetail, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<library::BookDetail, String> {
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
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn control_podcast_task(
    task_id: String,
    action: String,
    expected_revision: u64,
    request_id: String,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    // Claim check, worker suspend/kill and DB writes all block — off IPC thread.
    tauri::async_runtime::spawn_blocking(move || -> Result<tasks::TaskSnapshot, String> {
        if request_id.trim().is_empty() {
            return Err("INVALID_REQUEST_ID".to_string());
        }
        // P-11-F09: the task id is spliced into the discard-intent marker
        // path below — validate it before it ever reaches a `join`, not
        // just inside the worker layer.
        cache::validate_task_id(&task_id)?;
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
                    if !matches!(
                        action.as_str(),
                        "pause" | "resume" | "cancel" | "cancel_and_discard"
                    ) {
                        return Err("INVALID_TASK_CONTROL".to_string());
                    }
                    // P2-11: durable intent BEFORE process side effects —
                    // a crash between "worker killed" and "DB updated" used
                    // to leave the snapshot claiming the task still ran.
                    // For cancel_and_discard the sibling marker is written
                    // first too, so even a crash mid-discard stays
                    // recoverable via reconcile_cancel_and_discard.
                    let discard_locations = if action == "cancel_and_discard" {
                        let locations = storage::StorageLocations::current()?;
                        atomic_file::write(&discard_intent_path(&locations, &task_id), b"pending")?;
                        Some(locations)
                    } else {
                        None
                    };
                    let event = match control.control_task(&task_id, &action, expected_revision) {
                        Ok(event) => event,
                        Err(error) => {
                            if let Some(locations) = &discard_locations {
                                let _ = fs::remove_file(discard_intent_path(locations, &task_id));
                            }
                            return Err(error);
                        }
                    };
                    let side_effect = match action.as_str() {
                        "pause" => podcast::pause_task(&task_id),
                        "resume" => podcast::resume_task(&task_id),
                        "cancel" | "cancel_and_discard" => {
                            match podcast::cancel_task(&task_id) {
                                // Worker already gone → the recorded
                                // terminal intent is the truth.
                                Err(error) if error == "WORKER_NOT_RUNNING" => Ok(()),
                                other => other,
                            }
                        }
                        _ => unreachable!(),
                    };
                    if let Err(error) = side_effect {
                        // Compensating transition: pause↔resume restore the
                        // worker's real state so the DB stops lying about
                        // it. For cancel the recorded intent stands — the
                        // user did cancel; the Job Object and the
                        // interrupted-task reaper reap any orphaned worker,
                        // and a cancel_and_discard marker keeps the
                        // pending discard recoverable.
                        let inverse = match action.as_str() {
                            "pause" => Some("resume"),
                            "resume" => Some("pause"),
                            _ => None,
                        };
                        if let Some(inverse) = inverse {
                            let _ =
                                control.control_task(&task_id, inverse, event.snapshot.revision);
                        }
                        return Err(error);
                    }
                    if let Some(locations) = &discard_locations {
                        if let Err(error) = cache::discard_podcast_task_at(locations, &task_id) {
                            // Marker stays → the reconcile sweep retries;
                            // the intent is durable even though we report
                            // the failure.
                            return Err(format!("TASK_DISCARD_PENDING: {error}"));
                        }
                        let _ = fs::remove_file(discard_intent_path(locations, &task_id));
                    }
                    // Emit is best-effort: the state change already committed,
                    // so a broadcast failure must not poison the command
                    // result (the UI's snapshot poll converges anyway).
                    if let Err(error) = app.emit(podcast::TASK_EVENT_NAME, &event) {
                        eprintln!("Task event broadcast failed after persistence: {error}");
                    }
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
                        // Transient conflicts (revision/sequence races)
                        // are not terminal results — release the claim so
                        // an immediate retry re-executes cleanly instead
                        // of replaying a cached failure. Same for
                        // TASK_DISCARD_PENDING: the discard intent marker
                        // stays on disk and the reconcile sweep retries
                        // it — the failure describes the current attempt,
                        // not a terminal property of the request (05-F10).
                        if zhihu::is_transient_persist_conflict(&error)
                            || error.starts_with("TASK_DISCARD_PENDING")
                        {
                            control.release_command(&request_id)?;
                        } else {
                            control.complete_command(&request_id, "{}", Some(&error), None)?;
                        }
                        Err(error)
                    }
                }
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Fail-closed startup aborts: log to app.log first so the exit reason
    // survives on disk, not just on a stderr nobody reads (12-F8).
    if let Err(error) = tls::ensure_crypto_provider() {
        storage::app_log(
            "startup",
            &format!("TLS crypto provider init failed: {error}"),
        );
        panic!("failed to initialize the TLS crypto provider: {error}");
    }
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        if let Some(file_path) = initial_markdown_path(&args) {
            register_opened_markdown(Path::new(&file_path));
            let _ = app.emit("open-file", file_path);
        }
    }));
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let app = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
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
            get_app_settings,
            get_storage_locations,
            get_storage_usage,
            reveal_storage_directory,
            create_state_backup,
            list_state_backups,
            restore_state_backup,
            take_control_db_recovery_notice,
            update_app_settings,
            clear_safe_cache,
            get_secret_status,
            set_deepseek_api_key,
            delete_deepseek_api_key,
            get_publish_recovery_status,
            recover_publish_transactions,
            preview_legacy_migration,
            get_migration_runs,
            execute_settings_migration,
            migrate_sqlite_verified,
            reconcile_zhihu_archive,
            get_acquisition_snapshot,
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
            cancel_exit_fallback,
            cancel_and_discard,
            start_podcast_task,
            create_zhihu_task,
            get_zhihu_login_status,
            start_zhihu_login,
            clear_zhihu_login,
            start_zhihu_task,
            control_zhihu_task,
            restart_podcast_task,
            open_task_result,
            control_podcast_task,
        ])
        .manage(podcast::PodcastPreviewStore::default())
        .manage(std::sync::Arc::new(
            reader_server::ReaderServiceState::default(),
        ))
        .setup(|app| {
            storage::app_log(
                "app",
                &format!("immersive-reader v{} starting", env!("CARGO_PKG_VERSION")),
            );
            // P1-1: Job Objects kill podcast workers when the app exits, but
            // their tasks stayed Running forever with no recovery path. At
            // startup every still-active task whose worker is gone gets marked
            // Interrupted (restartable); the live-worker id set comes from the
            // worker registry so a just-started task is never clobbered.
            // These run synchronously inside setup: a background thread races
            // the webview's first invoke (a fast `import_markdown_folder` call
            // could watch its `.incoming` staging get swept mid-import).
            if let Ok(control) = control::ControlDb::open_current() {
                let active_ids = podcast::active_podcast_task_ids().unwrap_or_default();
                if let Err(error) = control.recover_interrupted_tasks(&active_ids) {
                    eprintln!("interrupted podcast task recovery failed: {error}");
                }
            }
            // P2-19: sweep abandoned import staging and P2-11 discard
            // markers left by a crash — before the UI can start new work.
            if let Ok(locations) = storage::StorageLocations::current_with_library_settings() {
                importer::sweep_staging_dirs(&locations.library_root);
                // Orphaned `.incoming` staging and over-cap `.revisions`
                // slots — residue a deleted journal can never reclaim.
                publish::sweep_publish_residue(&locations.library_root);
                // P-10-F1: a publish that crashed mid-commit left a book
                // wedged until the user found the recovery UI. Advance every
                // non-terminal journal to its converged state now — recovery
                // is deterministic (journal phase decides the direction) and
                // per-transaction tolerant so one bad journal cannot block
                // the rest.
                if let Ok(transactions) = publish::list_transactions(&locations.library_root) {
                    for transaction in transactions {
                        if matches!(
                            transaction.phase,
                            publish::PublishPhase::Committed | publish::PublishPhase::RolledBack
                        ) {
                            continue;
                        }
                        if let Err(error) = publish::recover_transaction(
                            &locations.library_root,
                            &transaction.transaction_id,
                        ) {
                            eprintln!(
                                "startup publish recovery: {} failed: {error}",
                                transaction.transaction_id
                            );
                        }
                    }
                }
                // Marker sweep + publish-index reconcile share one control
                // handle — the marker sweep consults task liveness before
                // deleting anything.
                if let Ok(control) = control::ControlDb::open_current() {
                    reconcile_discard_markers(&locations, &control);
                    // P-11-F08: the control.db publish index is only a
                    // shortlist — reconcile it against the journals on disk so
                    // recovered/swept transactions stop resurfacing and stale
                    // 'prepared' rows left by a crashed writer get their real
                    // terminal phase.
                    if let Err(error) =
                        control.reconcile_publish_transaction_index(&locations.library_root)
                    {
                        eprintln!("publish index reconcile failed: {error}");
                    }
                }
            }
            // 17-F3: the snapshot-path reap only runs when the UI asks for a
            // snapshot — a wedged worker could display "Running" for as long
            // as the user left the window alone. Sweep on a fixed cadence and
            // push the events so the row turns Interrupted without waiting
            // for a refresh trigger.
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(PODCAST_WORKER_REAP_INTERVAL);
                    let reaped = (|| -> Result<Vec<tasks::TaskEvent>, String> {
                        let control = control::ControlDb::open_current()?;
                        let locations = storage::StorageLocations::current_with_library_settings()?;
                        let work_root = locations.cache_root.join("Podcast").join("Tasks");
                        control.reap_stale_workers(&work_root, PODCAST_WORKER_STALE_AFTER)
                    })();
                    match reaped {
                        Ok(events) => {
                            for event in events {
                                let _ = handle.emit(podcast::TASK_EVENT_NAME, event);
                            }
                        }
                        Err(error) => {
                            eprintln!("periodic stale-worker reap failed: {error}")
                        }
                    }
                });
            }
            // Windows: file path passed as CLI argument
            let Some(window) = app.get_webview_window("main") else {
                return Err(std::io::Error::other("main webview window missing").into());
            };
            let args: Vec<String> = std::env::args().collect();
            if let Some(file_path) = initial_markdown_path(&args) {
                register_opened_markdown(Path::new(&file_path));
                if let Some(script) = initial_file_eval_script(&file_path) {
                    let _ = window.eval(script);
                }
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
        .unwrap_or_else(|error| {
            storage::app_log(
                "startup",
                &format!("tauri application build failed: {error}"),
            );
            panic!("error while building tauri application: {error}");
        });

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
                        register_opened_markdown(&path);
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
    use super::{
        book_asset_root, initial_file_eval_script, initial_markdown_path, is_markdown_path,
        reconcile_discard_markers,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scoped_temp_dir(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "immersive-lib-test-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn book_asset_root_covers_chapters_and_assets_but_not_outside() {
        // P2-44: a chapter under `chapters/` resolves to the manifest-bearing
        // book dir, so `assets/` siblings and `../img.png`-style references
        // inside the book keep working. Files outside the Library — and
        // references escaping the book dir (`../../`) — stay ungranted.
        let dir = scoped_temp_dir("asset-scope");
        let library = dir.join("library");
        let book = library.join("手动").join("书");
        fs::create_dir_all(book.join("chapters")).unwrap();
        fs::write(book.join("manifest.json"), "{}").unwrap();
        let chapter = book.join("chapters").join("01.md");
        fs::write(&chapter, "x").unwrap();
        let canonical_library = library.canonicalize().unwrap();

        assert_eq!(
            book_asset_root(&chapter.canonicalize().unwrap(), &canonical_library),
            Some(book.canonicalize().unwrap())
        );

        let outside = dir.join("elsewhere.md");
        fs::write(&outside, "x").unwrap();
        assert_eq!(
            book_asset_root(&outside.canonicalize().unwrap(), &canonical_library),
            None
        );

        let loose = library.join("loose.md");
        fs::write(&loose, "x").unwrap();
        assert_eq!(
            book_asset_root(&loose.canonicalize().unwrap(), &canonical_library),
            None
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reconcile_discard_markers_retries_pending_discards() {
        // P2-11: a leftover `<task>.discard-pending` marker re-triggers the
        // cache discard; sibling task dirs without a marker are untouched.
        let dir = scoped_temp_dir("discard-markers");
        let tasks_dir = dir.join("Cache").join("Podcast").join("Tasks");
        let doomed = tasks_dir.join("task-doomed");
        let kept = tasks_dir.join("task-kept");
        fs::create_dir_all(&doomed).unwrap();
        fs::create_dir_all(&kept).unwrap();
        fs::write(doomed.join("partial.bin"), "x").unwrap();
        fs::write(kept.join("partial.bin"), "x").unwrap();
        fs::write(tasks_dir.join("task-doomed.discard-pending"), b"pending").unwrap();

        let locations = crate::storage::StorageLocations {
            channel: "test".to_string(),
            settings_path: dir.join("settings.json"),
            data_root: dir.join("Data"),
            cache_root: dir.join("Cache"),
            logs_root: dir.join("Logs"),
            runtime_state_root: dir.join("RuntimeState"),
            backups_root: dir.join("Backups"),
            library_root: dir.join("Library"),
            runtime_root: dir.join("Runtime"),
        };
        // The marker task has no control.db row — that is the "task row gone"
        // case, which must still discard.
        let control =
            crate::control::ControlDb::open(&dir.join("Data").join("App").join("control.db"))
                .unwrap();
        reconcile_discard_markers(&locations, &control);

        assert!(!doomed.exists());
        assert!(!tasks_dir.join("task-doomed.discard-pending").exists());
        assert!(kept.join("partial.bin").exists());
        // Windows holds control.db open until the connection drops.
        drop(control);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reconcile_discard_markers_keeps_live_task_cache() {
        // 02-F8: a marker stranded next to a task that is still non-terminal
        // must not delete that task's cache mid-run.
        let dir = scoped_temp_dir("discard-markers-live");
        let tasks_dir = dir.join("Cache").join("Podcast").join("Tasks");
        let live = tasks_dir.join("task-live");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("partial.bin"), "x").unwrap();
        fs::write(tasks_dir.join("task-live.discard-pending"), b"pending").unwrap();

        let locations = crate::storage::StorageLocations {
            channel: "test".to_string(),
            settings_path: dir.join("settings.json"),
            data_root: dir.join("Data"),
            cache_root: dir.join("Cache"),
            logs_root: dir.join("Logs"),
            runtime_state_root: dir.join("RuntimeState"),
            backups_root: dir.join("Backups"),
            library_root: dir.join("Library"),
            runtime_root: dir.join("Runtime"),
        };
        let control =
            crate::control::ControlDb::open(&dir.join("Data").join("App").join("control.db"))
                .unwrap();
        let now = "2026-07-11T12:00:00Z".to_string();
        control
            .persist_task_event(&crate::tasks::TaskEvent {
                schema_version: 1,
                task_id: "task-live".to_string(),
                sequence: 1,
                revision: 1,
                event_type: "created".to_string(),
                created_at: now.clone(),
                snapshot: crate::tasks::TaskSnapshot {
                    id: "task-live".to_string(),
                    kind: crate::tasks::TaskKind::Podcast,
                    revision: 1,
                    last_sequence: 1,
                    lifecycle_state: crate::tasks::LifecycleState::Running,
                    outcome: crate::tasks::TaskOutcome::None,
                    required_action: crate::tasks::RequiredAction::None,
                    progress: crate::tasks::TaskProgress {
                        mode: crate::tasks::ProgressMode::Determinate,
                        percent: Some(0.0),
                        completed_units: Some(0),
                        total_units: Some(1),
                        label: None,
                        unit: None,
                        source_total_units: None,
                        skipped_units: None,
                    },
                    error_code: None,
                    error_message: None,
                    retry_after_seconds: None,
                    engine_stage: "transcribe".to_string(),
                    engine_status: "working".to_string(),
                    recoverable: true,
                    can_pause: true,
                    can_resume: false,
                    can_retry: false,
                    can_cancel: true,
                    book_id: None,
                    source_id: None,
                    display_name: None,
                    cache_lease_bytes: 0,
                    created_at: now.clone(),
                    updated_at: now,
                    last_heartbeat_at: None,
                    checkpoint_at: None,
                },
            })
            .unwrap();
        reconcile_discard_markers(&locations, &control);

        assert!(live.join("partial.bin").exists());
        assert!(tasks_dir.join("task-live.discard-pending").exists());
        drop(control);
        fs::remove_dir_all(dir).unwrap();
    }

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

    #[test]
    fn initial_file_eval_script_stays_a_json_string_literal() {
        // P3-23 regression: `window.__INITIAL_FILE__` must be injected as a
        // serde_json string literal — a path with quotes/backslashes would
        // break out of a naive `"{path}"` interpolation inside eval'd JS.
        let hostile = r#"C:\weird "quoted"\backslash\file.md"#;
        let script = initial_file_eval_script(hostile).expect("path serialization must succeed");
        assert!(script.contains(r#""quoted\""#));
        // The injected literal decodes back to exactly the original path.
        let literal = script
            .trim_start_matches("window.__INITIAL_FILE__ = ")
            .trim_end_matches(';');
        let decoded: String =
            serde_json::from_str(literal).expect("injected literal must be valid JSON");
        assert_eq!(decoded, hostile);
        assert!(initial_file_eval_script("plain.md").is_some());
    }
}
