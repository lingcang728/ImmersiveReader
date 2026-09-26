//! Import operation pipeline: an in-process registry of backgrounded
//! imports (`begin_import`/`get_import_status`/`cancel_import`) plus the
//! staging helpers that turn directories, picked files, Android content
//! URIs, and archives into something `importer`/`epub` can shelve.
//!
//! Status machine: `running` → `succeeded` | `failed` | `cancelled`.
//! Cancellation is cooperative — the worker's cancel flag is polled between
//! files by `importer::import_markdown_folder_inner` and returns
//! `IMPORT_CANCELLED`, which maps to the `cancelled` terminal state.
//! Staging dirs are per-op and always removed when the op reaches a
//! terminal state, whatever the outcome.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Hard cap on the uncompressed payload one archive may expand to — a zip
/// bomb must never reach the library root.
const MAX_ARCHIVE_EXPAND_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Bound on archive member count for the same reason.
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
/// Keep the last N operations addressable by `get_import_status`; older
/// finished ops are evicted so the map cannot grow forever.
const MAX_RETAINED_OPS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// Snapshot the status poller returns. `result` carries the flattened
/// import outcome (manifest + issues) once the op succeeded.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOperation {
    pub op_id: String,
    pub kind: String,
    pub label: String,
    pub status: ImportStatus,
    pub total_files: u32,
    pub done_files: u32,
    pub message: Option<String>,
    pub result: Option<serde_json::Value>,
    pub finished_at: Option<String>,
}

/// Live op record: the snapshot fields plus the cooperative-cancel flag and
/// the progress counters the worker shares with `import_markdown_folder_inner`.
struct OpEntry {
    snapshot: ImportOperation,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU32>,
    total: Arc<AtomicU32>,
}

/// Progress handle handed to the worker closure — read-only view of the
/// cancel flag plus the counters. `cancel()` on the entry flips the flag;
/// the worker just has to keep polling it.
pub struct OpProgress {
    op_id: String,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU32>,
    total: Arc<AtomicU32>,
}

impl OpProgress {
    /// The registered op id — the worker uses it to name its staging dir so
    /// sweeps and status reports correlate.
    pub fn op_id(&self) -> &str {
        &self.op_id
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Bump the total when the real file count becomes known mid-run
    /// (e.g. after archive expansion).
    pub fn set_total(&self, total: u32) {
        self.total.store(total, Ordering::Relaxed);
    }

    pub fn add_done(&self, count: u32) {
        self.done.fetch_add(count, Ordering::Relaxed);
    }

    /// The atomics `import_markdown_folder_inner` wants.
    pub(crate) fn counters(&self) -> (&AtomicBool, &AtomicU32) {
        (&self.cancel, &self.done)
    }
}

fn ops() -> &'static Mutex<HashMap<String, OpEntry>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, OpEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn snapshot(entry: &OpEntry) -> ImportOperation {
    let mut snapshot = entry.snapshot.clone();
    snapshot.done_files = entry.done.load(Ordering::Relaxed);
    snapshot.total_files = entry.total.load(Ordering::Relaxed);
    snapshot
}

/// Register + spawn the worker on a plain thread. `work` receives the
/// progress handle and returns the JSON-serializable import payload; the
/// `IMPORT_CANCELLED` error string maps to `cancelled`, anything else to
/// `failed`.
pub fn begin<F>(kind: &str, label: &str, work: F) -> Result<ImportOperation, String>
where
    F: FnOnce(&OpProgress) -> Result<serde_json::Value, String> + Send + 'static,
{
    evict_finished_ops();
    let op_id = uuid::Uuid::new_v4().to_string();
    let entry = OpEntry {
        snapshot: ImportOperation {
            op_id: op_id.clone(),
            kind: kind.to_string(),
            label: label.to_string(),
            status: ImportStatus::Running,
            total_files: 0,
            done_files: 0,
            message: None,
            result: None,
            finished_at: None,
        },
        cancel: Arc::new(AtomicBool::new(false)),
        done: Arc::new(AtomicU32::new(0)),
        total: Arc::new(AtomicU32::new(0)),
    };
    let progress = OpProgress {
        op_id: op_id.clone(),
        cancel: entry.cancel.clone(),
        done: entry.done.clone(),
        total: entry.total.clone(),
    };
    ops()
        .lock()
        .map_err(|_| "import operation registry is poisoned".to_string())?
        .insert(op_id.clone(), entry);
    let worker_id = op_id.clone();
    std::thread::spawn(move || {
        let result = work(&progress);
        let mut registry = match ops().lock() {
            Ok(registry) => registry,
            Err(_) => return,
        };
        let Some(entry) = registry.get_mut(&worker_id) else {
            return;
        };
        entry.snapshot.finished_at = Some(chrono::Utc::now().to_rfc3339());
        match result {
            Ok(result) => {
                entry.snapshot.status = ImportStatus::Succeeded;
                entry.snapshot.result = Some(result);
            }
            Err(message) if message == crate::importer::IMPORT_CANCELLED => {
                entry.snapshot.status = ImportStatus::Cancelled;
                entry.snapshot.message = Some(message);
            }
            Err(message) => {
                entry.snapshot.status = ImportStatus::Failed;
                entry.snapshot.message = Some(message);
            }
        }
    });
    // The just-registered snapshot: done/total read 0 either way.
    status(&op_id).ok_or_else(|| "import operation was not registered".to_string())
}

