use encoding_rs::{GB18030, UTF_16BE, UTF_16LE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{Emitter, Manager};
mod atomic_file;
pub mod cache;
mod contracts;
pub mod control;
mod duplicates;
mod epub;
mod import_ops;
mod importer;
#[cfg(windows)]
pub mod job_object;
mod library;
pub mod migration;
mod open_inputs;
mod progress;
pub mod publish;
mod reader_db;
mod reader_http;
#[cfg(test)]
mod reader_ipc_tests;
mod reader_preferences;
mod reader_server;
mod secrets;
mod settings;
mod startup;
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
    #[cfg(target_os = "android")]
    {
        // The legacy mmbook state dir only ever existed on desktop installs,
        // and `dirs` returns nothing on Android anyway. Point at an inert
        // app-private path instead of the process cwd so the `exists()`
        // lookups below simply find nothing.
        return storage::android_base_dirs()
            .map(|base| base.app_local_data.join("legacy-mmbook-none"))
            .unwrap_or_else(|_| PathBuf::from("/data/local/tmp/immersive-reader-legacy-none"));
    }
    #[cfg(not(target_os = "android"))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mmbook")
    }
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
/// How often the background sweep runs — the snapshot-path reap alone meant
/// a wedged worker survived indefinitely whenever the user never touched the
/// task list.