/// Poll an op. `None` means the id is unknown or was evicted.
pub fn status(op_id: &str) -> Option<ImportOperation> {
    let registry = ops().lock().ok()?;
    let entry = registry.get(op_id)?;
    Some(snapshot(entry))
}

/// Cooperative cancel: flags the op so the copy loop aborts between files.
/// Already-terminal ops report their current state (idempotent).
pub fn cancel(op_id: &str) -> Result<ImportOperation, String> {
    let mut registry = ops()
        .lock()
        .map_err(|_| "import operation registry is poisoned".to_string())?;
    let Some(entry) = registry.get_mut(op_id) else {
        return Err(format!("Unknown import operation: {op_id}"));
    };
    if entry.snapshot.status == ImportStatus::Running {
        entry.cancel.store(true, Ordering::Relaxed);
    }
    Ok(snapshot(entry))
}

/// Evict the oldest finished ops beyond the retention cap. Called after an
/// op reaches a terminal state — safe to run on every begin too.
fn evict_finished_ops() {
    let mut registry = match ops().lock() {
        Ok(registry) => registry,
        Err(_) => return,
    };
    if registry.len() <= MAX_RETAINED_OPS {
        return;
    }
    let mut finished: Vec<(String, String)> = registry
        .iter()
        .filter(|(_, entry)| entry.snapshot.status != ImportStatus::Running)
        .map(|(id, entry)| {
            (
                id.clone(),
                entry.snapshot.finished_at.clone().unwrap_or_default(),
            )
        })
        .collect();
    finished.sort_by(|left, right| left.1.cmp(&right.1));
    for (id, _) in finished
        .into_iter()
        .take(registry.len().saturating_sub(MAX_RETAINED_OPS))
    {
        registry.remove(&id);
    }
}

/// Bounded pre-count for the `directory` kind's progress total — mirrors
/// `importer::collect_files`'s skip rules (hidden entries, symlinks,
/// reparse points, depth cap) so `done/total` stay honest.
pub(crate) fn count_importable_files(dir: &Path) -> u32 {
    const MAX_DEPTH: usize = 64;
    let mut count = 0_u32;
    let mut stack = vec![(dir.to_path_buf(), 0_usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if crate::atomic_file::is_reparse_point(&metadata) {
                continue;
            }
            if file_type.is_dir() {
                stack.push((entry.path(), depth + 1));
            } else if file_type.is_file() {
                count = count.saturating_add(1);
            }
        }
    }
    count
}

/// The per-op staging dir. On Android SAF staging belongs in the private
/// app cache (`mobile-import-<op>`) so it dies with process death and shows
/// up in `temporary_content`; elsewhere a managed `Cache\imports` subtree.
pub(crate) fn staging_dir(
    locations: &crate::storage::StorageLocations,
    op_id: &str,
) -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    let dir = crate::storage::android_base_dirs()?
        .app_cache
        .join(format!("mobile-import-{op_id}"));
    #[cfg(not(target_os = "android"))]
    let dir = locations
        .cache_root
        .join("imports")
        .join(format!("import-{op_id}"));
    let dir = crate::atomic_file::long_path(&dir);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// Copy a picked local file list into `staging` (flat — files arrive from a