/// Android cold start: the deferred residue sweep waits this long after
/// `setup` returns so it can never compete with the webview's first paint or
/// the frontend's first invokes. This is a background thread started by
/// `run()`, not a timer the UI waits on.
#[cfg(target_os = "android")]
const ANDROID_DEFERRED_MAINTENANCE_DELAY: Duration = Duration::from_secs(3);

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
/// Returns the normalized path plus the size the metadata check just observed
/// (the read pre-sizes its buffer from it; the `take()` cap still guards a
/// file swapped in between).
fn markdown_read_target(path: &str) -> Result<(PathBuf, u64), String> {
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
    Ok((normalized, metadata.len()))
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

/// Non-Markdown counterpart of `initial_markdown_path` — a second-instance
/// argv tail naming an existing file (`.epub`, `.zip`, folders' files…)
/// goes to the pending-open queue for `begin_import` instead of `open-file`.
/// Requiring `is_file` keeps random launcher flags out of the queue.
fn initial_file_path(args: &[String]) -> Option<String> {
    if args.len() <= 1 {
        return None;
    }
    let exact = args[1].clone();
    if !exact.starts_with('-') && std::path::Path::new(&exact).is_file() {
        return Some(exact);
    }
    let joined = args[1..].join(" ");
    if std::path::Path::new(&joined).is_file() {
        return Some(joined);
    }
    None
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

/// Android-only: the ContentReaderPlugin Kotlin class registered via the
/// `content-reader` plugin in `run()` below.
#[cfg(target_os = "android")]
struct ContentReaderHandle(tauri::plugin::PluginHandle<tauri::Wry>);

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct StagedContentFile {
    path: String,
    name: String,
}

/// Android SAF picks hand back `content://` URIs that `std::fs` cannot open.
/// The fs plugin's fd path panics (`unimplemented!()`) when a provider
/// returns a null descriptor, leaving the JS invoke pending forever — our
/// own plugin streams the bytes through ContentResolver.openInputStream into
/// an app-private staging dir instead, with real errors on failure.
#[tauri::command]
async fn stage_content_uri(
    app: tauri::AppHandle,
    uri: String,
    dest_dir: String,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        return tauri::async_runtime::spawn_blocking(
            move || -> Result<serde_json::Value, String> {
                use tauri::Manager;
                // The renderer chooses the staging dir name but the write
                // must stay inside the app cache — never arbitrary storage.
                let cache = app
                    .path()
                    .app_cache_dir()
                    .map_err(|e| format!("app cache dir unavailable: {e}"))?;
                if !Path::new(&dest_dir).starts_with(&cache) {
                    return Err("STAGE_DIR_OUTSIDE_CACHE".to_string());
                }
                let handle = app
                    .try_state::<ContentReaderHandle>()
                    .ok_or_else(|| "content reader plugin unavailable".to_string())?;
                let staged = handle
                    .0
                    .run_mobile_plugin::<StagedContentFile>(
                        "copyToDir",
                        serde_json::json!({ "uri": uri, "dir": dest_dir }),
                    )
                    .map_err(|error| format!("failed to open file: {error}"))?;
                Ok(serde_json::json!({ "path": staged.path, "name": staged.name }))
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, uri, dest_dir);
        Err("stage_content_uri is only supported on Android".to_string())
    }
}

#[tauri::command]
async fn read_markdown_file(app: tauri::AppHandle, path: String) -> Result<ReadResult, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<ReadResult, String> {
        let (path, len) = markdown_read_target(&path)?;
        // P2-18: the metadata check above can be raced — cap the read itself
        // so a swapped-in file can never slurp more than the limit + 1 byte.
        let bytes = {
            let file = fs::File::open(&path).map_err(|e| e.to_string())?;
            // Pre-size from the checked metadata: a large Markdown otherwise
            // reallocates through several Vec growth steps before decode.
            let mut bytes = Vec::with_capacity(len as usize);
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
    crate::atomic_file::read_to_string(&normalized).map_err(|error| error.to_string())
}

#[tauri::command]
async fn load_reading_state(path: String) -> Result<ReadingState, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<ReadingState, String> {
        let sp = state_path_for(&path);
        atomic_file::recover_file(&sp).map_err(|error| error.to_string())?;
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
        // `mut` only under Android (staging dirs fold into the total there).
        #[allow(unused_mut)]
        let mut cache_bytes = directory_size(&locations.cache_root)?;
        // Android: SAF pick staging sits in the private app cache, outside
        // `Cache\` — count it so the usage bar is honest on mobile.
        #[cfg(target_os = "android")]
        if let Ok(base) = storage::android_base_dirs() {
            for dir in temporary_content::android_staging_dirs(&base.app_cache) {
                cache_bytes = cache_bytes.saturating_add(directory_size(&dir)?);
            }
        }
        Ok(StorageUsage {
            library_bytes: directory_size(&locations.library_root)?,
            data_bytes: directory_size(&locations.data_root)?,
            cache_bytes,
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
        let stamp = crate::atomic_file::read_to_string(path.join("backup-manifest.json"))
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
        let manifest = crate::atomic_file::read_to_string(path.join("backup-manifest.json"))
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
    let manifest_raw = crate::atomic_file::read_to_string(backup_dir.join("backup-manifest.json"))
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
    // Shelf/resume snapshots are read-only. Explicit archival actions own
    // engine startup; merely opening the reader must not spawn a service.
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
        let control = control::ControlDb::open_current()?;
        let mut tasks = control.task_snapshots(Some(tasks::TaskKind::Zhihu))?;
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
async fn repair_library_duplicates(apply: bool) -> Result<duplicates::RepairReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        startup::wait();
        let value = settings::load_settings()?;
        duplicates::repair(Path::new(&value.library_root), apply)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn load_library_snapshot() -> Result<Option<library::LibraryScan>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let value = settings::load_settings()?;
        Ok(library::load_snapshot(Path::new(&value.library_root)))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn scan_library() -> Result<library::LibraryScan, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let value = settings::load_settings()?;
        let root = Path::new(&value.library_root);
        let scan = library::scan_library(root)?;
        library::save_snapshot(root, &scan);
        Ok(scan)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn open_book(app: tauri::AppHandle, book_id: String) -> Result<library::BookDetail, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (detail, book_root) = open_book_detail_at(&book_id)?;
        // The whole book dir is granted recursively so chapter assets
        // (`assets/*`, EPUB `content/*`, covers) resolve via convertFileSrc.
        let _ = app.asset_protocol_scope().allow_directory(&book_root, true);
        Ok(detail)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn open_book_detail(book_id: &str) -> Result<library::BookDetail, String> {
    open_book_detail_at(book_id).map(|(detail, _)| detail)
}

fn open_book_detail_at(book_id: &str) -> Result<(library::BookDetail, PathBuf), String> {
    let value = settings::load_settings()?;
    // One `find_book` pass returns both the detail and the resolved book root
    // (the command needs the root for the asset-scope grant). Calling
    // `book_context` + `open_book` separately re-read/re-parsed the manifest
    // and .reading.json and re-opened reader.db for every book open.
    let (mut detail, book_root) =
        library::open_book_with_root(Path::new(&value.library_root), book_id)?;
    // Task records are advisory — a busy/missing control.db must not fail a
    // book open; it degrades to an empty list like unreadable provenance.
    detail.task_records = control::ControlDb::open_current()
        .and_then(|control| control.task_snapshots_for_book(&detail.manifest.book_id))
        .unwrap_or_else(|error| {
            storage::app_log(
                "library",
                &format!("open_book: task records unavailable for {book_id}: {error}"),
            );
            Vec::new()
        });
    Ok((detail, book_root))
}

#[tauri::command]
async fn get_book_chapter_path(
    app: tauri::AppHandle,
    book_id: String,
    chapter_id: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let value = settings::load_settings()?;
        let path = library::chapter_path(Path::new(&value.library_root), &book_id, &chapter_id)?;
        // Same recursive grant as open_book — the chapter's relative
        // resources resolve against the book root's scope.
        if let (Ok(canonical_file), Ok(canonical_library)) = (
            path.canonicalize(),
            Path::new(&value.library_root).canonicalize(),
        ) {
            if let Some(book_root) = book_asset_root(&canonical_file, &canonical_library) {
                let _ = app.asset_protocol_scope().allow_directory(book_root, true);
            }
        }
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
        startup::wait();
        let value = settings::load_settings()?;
        importer::import_markdown_folder(Path::new(&path), Path::new(&value.library_root))
    })
    .await
    .map_err(|error| error.to_string())?
}

// ===== Import operations (begin/status/cancel) =========================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportRequest {
    /// "directory" | "files" | "uris" | "archive".
    kind: String,
    /// `directory`/`archive`: the source path.
    path: Option<String>,
    /// `files`: absolute paths of picked files.
    paths: Option<Vec<String>>,
    /// `uris`: Android `content://` URIs to stage via the plugin.
    uris: Option<Vec<String>>,
    /// Optional caller-chosen book title (staged dirs have opaque names).
    title: Option<String>,
}

#[tauri::command]
async fn begin_import(
    app: tauri::AppHandle,
    request: ImportRequest,
) -> Result<import_ops::ImportOperation, String> {
    tauri::async_runtime::spawn_blocking(move || {
        startup::wait();
        // `app` only feeds the Android `uris` arm — mark it used elsewhere.
        let _ = &app;
        let locations = storage::StorageLocations::current_with_library_settings()?;
        let title = request.title.clone();
        let label = title.clone().unwrap_or_else(|| "导入".to_string());
        match request.kind.as_str() {
            "directory" => {
                let path = request
                    .path
                    .clone()
                    .ok_or_else(|| "IMPORT_REQUEST_MISSING_PATH".to_string())?;
                let source = atomic_file::long_path(Path::new(&path));
                if !source.is_dir() {
                    return Err("IMPORT_SOURCE_NOT_DIRECTORY".to_string());
                }
                let total = import_ops::count_importable_files(&source);
                import_ops::begin("directory", &label, move |progress| {
                    progress.set_total(total);
                    open_inputs::import_directory(&source, title.as_deref(), &locations, progress)
                })
            }
            "files" => {
                let paths = request
                    .paths
                    .clone()
                    .filter(|paths| !paths.is_empty())
                    .ok_or_else(|| "IMPORT_REQUEST_MISSING_PATHS".to_string())?;
                import_ops::begin("files", &label, move |progress| {
                    open_inputs::import_batch(
                        &paths
                            .iter()
                            .map(|path| atomic_file::long_path(Path::new(path)))
                            .collect::<Vec<_>>(),
                        &locations,
                        progress,
                    )
                })
            }
            "uris" => {
                let uris = request
                    .uris
                    .clone()
                    .filter(|uris| !uris.is_empty())
                    .ok_or_else(|| "IMPORT_REQUEST_MISSING_URIS".to_string())?;
                #[cfg(not(target_os = "android"))]
                {
                    let _ = (uris, label, locations, title);
                    Err("IMPORT_KIND_UNSUPPORTED".to_string())
                }
                #[cfg(target_os = "android")]
                {
                    let app = app.clone();
                    import_ops::begin("uris", &label, move |progress| {
                        import_ops::run_with_staging(&locations, progress.op_id(), |staging| {
                            let handle = app
                                .try_state::<ContentReaderHandle>()
                                .ok_or_else(|| "content reader plugin unavailable".to_string())?;
                            progress.set_total(uris.len() as u32);
                            for uri in &uris {
                                if progress.is_cancelled() {
                                    return Err(importer::IMPORT_CANCELLED.to_string());
                                }
                                handle
                                    .0
                                    .run_mobile_plugin::<StagedContentFile>(
                                        "copyToDir",
                                        serde_json::json!({
                                            "uri": uri,
                                            "dir": staging.to_string_lossy(),
                                        }),
                                    )
                                    .map_err(|error| format!("内容 URI 读取失败：{error}"))?;
                                progress.add_done(1);
                            }
                            import_ops::route_staged(
                                staging,
                                title.as_deref(),
                                &locations,
                                progress,
                            )
                        })
                    })
                }
            }
            "archive" => {
                let path = request
                    .path
                    .clone()
                    .ok_or_else(|| "IMPORT_REQUEST_MISSING_PATH".to_string())?;
                let source = atomic_file::long_path(Path::new(&path));
                if !source.is_file() {
                    return Err("IMPORT_SOURCE_NOT_FILE".to_string());
                }
                import_ops::begin("archive", &label, move |progress| {
                    import_ops::run_with_staging(&locations, progress.op_id(), |staging| {
                        let lower = path.to_ascii_lowercase();
                        if lower.ends_with(".epub") {
                            return open_inputs::import_one_file(
                                &source,
                                title.as_deref(),
                                &locations,
                                progress,
                            );
                        }
                        if lower.ends_with(".zip") || lower.ends_with(".cbz") {
                            let expanded = import_ops::expand_archive(&source, staging)?;
                            return import_ops::route_staged(
                                &expanded,
                                title.as_deref(),
                                &locations,
                                progress,
                            );
                        }
                        Err("IMPORT_ARCHIVE_UNSUPPORTED".to_string())
                    })
                })
            }
            _ => Err("IMPORT_KIND_UNSUPPORTED".to_string()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_import_status(op_id: String) -> Result<Option<import_ops::ImportOperation>, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(import_ops::status(&op_id)))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn cancel_import(op_id: String) -> Result<import_ops::ImportOperation, String> {
    tauri::async_runtime::spawn_blocking(move || import_ops::cancel(&op_id))
        .await
        .map_err(|error| error.to_string())?
}

// ===== Platform capabilities ===========================================

/// What this build can actually do — the frontend hides entries whose flag
/// is false rather than discovering failures at invoke time.
///
/// The first six fields are the original 1.2.x wire shape and stay for older
/// frontends; the frontend normalizer accepts both snake_case and camelCase
/// and falls back per-field, so added fields are additive. `isMobile`,
/// `taskAcquisition`, `directoryImport`, `secretStore`, `markdownEditing` and
/// `supportedFormats` are the mobile-aware flags (the Android shell gates the
/// Zhihu/podcast/deepseek surfaces on them).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformCapabilities {
    platform: &'static str,
    /// The tiny_http 连读 surface exists on desktop only.
    reader_session: bool,
    /// SAF `content://` staging via the content-reader plugin.
    content_uri_import: bool,
    /// EPUB import + chapter serving.
    epub: bool,
    /// Volume-key page turning (Android hardware keys).
    volume_key_capture: bool,
    /// Reading bundle export/import (zip).
    reading_bundle: bool,
    /// Running in a phone/tablet shell (touch-first layout).
    is_mobile: bool,
    /// Zhihu/podcast acquisition engines (managed runtimes + control DB).
    task_acquisition: bool,
    /// Filesystem directory picking + folder import.
    directory_import: bool,
    /// Credential Manager (desktop) / Keystore (retired on Android) store.
    secret_store: bool,
    /// In-place Markdown editing (`save_markdown_file`).
    markdown_editing: bool,
    /// Formats the importer accepts on this platform.
    supported_formats: Vec<&'static str>,
}

/// Formats the desktop importer/reader accepts (Markdown + text chapters,
/// EPUB, and reading-bundle `.zip`). Android drops archive imports.
#[cfg(not(target_os = "android"))]
const DESKTOP_SUPPORTED_FORMATS: [&str; 5] = ["md", "markdown", "txt", "epub", "zip"];
/// Android imports Markdown/text chapters and EPUB through SAF; `.zip`/`.cbz`
/// archives are not offered by the mobile shell.
#[cfg(target_os = "android")]
const ANDROID_SUPPORTED_FORMATS: [&str; 4] = ["md", "markdown", "txt", "epub"];

#[tauri::command]
fn get_platform_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "android")]
    {
        return PlatformCapabilities {
            platform: "android",
            reader_session: false,
            content_uri_import: true,
            epub: true,
            volume_key_capture: true,
            reading_bundle: true,
            is_mobile: true,
            task_acquisition: false,
            directory_import: false,
            secret_store: false,
            markdown_editing: false,
            supported_formats: ANDROID_SUPPORTED_FORMATS.to_vec(),
        };
    }
    #[cfg(target_os = "windows")]
    {
        PlatformCapabilities {
            platform: "windows",
            reader_session: true,
            content_uri_import: false,
            epub: true,
            volume_key_capture: false,
            reading_bundle: true,
            is_mobile: false,
            task_acquisition: true,
            directory_import: true,
            secret_store: true,
            markdown_editing: true,
            supported_formats: DESKTOP_SUPPORTED_FORMATS.to_vec(),
        }
    }
    #[cfg(not(any(target_os = "android", target_os = "windows")))]
    PlatformCapabilities {
        platform: "other",
        reader_session: true,
        content_uri_import: false,
        epub: true,
        volume_key_capture: false,
        reading_bundle: true,
        is_mobile: false,
        task_acquisition: true,
        directory_import: true,
        secret_store: true,
        markdown_editing: true,
        supported_formats: DESKTOP_SUPPORTED_FORMATS.to_vec(),
    }
}

/// Volume-key page-turn state: the flag is authoritative for the reader
/// surface; on Android the value is additionally pushed into the
/// content-reader plugin best-effort (hardware interception is advisory —
/// a plugin miss never blocks the toggle).
static VOLUME_KEY_CAPTURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[tauri::command]
async fn set_volume_key_capture(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        VOLUME_KEY_CAPTURE.store(enabled, Ordering::Relaxed);
        #[cfg(target_os = "android")]
        {
            if let Some(handle) = app.try_state::<ContentReaderHandle>() {
                if let Err(error) = handle.0.run_mobile_plugin::<serde_json::Value>(
                    "setVolumeKeyCapture",
                    serde_json::json!({ "enabled": enabled }),
                ) {
                    storage::app_log(
                        "reader",
                        &format!("setVolumeKeyCapture plugin call failed: {error}"),
                    );
                }
            }
        }
        let _ = app;
        Ok(enabled)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn set_reader_immersive(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "android")]
        {
            let handle = app
                .try_state::<ContentReaderHandle>()
                .ok_or_else(|| "content reader plugin unavailable".to_string())?;
            handle
                .0
                .run_mobile_plugin::<serde_json::Value>(
                    "setReaderImmersive",
                    serde_json::json!({ "enabled": enabled }),
                )
                .map_err(|error| error.to_string())?;
        }
        let _ = app;
        Ok(enabled)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Open-intent URIs queued for the frontend to drain — desktop "open with"
/// for non-Markdown payloads lands here instead of the Markdown-only
/// `open-file` event. On Android the Kotlin activity's VIEW-intent URIs
/// additionally live in the plugin and are merged in on each take.
fn pending_open_uris() -> &'static Mutex<Vec<String>> {
    static URIS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    URIS.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn queue_pending_open_uri(uri: String) {
    if let Ok(mut uris) = pending_open_uris().lock() {
        uris.push(uri);
    }
}

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct PluginPendingUris {
    #[serde(default)]
    uris: Vec<String>,
}

#[tauri::command]
async fn take_pending_open_uris(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // `mut` only under Android (the plugin queue folds into this vec).
        #[allow(unused_mut)]
        let mut uris = pending_open_uris()
            .lock()
            .map(|mut uris| std::mem::take(&mut *uris))
            .unwrap_or_default();
        // The plugin's queue holds URIs from intents that arrived before the
        // Rust side was ready — drain it every time. A plugin failure must
        // not eat the local queue (logged, not fatal).
        #[cfg(target_os = "android")]
        if let Some(handle) = app.try_state::<ContentReaderHandle>() {
            match handle.0.run_mobile_plugin::<PluginPendingUris>(
                "takePendingOpenUris",
                serde_json::json!({}),
            ) {
                Ok(result) => uris.extend(result.uris),
                Err(error) => storage::app_log(
                    "app",
                    &format!("takePendingOpenUris plugin call failed: {error}"),
                ),
            }
        }
        let _ = app;
        Ok(uris)
    })
    .await
    .map_err(|error| error.to_string())?
}

// ===== Readable chapters (Markdown + EPUB) =============================

/// One reader-ready chapter. Markdown books answer from disk directly; EPUB
/// books route through `epub::read_epub_chapter_with_neighbors` (sanitized
/// XHTML, neighbours taken from the manifest loaded once here).
/// Either way the book root gets the recursive asset scope grant so
/// chapter-relative resources render.
#[tauri::command]
async fn get_readable_chapter(
    app: tauri::AppHandle,
    book_id: String,
    chapter_id: String,
) -> Result<epub::ReadableChapter, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        let (book_root, manifest, _) =
            library::book_context(Path::new(&value.library_root), &book_id)?;
        let position = manifest
            .chapters
            .iter()
            .position(|chapter| chapter.id == chapter_id)
            .ok_or_else(|| format!("Chapter not found: {chapter_id}"))?;
        let chapter = &manifest.chapters[position];
        // Recursive grant on the book root covers everything a chapter may
        // reference — content/, assets/, cover — nothing more is needed.
        let _ = app.asset_protocol_scope().allow_directory(&book_root, true);
        if epub::publication_path(&book_root).is_file() {
            // The manifest is already in memory (see `position` above) — derive
            // the neighbours here instead of letting the chapter reader re-read
            // and re-parse manifest.json on every page turn.
            let prev_chapter_id = position
                .checked_sub(1)
                .map(|index| manifest.chapters[index].id.clone());
            let next_chapter_id = (position + 1 < manifest.chapters.len())
                .then(|| manifest.chapters[position + 1].id.clone());
            return epub::read_epub_chapter_with_neighbors(
                &book_root,
                chapter,
                (prev_chapter_id, next_chapter_id),
            );
        }
        // Markdown chapter: enforce the same cap+decode the standalone file
        // path uses, then hand the reader a ReadableChapter-shaped payload.
        let file = atomic_file::long_path(
            &book_root.join(chapter.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
        );
        let metadata = fs::metadata(&file).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("Chapter file is missing".to_string());
        }
        if metadata.len() > MAX_MARKDOWN_FILE_BYTES {
            return Err("MARKDOWN_FILE_TOO_LARGE".to_string());
        }
        let bytes = {
            let file = fs::File::open(&file).map_err(|error| error.to_string())?;
            // Pre-size from the metadata just read: otherwise a multi-MB
            // chapter reallocates/copies through several Vec growth steps
            // before decode. `take()` still caps a swapped-in file.
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            file.take(MAX_MARKDOWN_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 > MAX_MARKDOWN_FILE_BYTES {
                return Err("MARKDOWN_FILE_TOO_LARGE".to_string());
            }
            bytes
        };
        let (content, _) = decode_markdown_bytes(bytes)?;
        let resource_dir = Path::new(&chapter.path)
            .parent()
            .map(|parent| parent.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        Ok(epub::ReadableChapter {
            format: "markdown".to_string(),
            chapter_id: chapter.id.clone(),
            title: chapter.title.clone(),
            content,
            resource_dir,
            prev_chapter_id: position
                .checked_sub(1)
                .map(|index| manifest.chapters[index].id.clone()),
            next_chapter_id: (position + 1 < manifest.chapters.len())
                .then(|| manifest.chapters[position + 1].id.clone()),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Resolve a chapter-relative resource href (an image in an EPUB chapter)
/// to an absolute in-book path the webview can load via convertFileSrc.
/// Returns `None` for out-of-book or unresolvable hrefs.
#[tauri::command]
async fn resolve_book_resource(
    app: tauri::AppHandle,
    book_id: String,
    chapter_id: String,
    href: String,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let value = settings::load_settings()?;
        let (book_root, manifest, _) =
            library::book_context(Path::new(&value.library_root), &book_id)?;
        let chapter = manifest
            .chapters
            .iter()
            .find(|chapter| chapter.id == chapter_id)
            .ok_or_else(|| format!("Chapter not found: {chapter_id}"))?;
        let chapter_path = book_root.join(chapter.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let Some(resolved) = epub::resolve_book_resource(&book_root, &chapter_path, &href) else {
            return Ok(None);
        };
        let _ = app.asset_protocol_scope().allow_file(&resolved);
        Ok(Some(resolved.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|error| error.to_string())?
}

// ===== Reader state: locators, bookmarks, search (reader.db) ===========

#[tauri::command]
async fn save_reader_locator(book_id: String, locator_json: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let locator: contracts::ReaderLocator =
            serde_json::from_str(&locator_json).map_err(|error| error.to_string())?;
        contracts::validate_reader_locator(&locator)?;
        if locator.book_id != book_id {
            return Err("阅读位置与书籍不匹配".to_string());
        }
        reader_db::ReaderDb::open_current()?.save_locator(&book_id, &locator_json)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn load_reader_locator(book_id: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        reader_db::ReaderDb::open_current()?.load_locator(&book_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn add_reader_bookmark(
    book_id: String,
    bookmark_id: Option<String>,
    locator_json: String,
    label: String,
) -> Result<reader_db::BookmarkRow, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bookmark_id = bookmark_id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let db = reader_db::ReaderDb::open_current()?;
        db.add_bookmark(&book_id, &bookmark_id, &locator_json, &label)?;
        db.list_bookmarks(&book_id)?
            .into_iter()
            .find(|row| row.bookmark_id == bookmark_id)
            .ok_or_else(|| "Bookmark was not persisted".to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn remove_reader_bookmark(book_id: String, bookmark_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        reader_db::ReaderDb::open_current()?.remove_bookmark(&book_id, &bookmark_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn list_reader_bookmarks(book_id: String) -> Result<Vec<reader_db::BookmarkRow>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        reader_db::ReaderDb::open_current()?.list_bookmarks(&book_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Full-text search inside one book. The chapter-text index is built lazily
/// on first search — Markdown reads the chapter files, EPUB goes through
/// `epub::chapter_plain_text` — then FTS5 (or the LIKE fallback) answers.
#[tauri::command]
async fn search_book(
    book_id: String,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<reader_db::SearchHit>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = reader_db::ReaderDb::open_current()?;
        if !db.fts_book_indexed(&book_id)? {
            let value = settings::load_settings()?;
            let (book_root, manifest, _) =
                library::book_context(Path::new(&value.library_root), &book_id)?;
            let is_epub = epub::publication_path(&book_root).is_file();
            for chapter in &manifest.chapters {
                let text = if is_epub {
                    epub::chapter_plain_text(&book_root, chapter)?
                } else {
                    let path = atomic_file::long_path(
                        &book_root.join(chapter.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
                    );
                    match fs::metadata(&path) {
                        Ok(metadata)
                            if metadata.is_file() && metadata.len() <= MAX_MARKDOWN_FILE_BYTES =>
                        {
                            fs::read(&path)
                                .ok()
                                .and_then(|bytes| decode_markdown_bytes(bytes).ok())
                                .map(|(content, _)| content)
                                .unwrap_or_default()
                        }
                        _ => String::new(),
                    }
                };
                db.fts_replace_chapter(&book_id, &chapter.id, &chapter.title, &text)?;
            }
        }
        db.fts_search(&book_id, &query, limit.unwrap_or(50))
    })
    .await
    .map_err(|error| error.to_string())?
}

// ===== Reading bundle export/import ====================================

/// Bundle shape: `bundle.json` header + `settings.json` +
/// `reader-preferences.json` + `reader_db.json` + `books/<dir_rel>/…`
/// (full book directories — the point is library/progress/bookmarks
/// restoration, so chapter/asset payloads ship wholesale under the 4 GiB
/// uncompressed guard).
const BUNDLE_SCHEMA_VERSION: u32 = 1;
const BUNDLE_MAX_CONTENT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// JSON metadata members stay tiny — a multi-MB `settings.json` is hostile.
const BUNDLE_META_ENTRY_CAP: u64 = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleExportResult {
    path: String,
    books: u32,
    bytes: u64,
    issues: Vec<library::LibraryIssue>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleImportResult {
    books_imported: u32,
    books_skipped: u32,
    reader_books: u32,
    locators: u32,
    bookmarks: u32,
    settings_restored: bool,
    preferences_restored: bool,
    issues: Vec<library::LibraryIssue>,
}

/// Collect every regular file under `root` as ("/"-joined rel, abs) pairs —
/// the bundle walk skips symlinks/reparse points and dot-prefixed CONTROL
/// dirs (`.trash`, `.incoming`) but keeps dot-prefixed FILES like
/// `.reading.json`, which is exactly the state a bundle must carry.
fn bundle_walk_book_files(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 32 {
            continue;
        }
        for entry in fs::read_dir(&dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            if file_type.is_dir() {
                if name.starts_with('.') || crate::atomic_file::is_reparse_point(&metadata) {
                    continue;
                }
                stack.push((entry.path(), depth + 1));
            } else if file_type.is_file() && !crate::atomic_file::is_reparse_point(&metadata) {
                let rel = entry
                    .path()
                    .strip_prefix(root)
                    .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if !rel.is_empty() {
                    out.push((rel, entry.path()));
                }
            }
        }
    }
    Ok(out)
}

/// Every component of a `books/…` member path must be a real directory name
/// — no empties, dots, separators, drives or device names — or the whole
/// book entry is refused.
fn bundle_safe_member_path(member: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in member.split('/') {
        // `.reading.json` is the one dot-file a bundle legitimately carries.
        let is_dotfile = component.starts_with('.') && component != ".reading.json";
        if component.is_empty()
            || is_dotfile
            || component == ".."
            || component.contains('\\')
            || component.contains(':')
            || crate::contracts::is_reserved_device_name(component)
        {
            return None;
        }
        out.push(component);
    }
    Some(out)
}

#[tauri::command]
async fn export_reading_bundle(dest_path: String) -> Result<BundleExportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;

        let locations = storage::StorageLocations::current_with_library_settings()?;
        let dest = PathBuf::from(&dest_path);
        if !dest.is_absolute() {
            return Err("BUNDLE_PATH_NOT_ABSOLUTE".to_string());
        }
        let parent = dest
            .parent()
            .ok_or_else(|| "BUNDLE_PATH_NO_PARENT".to_string())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        // A bundle inside the library would ingest itself on the next export.
        if storage::path_within(&locations.library_root, &dest) {
            return Err("BUNDLE_DEST_INSIDE_LIBRARY".to_string());
        }
        let library_root = atomic_file::long_path(&locations.library_root);
        let mut manifest_paths = Vec::new();
        let mut issues = Vec::new();
        library::collect_manifests(&library_root, 0, &mut manifest_paths, &mut issues, false);
        manifest_paths.sort();

        // Write to a sibling temp name, then rename — the bundle is either
        // complete or absent, never a truncated zip at the real path.
        let tmp = dest.with_file_name(format!(
            ".{}.tmp-{}",
            dest.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "bundle".to_string()),
            uuid::Uuid::new_v4().simple()
        ));
        let outcome = (|| -> Result<(u32, u64), String> {
            let file = fs::File::create(&tmp).map_err(|error| error.to_string())?;
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("bundle.json", options)
                .map_err(|error| error.to_string())?;
            serde_json::to_writer(
                &mut zip,
                &serde_json::json!({
                    "schemaVersion": BUNDLE_SCHEMA_VERSION,
                    "kind": "immersive-reader-bundle",
                    "appVersion": env!("CARGO_PKG_VERSION"),
                    "channel": locations.channel,
                    "createdAt": chrono::Utc::now().to_rfc3339(),
                }),
            )
            .map_err(|error| error.to_string())?;
            for (member, source) in [
                ("settings.json", locations.settings_path.clone()),
                (
                    "reader-preferences.json",
                    settings::app_state_dir().join("reader-preferences.json"),
                ),
            ] {
                if let Ok(bytes) = fs::read(atomic_file::long_path(&source)) {
                    zip.start_file(member, options)
                        .map_err(|error| error.to_string())?;
                    zip.write_all(&bytes).map_err(|error| error.to_string())?;
                }
            }
            // reader.db state — index, locators, bookmarks. A missing/unopenable
            // DB just omits the member (export never fails on it).
            if let Ok(db) = reader_db::ReaderDb::open_current() {
                if let Ok(state) = db.export_state() {
                    zip.start_file("reader_db.json", options)
                        .map_err(|error| error.to_string())?;
                    serde_json::to_writer(&mut zip, &state).map_err(|error| error.to_string())?;
                }
            }
            let mut books = 0_u32;
            let mut total = 0_u64;
            for manifest_path in manifest_paths {
                let Some(book_root) = manifest_path.parent() else {
                    continue;
                };
                // Unreadable manifest → issue, and the dir is not a book.
                if library::read_manifest(&manifest_path).is_err() {
                    issues.push(library::LibraryIssue {
                        path: manifest_path.to_string_lossy().into_owned(),
                        message: "BUNDLE_BOOK_MANIFEST_INVALID".to_string(),
                    });
                    continue;
                }
                let dir_rel = book_root
                    .strip_prefix(&library_root)
                    .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if dir_rel.is_empty() {
                    continue;
                }
                for (rel, file) in bundle_walk_book_files(book_root)? {
                    let len = fs::metadata(&file)
                        .map_err(|error| error.to_string())?
                        .len();
                    total = total.saturating_add(len);
                    if total > BUNDLE_MAX_CONTENT_BYTES {
                        return Err("BUNDLE_TOO_LARGE".to_string());
                    }
                    zip.start_file(format!("books/{dir_rel}/{rel}"), options)
                        .map_err(|error| error.to_string())?;
                    let mut input = fs::File::open(&file).map_err(|error| error.to_string())?;
                    std::io::copy(&mut input, &mut zip).map_err(|error| error.to_string())?;
                }
                books += 1;
            }
            zip.finish().map_err(|error| error.to_string())?;
            Ok((books, total))
        })();
        match outcome {
            Ok((books, bytes)) => {
                if dest.exists() {
                    fs::remove_file(&dest).map_err(|error| error.to_string())?;
                }
                fs::rename(&tmp, &dest).map_err(|error| error.to_string())?;
                Ok(BundleExportResult {
                    path: dest.to_string_lossy().into_owned(),
                    books,
                    bytes,
                    issues,
                })
            }
            Err(error) => {
                let _ = fs::remove_file(&tmp);
                Err(error)
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn import_reading_bundle(src_path: String) -> Result<BundleImportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        startup::wait();
        use std::io::Read as _;

        let locations = storage::StorageLocations::current_with_library_settings()?;
        let src = atomic_file::long_path(Path::new(&src_path));
        let metadata = fs::metadata(&src).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("BUNDLE_SRC_NOT_FILE".to_string());
        }
        if metadata.len() > BUNDLE_MAX_CONTENT_BYTES {
            return Err("BUNDLE_TOO_LARGE".to_string());
        }
        let library_root = atomic_file::long_path(&locations.library_root);
        let mut issues: Vec<library::LibraryIssue> = Vec::new();
        let push_issue = |issues: &mut Vec<library::LibraryIssue>, member: &str, code: &str| {
            issues.push(library::LibraryIssue {
                path: member.to_string(),
                message: code.to_string(),
            });
        };

        // Staging lives under the swept `手动/.incoming` — same volume as the
        // final rename, cleaned at startup if we die mid-restore.
        let staging = atomic_file::long_path(
            &library_root
                .join("手动")
                .join(".incoming")
                .join(format!("bundle-{}", uuid::Uuid::new_v4())),
        );
        fs::create_dir_all(&staging).map_err(|error| error.to_string())?;

        let result = (|| -> Result<BundleImportResult, String> {
            let file = fs::File::open(&src).map_err(|error| error.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;

            // ---- header gate ------------------------------------------
            let header: serde_json::Value = (|| {
                let mut entry = zip
                    .by_name("bundle.json")
                    .map_err(|_| "BUNDLE_MANIFEST_MISSING".to_string())?;
                if entry.size() > BUNDLE_META_ENTRY_CAP {
                    return Err("BUNDLE_META_TOO_LARGE".to_string());
                }
                serde_json::from_reader(&mut entry).map_err(|error| error.to_string())
            })()?;
            if header.get("schemaVersion").and_then(|v| v.as_u64())
                != Some(BUNDLE_SCHEMA_VERSION as u64)
            {
                return Err("BUNDLE_SCHEMA_UNSUPPORTED".to_string());
            }

            // ---- pass 1: meta members + book extraction ---------------
            let mut settings_bytes: Option<Vec<u8>> = None;
            let mut prefs_bytes: Option<Vec<u8>> = None;
            let mut reader_db_bytes: Option<Vec<u8>> = None;
            let mut total = 0_u64;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
                if entry.is_dir() {
                    continue;
                }
                let Some(name) = entry
                    .enclosed_name()
                    .map(|p| p.to_string_lossy().into_owned())
                else {
                    continue;
                };
                let name = name.as_str();
                if entry.size() > BUNDLE_MAX_CONTENT_BYTES {
                    push_issue(&mut issues, name, "BUNDLE_ENTRY_TOO_LARGE");
                    continue;
                }
                match name {
                    "settings.json" | "reader-preferences.json" | "reader_db.json" => {
                        if entry.size() > BUNDLE_META_ENTRY_CAP {
                            push_issue(&mut issues, name, "BUNDLE_META_TOO_LARGE");
                            continue;
                        }
                        let mut bytes = Vec::new();
                        entry
                            .take(BUNDLE_META_ENTRY_CAP + 1)
                            .read_to_end(&mut bytes)
                            .map_err(|error| error.to_string())?;
                        match name {
                            "settings.json" => settings_bytes = Some(bytes),
                            "reader-preferences.json" => prefs_bytes = Some(bytes),
                            _ => reader_db_bytes = Some(bytes),
                        }
                    }
                    _ => {
                        let Some(book_rel) = name.strip_prefix("books/") else {
                            continue; // unknown top-level members are ignored
                        };
                        let Some(rel) = bundle_safe_member_path(book_rel) else {
                            push_issue(&mut issues, name, "BUNDLE_MEMBER_UNSAFE_PATH");
                            continue;
                        };
                        total = total.saturating_add(entry.size());
                        if total > BUNDLE_MAX_CONTENT_BYTES {
                            return Err("BUNDLE_TOO_LARGE".to_string());
                        }
                        let target = staging.join(&rel);
                        if let Some(parent) = target.parent() {
                            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                        }
                        let mut out =
                            fs::File::create(&target).map_err(|error| error.to_string())?;
                        std::io::copy(&mut entry, &mut out).map_err(|error| error.to_string())?;
                    }
                }
            }

            // ---- validate + place each staged book --------------------
            let mut staged_manifests = Vec::new();
            library::collect_manifests(&staging, 0, &mut staged_manifests, &mut issues, false);
            staged_manifests.sort();
            // Book ids already on the shelf — bundle entries with a
            // duplicate bookId are skipped instead of overwriting.
            let mut existing_ids = std::collections::HashSet::new();
            let mut shelf_manifests = Vec::new();
            library::collect_manifests(
                &library_root,
                0,
                &mut shelf_manifests,
                &mut Vec::new(),
                false,
            );
            for path in &shelf_manifests {
                if let Ok(manifest) = library::read_manifest(path) {
                    existing_ids.insert(manifest.book_id);
                }
            }
            let mut books_imported = 0_u32;
            let mut books_skipped = 0_u32;
            let mut known_book_ids = std::collections::HashSet::new();
            for manifest_path in staged_manifests {
                let Some(staged_root) = manifest_path.parent() else {
                    continue;
                };
                let manifest = match library::read_manifest(&manifest_path) {
                    Ok(manifest) => manifest,
                    Err(error) => {
                        push_issue(
                            &mut issues,
                            &manifest_path.to_string_lossy(),
                            &format!("BUNDLE_BOOK_MANIFEST_INVALID: {error}"),
                        );
                        continue;
                    }
                };
                if existing_ids.contains(&manifest.book_id) {
                    books_skipped += 1;
                    // Its reader state still merges — the shelf copy is the
                    // same book.
                    known_book_ids.insert(manifest.book_id.clone());
                    push_issue(
                        &mut issues,
                        &staged_root.to_string_lossy(),
                        "BUNDLE_BOOK_EXISTS",
                    );
                    continue;
                }
                // `.reading.json` must validate against THIS manifest — a
                // corrupt one is dropped (with an issue), never written.
                let progress_file = staged_root.join(".reading.json");
                if progress_file.is_file() {
                    let valid = crate::atomic_file::read_to_string(&progress_file)
                        .ok()
                        .and_then(|raw| {
                            serde_json::from_str::<contracts::ReadingProgress>(&raw).ok()
                        })
                        .map(|progress| contracts::validate_reading(&progress, &manifest).is_ok())
                        .unwrap_or(false);
                    if !valid {
                        let _ = fs::remove_file(&progress_file);
                        push_issue(
                            &mut issues,
                            &format!("{}/.reading.json", manifest.book_id),
                            "BUNDLE_PROGRESS_INVALID",
                        );
                    }
                }
                // publication.json must parse as a Publication — the epub
                // reader refuses a malformed sidecar anyway.
                let publication_file = staged_root.join("publication.json");
                if publication_file.is_file() {
                    let valid = crate::atomic_file::read_to_string(&publication_file)
                        .ok()
                        .and_then(|raw| serde_json::from_str::<epub::Publication>(&raw).ok())
                        .is_some();
                    if !valid {
                        let _ = fs::remove_file(&publication_file);
                        push_issue(
                            &mut issues,
                            &format!("{}/publication.json", manifest.book_id),
                            "BUNDLE_PUBLICATION_INVALID",
                        );
                    }
                }
                // Restore under the bundle's original dir_rel when its
                // components are safe, else fall back to 手动/<title>.
                let rel = staged_root
                    .strip_prefix(&staging)
                    .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let target_parent = bundle_safe_member_path(&rel)
                    .and_then(|rel| rel.parent().map(|p| library_root.join(p)))
                    .unwrap_or_else(|| library_root.join("手动"));
                let target_name = staged_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("imported-book");
                fs::create_dir_all(&target_parent).map_err(|error| error.to_string())?;
                let target = importer::unique_target(&target_parent, target_name);
                fs::rename(staged_root, atomic_file::long_path(&target))
                    .map_err(|error| error.to_string())?;
                books_imported += 1;
                known_book_ids.insert(manifest.book_id.clone());
            }

            // ---- reader.db merge --------------------------------------
            let mut merged = reader_db::ReaderDbMergeReport::default();
            if let Some(bytes) = reader_db_bytes {
                match serde_json::from_slice::<serde_json::Value>(&bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|state| {
                        reader_db::ReaderDb::open_current()
                            .and_then(|db| db.import_state(&state, &known_book_ids))
                    }) {
                    Ok(report) => merged = report,
                    Err(error) => push_issue(&mut issues, "reader_db.json", &error),
                }
            }

            // ---- settings + reader preferences -------------------------
            let mut settings_restored = false;
            if let Some(bytes) = settings_bytes {
                match serde_json::from_slice::<settings::AppSettings>(&bytes) {
                    Ok(mut incoming) => {
                        // The library root is machine-local — never import it.
                        incoming.library_root =
                            locations.library_root.to_string_lossy().into_owned();
                        match settings::save_settings(&incoming) {
                            Ok(()) => settings_restored = true,
                            Err(error) => push_issue(&mut issues, "settings.json", &error),
                        }
                    }
                    Err(error) => push_issue(&mut issues, "settings.json", &error.to_string()),
                }
            }
            let mut preferences_restored = false;
            if let Some(bytes) = prefs_bytes {
                match serde_json::from_slice::<reader_preferences::ReaderPreferences>(&bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|prefs| reader_preferences::save(&prefs))
                {
                    Ok(()) => preferences_restored = true,
                    Err(error) => push_issue(&mut issues, "reader-preferences.json", &error),
                }
            }

            Ok(BundleImportResult {
                books_imported,
                books_skipped,
                reader_books: merged.books,
                locators: merged.locators,
                bookmarks: merged.bookmarks,
                settings_restored,
                preferences_restored,
                issues,
            })
        })();
        let _ = fs::remove_dir_all(&staging);
        result
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
        startup::wait();
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
        startup::wait();
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
        startup::wait();
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
        startup::wait();
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
    tauri::async_runtime::spawn_blocking(|| {
        let locations = storage::StorageLocations::current()?;
        // Only Android stages SAF picks in the private app cache.
        #[cfg(target_os = "android")]
        let app_cache = storage::android_base_dirs()
            .ok()
            .map(|base| base.app_cache.clone());
        #[cfg(not(target_os = "android"))]
        let app_cache: Option<PathBuf> = None;
        temporary_content::items(&locations, app_cache.as_deref())
    })
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
    // The tiny_http 连读 server is a desktop surface — on Android the
    // backend refuses outright (the frontend hides the entry via
    // get_platform_capabilities, this is the hard gate behind it).
    #[cfg(target_os = "android")]
    {
        let _ = (book_id, state);
        return Err("READER_SESSION_UNSUPPORTED_PLATFORM".to_string());
    }
    #[cfg(not(target_os = "android"))]
    {
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
async fn create_zhihu_task(
    request: zhihu::CreateZhihuTaskRequest,
    app: tauri::AppHandle,
) -> Result<tasks::TaskSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<tasks::TaskSnapshot, String> {
        startup::wait();
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
        startup::wait();
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
        startup::wait();
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
            Err(error) => Err(error),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(desktop)]
    startup::begin();
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
    {
        // Queue before creating the WebView: setup-time eval can run in the
        // initial blank document and disappear before the frontend mounts.
        let args: Vec<String> = std::env::args().collect();
        if let Some(path) = initial_markdown_path(&args).or_else(|| initial_file_path(&args)) {
            queue_pending_open_uri(path);
        }
    }
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        if let Some(path) = initial_markdown_path(&args).or_else(|| initial_file_path(&args)) {
            // Persist first, then notify/focus. Both an already mounted page
            // and a page still starting drain the same queue exactly once.
            queue_pending_open_uri(path);
            let _ = app.emit("pending-open-files", ());
        }
        let hidden_qa = matches!(settings::AppChannel::current(), settings::AppChannel::Qa(_))
            && std::env::var("IMMERSIVE_QA_HEADLESS").as_deref() == Ok("1");
        if let Some(window) = app.get_webview_window("main").filter(|_| !hidden_qa) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }));
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let context = tauri::generate_context!();
    #[cfg(desktop)]
    let context = {
        let mut context = context;
        // The installed executable can be exercised without opening a window on a user's desktop.
        // A valid isolated QA channel is required; production never consumes this switch.
        if let settings::AppChannel::Qa(run_id) = settings::AppChannel::current() {
            // Isolate the WebView profile and single-instance identity too, not only Rust data roots.
            context.config_mut().identifier = format!(
                "{}.qa.{}",
                context.config().identifier,
                run_id.replace('_', "-")
            );
            let hidden = std::env::var("IMMERSIVE_QA_HEADLESS").as_deref() == Ok("1");
            for window in &mut context.config_mut().app.windows {
                if hidden {
                    window.visible = false;
                    window.focus = false;
                }
            }
        }
        context
    };
    let app = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        // The ONLY in-app mobile plugin: streams SAF content:// picks into
        // the app cache (ContentReaderPlugin.kt in the Android app module).
        // Every `run_mobile_plugin` call in this crate targets one of its
        // `@Command`s — `register_android_plugin` resolves the class name
        // synchronously and a missing class fails plugin setup, which fails
        // `Builder::build()` and kills the whole app before the first frame
        // (the 1.2.1 Android no-launch bug: a `secrets` plugin was registered
        // for a SecretsPlugin.kt that never existed). Keep this one-to-one.
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("content-reader")
                .setup(|app, api| {
                    #[cfg(target_os = "android")]
                    {
                        let handle = api.register_android_plugin(
                            "com.lingcang.immersivereading",
                            "ContentReaderPlugin",
                        )?;
                        app.manage(ContentReaderHandle(handle));
                    }
                    let _ = (app, api);
                    Ok(())
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_file_mtime,
            stage_content_uri,
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
            scan_library,
            load_library_snapshot,
            repair_library_duplicates,
            open_book,
            get_book_chapter_path,
            save_book_progress,
            import_markdown_folder,
            begin_import,
            get_import_status,
            cancel_import,
            get_platform_capabilities,
            set_volume_key_capture,
            set_reader_immersive,
            take_pending_open_uris,
            get_readable_chapter,
            resolve_book_resource,
            save_reader_locator,
            load_reader_locator,
            add_reader_bookmark,
            remove_reader_bookmark,
            list_reader_bookmarks,
            search_book,
            export_reading_bundle,
            import_reading_bundle,
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
            create_zhihu_task,
            get_zhihu_login_status,
            start_zhihu_login,
            clear_zhihu_login,
            start_zhihu_task,
            control_zhihu_task,
            open_task_result,
        ])
        .manage(std::sync::Arc::new(
            reader_server::ReaderServiceState::default(),
        ))
        .setup(|app| {
            let _ = app;
            // Android has no `dirs` backend — every storage root derives from
            // the app-private directories exposed by the mobile path plugin.
            // This must run before anything that touches
            // `StorageLocations::current()` (app_log, settings). Failure is
            // fatal: without storage roots the app cannot run at all.
            #[cfg(target_os = "android")]
            if let Err(error) = storage::init_android_roots(app.handle()) {
                eprintln!("Android storage roots init failed: {error}");
                panic!("failed to initialize Android storage roots: {error}");
            }
            storage::app_log(
                "app",
                &format!("immersive-reader v{} starting", env!("CARGO_PKG_VERSION")),
            );
            // Android cold start: `setup` completes before the webview can paint
            // its first frame, so nothing here may block on filesystem walks or
            // a database open. The only required Android startup work is the
            // storage roots above plus this log line; residue sweeps move to a
            // background thread. A late sweep is safe: SAF staging in the app
            // cache cannot outlive the process that created it, so leftovers are
            // always residue (the fixed delay keeps it off the cold path).
            #[cfg(target_os = "android")]
            {
                let stale_staging = storage::android_base_dirs()
                    .map(|base| temporary_content::android_staging_dirs(&base.app_cache))
                    .unwrap_or_default();
                std::thread::spawn(move || {
                    std::thread::sleep(ANDROID_DEFERRED_MAINTENANCE_DELAY);
                    let Ok(base) = storage::android_base_dirs() else {
                        return;
                    };
                    // SAF staging cannot outlive the op that created it — the URI
                    // grant dies with process death, so `import-*`,
                    // `mobile-import-*` and `临时` leftovers are always residue.
                    let removed = temporary_content::sweep_stale_app_cache_staging(
                        &base.app_cache,
                        &stale_staging,
                    );
                    if removed > 0 {
                        storage::app_log(
                            "app",
                            &format!("swept {removed} stale app-cache staging dirs"),
                        );
                    }
                });
            }
            // Desktop-only startup recovery: control.db task recovery, import
            // staging sweep, publish residue/transaction recovery and the
            // publish-index reconcile. The acquisition engines never ship on
            // Android, so none of it has work to do there — and the synchronous
            // control.db open blocked the first frame on every cold start.
            #[cfg(desktop)]
            {
                // P1-1: Job Objects kill podcast workers when the app exits, but
                // their tasks stayed Running forever with no recovery path. At
                // startup every still-active task whose worker is gone gets marked
                // Interrupted (restartable); the live-worker id set comes from the
                // worker registry so a just-started task is never clobbered.
                // These run synchronously inside setup: a background thread races
                // the webview's first invoke (a fast `import_markdown_folder` call
                // could watch its `.incoming` staging get swept mid-import).
                startup::defer(|| {
                    // P2-19: sweep abandoned import staging and P2-11 discard
                    // markers left by a crash — before the UI can start new work.
                    if let Ok(locations) =
                        storage::StorageLocations::current_with_library_settings()
                    {
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
                        if let Ok(transactions) =
                            publish::list_transactions(&locations.library_root)
                        {
                            for transaction in transactions {
                                if matches!(
                                    transaction.phase,
                                    publish::PublishPhase::Committed
                                        | publish::PublishPhase::RolledBack
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
                });
            }
            Ok(())
        })
        .build(context)
        .unwrap_or_else(|error| {
            storage::app_log(
                "startup",
                &format!("tauri application build failed: {error}"),
            );
            panic!("error while building tauri application: {error}");
        });

    // macOS: file opened via Apple Event (double-click / Open With)
    app.run(|app_handle, event| match event {
        #[cfg(desktop)]
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            api.prevent_close();
            let _ = label;
            let _ = app_handle.emit("request-app-exit", serde_json::json!({"mode": "preserve"}));
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        RunEvent::Opened { urls } => {
            for url in urls {
                if let Ok(path) = url.to_file_path() {
                    let path_str = path.to_string_lossy().to_string();
                    if is_markdown_path(&path_str) {
                        register_opened_markdown(&path);
                        let _ = app_handle.emit("open-file", path_str);
                    } else {
                        // .epub/.zip etc: the frontend drains the queue via
                        // take_pending_open_uris and runs begin_import.
                        queue_pending_open_uri(path_str);
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
        book_asset_root, get_platform_capabilities, initial_file_path, initial_markdown_path,
        is_markdown_path, reconcile_discard_markers,
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
    fn platform_capabilities_keep_legacy_flags_and_add_mobile_aware_ones() {
        // The Android shell gates Zhihu/podcast/deepseek surfaces on these
        // fields; the wire keeps the historic camelCase spellings and older
        // frontends ignore the additions (normalizer falls back per field).
        let caps = get_platform_capabilities();
        let json = serde_json::to_value(&caps).expect("capabilities must serialize");
        let object = json.as_object().expect("capabilities must be an object");
        for key in [
            "platform",
            "readerSession",
            "contentUriImport",
            "epub",
            "volumeKeyCapture",
            "readingBundle",
            "isMobile",
            "taskAcquisition",
            "directoryImport",
            "secretStore",
            "markdownEditing",
            "supportedFormats",
        ] {
            assert!(object.contains_key(key), "missing capability field {key}");
        }

        #[cfg(target_os = "windows")]
        {
            assert_eq!(caps.platform, "windows");
            assert!(!caps.is_mobile);
            assert!(caps.task_acquisition);
            assert!(caps.directory_import);
            assert!(caps.secret_store);
            assert!(caps.markdown_editing);
            assert_eq!(
                caps.supported_formats,
                ["md", "markdown", "txt", "epub", "zip"]
            );
        }
        #[cfg(target_os = "android")]
        {
            assert_eq!(caps.platform, "android");
            assert!(caps.is_mobile);
            assert!(!caps.task_acquisition);
            assert!(!caps.directory_import);
            assert!(!caps.secret_store);
            assert!(!caps.markdown_editing);
            assert_eq!(caps.supported_formats, ["md", "markdown", "txt", "epub"]);
            assert!(!caps.reader_session);
            assert!(caps.content_uri_import);
        }
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
    fn initial_file_path_preserves_existing_epub_path_and_rejects_flags() {
        let dir = scoped_temp_dir("launch path with spaces");
        let path = dir.join("book.epub");
        fs::write(&path, b"synthetic launch fixture").unwrap();
        let raw = path.to_string_lossy().into_owned();
        assert_eq!(
            initial_file_path(&["reader.exe".into(), raw.clone()]),
            Some(raw)
        );
        assert_eq!(
            initial_file_path(&["reader.exe".into(), "--help".into()]),
            None
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