/// picker with no shared tree). Duplicate basenames get a `-N` suffix, dirs
/// and symlinks are refused, and the cancel flag is polled between files.
/// Returns the staged file count.
pub(crate) fn stage_local_files(
    paths: &[String],
    staging: &Path,
    progress: &OpProgress,
) -> Result<u32, String> {
    let mut staged = 0_u32;
    progress.set_total(paths.len() as u32);
    for source in paths {
        if progress.is_cancelled() {
            return Err(crate::importer::IMPORT_CANCELLED.to_string());
        }
        let path = crate::atomic_file::long_path(Path::new(source));
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("无法读取所选文件 {source}：{error}"))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || crate::atomic_file::is_reparse_point(&metadata)
        {
            return Err(format!("所选路径不是普通文件：{source}"));
        }
        if metadata.len() > MAX_ARCHIVE_EXPAND_BYTES {
            return Err(format!("所选文件超出大小上限：{source}"));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("所选文件名无效：{source}"))?;
        // A hostile `con.md`-style basename becomes a Win32 device node at
        // the destination — validate before copying.
        if !crate::contracts::is_safe_relative_path(name) {
            return Err(format!("所选文件名不安全：{name}"));
        }
        let mut target = staging.join(name);
        for attempt in 1.. {
            if !target.exists() {
                break;
            }
            let stem = Path::new(name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("file");
            let ext = Path::new(name)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("");
            let renamed = if ext.is_empty() {
                format!("{stem}-{attempt}")
            } else {
                format!("{stem}-{attempt}.{ext}")
            };
            target = staging.join(renamed);
            if attempt > 1000 {
                return Err(format!("所选文件名冲突无法消除：{name}"));
            }
        }
        crate::atomic_file::copy_file_synced(&path, &crate::atomic_file::long_path(&target))?;
        staged += 1;
        progress.add_done(1);
    }
    Ok(staged)
}

/// One staged file was picked that is actually an archive — expand it into
/// a child dir with zip-slip protection and total-size cap, then route the
/// expanded tree. Only `.zip` is an archive here; `.epub` routes directly.
/// `pub(crate)` for the `archive` begin_import kind, which expands a
/// user-picked path without re-staging it.
pub(crate) fn expand_archive(archive: &Path, staging: &Path) -> Result<PathBuf, String> {
    let expanded = staging.join("expanded");
    fs::create_dir_all(&expanded).map_err(|error| error.to_string())?;
    let file = fs::File::open(crate::atomic_file::long_path(archive))
        .map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err("ARCHIVE_TOO_MANY_ENTRIES".to_string());
    }
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        // `enclosed_name` refuses `..`/absolute/drive-letter members.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_ARCHIVE_EXPAND_BYTES {
            return Err("ARCHIVE_TOO_LARGE".to_string());
        }
        if !crate::contracts::is_safe_relative_path(&name.to_string_lossy()) {
            continue;
        }
        let target = crate::atomic_file::long_path(&expanded.join(&name));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut out = fs::File::create(&target).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|error| error.to_string())?;
    }
    Ok(expanded)
}

/// Route a staged dir to the right importer:
/// - exactly one regular file ending `.epub` → `epub::import_epub_file`;
/// - exactly one `.zip`/`.cbz` → expand then route again on the tree;
/// - anything else → the Markdown folder importer (siblings preserved).
///
/// Returns the serializable import outcome.
pub(crate) fn route_staged(
    staging: &Path,
    title: Option<&str>,
    locations: &crate::storage::StorageLocations,
    progress: &OpProgress,
) -> Result<serde_json::Value, String> {
    let entries = fs::read_dir(staging)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let files: Vec<PathBuf> = entries
        .iter()
        .filter(|entry| {
            entry
                .file_type()
                .map(|ty| ty.is_file() && !ty.is_symlink())
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .collect();
    if files.len() == 1 && entries.len() == 1 {
        let extension = files[0]
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if extension == "epub" {
            let outcome = crate::epub::import_epub_file(&files[0], &locations.library_root, title)?;
            return serde_json::to_value(&outcome).map_err(|error| error.to_string());
        }
        if matches!(extension.as_str(), "zip" | "cbz") {
            let expanded = expand_archive(&files[0], staging)?;
            return route_staged(&expanded, title, locations, progress);
        }
    }
    let (cancel, done) = progress.counters();
    let outcome = crate::importer::import_markdown_folder_inner(
        staging,
        &locations.library_root,
        title,
        Some(cancel),
        Some(done),
    )?;
    serde_json::to_value(&outcome).map_err(|error| error.to_string())
}

/// Worker-body helper shared by all begin_import kinds: run `body`, then
/// always remove the op's staging dir before reporting the terminal state —
/// residue must not outlive the operation that created it.
pub(crate) fn run_with_staging<F>(
    locations: &crate::storage::StorageLocations,
    op_id: &str,
    body: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(&Path) -> Result<serde_json::Value, String>,
{
    let staging = staging_dir(locations, op_id)?;
    let result = body(&staging);
    if let Err(error) = fs::remove_dir_all(&staging) {
        crate::storage::app_log(
            "import_ops",
            &format!("staging dir {} cleanup failed: {error}", staging.display()),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_terminal(op_id: &str) -> ImportOperation {
        for _ in 0..100 {
            let op = status(op_id).expect("op must exist");
            if op.status != ImportStatus::Running {
                return op;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("operation never reached a terminal state");
    }

    #[test]
    fn operation_runs_to_succeeded_with_result() {
        let op = begin("directory", "test-op", |_progress| {
            Ok(serde_json::json!({"bookId": "b1"}))
        })
        .expect("begin");
        let finished = wait_terminal(&op.op_id);
        assert_eq!(finished.status, ImportStatus::Succeeded);
        assert_eq!(
            finished.result.as_ref().and_then(|v| v.get("bookId")),
            Some(&serde_json::json!("b1"))
        );
        assert!(finished.finished_at.is_some());
    }

    #[test]
    fn operation_failure_is_terminal_with_message() {
        let op = begin("files", "failing", |_progress| Err("boom".to_string())).expect("begin");
        let finished = wait_terminal(&op.op_id);
        assert_eq!(finished.status, ImportStatus::Failed);
        assert_eq!(finished.message.as_deref(), Some("boom"));
    }

    #[test]
    fn cancelled_flag_maps_to_cancelled_state() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let op = begin("directory", "cancellable", move |progress| {
            // Park until the test cancels, then observe the flag.
            rx.recv().ok();
            if progress.is_cancelled() {
                return Err(crate::importer::IMPORT_CANCELLED.to_string());
            }
            Err("should have been cancelled".to_string())
        })
        .expect("begin");
        let mid = cancel(&op.op_id).expect("cancel must succeed");
        assert_eq!(mid.status, ImportStatus::Running);
        tx.send(()).ok();
        let finished = wait_terminal(&op.op_id);
        assert_eq!(finished.status, ImportStatus::Cancelled);
        assert_eq!(
            finished.message.as_deref(),
            Some(crate::importer::IMPORT_CANCELLED)
        );
    }

    #[test]
    fn cancel_is_idempotent_on_terminal_ops() {
        let op = begin("files", "done", |_progress| Ok(serde_json::json!({}))).expect("begin");
        let finished = wait_terminal(&op.op_id);
        let again = cancel(&op.op_id).expect("cancel on finished op is a no-op");
        assert_eq!(again.status, finished.status);
    }

    #[test]
    fn progress_counters_flow_through() {
        let op = begin("files", "progress", |progress| {
            progress.set_total(3);
            progress.add_done(2);
            Ok(serde_json::json!({}))
        })
        .expect("begin");
        let finished = wait_terminal(&op.op_id);
        assert_eq!(finished.total_files, 3);
        assert_eq!(finished.done_files, 2);
    }

    #[test]
    fn unknown_op_status_and_cancel_error() {
        assert!(status("no-such-op").is_none());
        assert!(cancel("no-such-op").is_err());
    }

    #[test]
    fn stage_local_files_copies_and_dedupes_names() {
        let root = std::env::temp_dir().join(format!(
            "ir-stage-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let staging = root.join("staging");
        fs::create_dir_all(&staging).unwrap();
        let a = root.join("a.txt");
        let b_dir = root.join("other");
        fs::create_dir_all(&b_dir).unwrap();
        let b = b_dir.join("a.txt");
        fs::write(&a, "first").unwrap();
        fs::write(&b, "second").unwrap();
        let progress = OpProgress {
            op_id: "test-op".to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicU32::new(0)),
            total: Arc::new(AtomicU32::new(0)),
        };
        let staged = stage_local_files(
            &[
                a.to_string_lossy().into_owned(),
                b.to_string_lossy().into_owned(),
            ],
            &staging,
            &progress,
        )
        .expect("staging must succeed");
        assert_eq!(staged, 2);
        assert!(staging.join("a.txt").is_file());
        // The second same-named file lands under a deduped name.
        assert_eq!(
            fs::read_dir(&staging)
                .unwrap()
                .filter_map(Result::ok)
                .count(),
            2
        );
        fs::remove_dir_all(root).unwrap();
    }
}
