use crate::contracts::{validate_manifest, Manifest, ReadingProgress};
use crate::progress::load_progress;
use crate::tasks::TaskSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSummary {
    pub book_id: String,
    pub title: String,
    pub source: String,
    pub chapter_count: usize,
    pub read_count: usize,
    pub progress: f64,
    pub current_chapter_title: Option<String>,
    pub updated_at: String,
    pub last_read_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryIssue {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryScan {
    pub books: Vec<BookSummary>,
    pub issues: Vec<LibraryIssue>,
    pub writable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookProvenance {
    // Integral floats (`1.0`) are valid per the schema `const`; keep the read
    // set identical to manifest/reading schemaVersion handling.
    #[serde(deserialize_with = "crate::contracts::deserialize_schema_version")]
    pub schema_version: u32,
    pub book_id: String,
    pub source_id: Option<String>,
    pub source_kind: Option<String>,
    pub created_by_task_id: Option<String>,
    pub last_successful_task_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::contracts::deserialize_optional_u64"
    )]
    pub revision: Option<u64>,
    pub manifest_sha256: Option<String>,
    pub engine_version: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetail {
    pub manifest: Manifest,
    pub progress: ReadingProgress,
    pub provenance: Option<BookProvenance>,
    pub task_records: Vec<TaskSnapshot>,
}

/// P2-17: per-entry tolerance — a directory/entry that cannot be read is
/// recorded in `issues` and skipped instead of failing the whole scan.
/// P2-20: the depth cap keeps the recursion bounded.
/// Returns how many regular files sit DIRECTLY in `dir` — the caller uses it
/// to flag a directory that holds its own content but produced no manifest
/// anywhere in its subtree (a half-deleted book whose manifest vanished is
/// invisible on the shelf yet still occupies disk; report it instead of
/// silently keeping the orphan). `inside_book` marks subtrees beneath a dir
/// that already yielded a manifest — chapter/asset dirs inside a book are
/// never books themselves and must not be flagged.
fn collect_manifests(
    dir: &Path,
    depth: usize,
    manifests: &mut Vec<PathBuf>,
    issues: &mut Vec<LibraryIssue>,
    inside_book: bool,
) -> usize {
    if depth > 3 {
        issues.push(LibraryIssue {
            path: dir.to_string_lossy().into_owned(),
            message: "LIBRARY_DEPTH_LIMIT".to_string(),
        });
        return 0;
    }
    if !dir.exists() {
        return 0;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            issues.push(LibraryIssue {
                path: dir.to_string_lossy().into_owned(),
                message: format!("目录无法读取：{error}"),
            });
            return 0;
        }
    };
    // This dir IS a book when it carries its own manifest — subdirectory
    // content is then book payload, never a candidate book or orphan. A
    // root-level manifest is rejected below as phantom, so it does not count.
    let self_is_book = inside_book || (depth > 0 && dir.join("manifest.json").is_file());
    let mut own_files = 0_usize;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(LibraryIssue {
                    path: dir.to_string_lossy().into_owned(),
                    message: format!("目录条目无法读取：{error}"),
                });
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                issues.push(LibraryIssue {
                    path: entry.path().to_string_lossy().into_owned(),
                    message: format!("条目类型无法读取：{error}"),
                });
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        // A junction is a reparse point, not a symlink — `is_symlink()` does
        // not catch it. Skipping reparse points keeps an out-of-library
        // junction from being collected as a book (it would be readable yet
        // undeletable: ensure_book_inside_library rejects the resolved root).
        match entry.metadata() {
            Ok(metadata) if crate::atomic_file::is_reparse_point(&metadata) => continue,
            Ok(_) => {}
            Err(error) => {
                issues.push(LibraryIssue {
                    path: entry.path().to_string_lossy().into_owned(),
                    message: format!("条目元数据无法读取：{error}"),
                });
                continue;
            }
        }
        if file_type.is_file() {
            own_files += 1;
            if entry.file_name() == "manifest.json" {
                // A manifest directly at the library root would make the root
                // itself a phantom book: visible and readable (including
                // .trash contents via chapter paths) but impossible to remove.
                if entry.path().parent() == Some(dir) && depth == 0 {
                    issues.push(LibraryIssue {
                        path: entry.path().to_string_lossy().into_owned(),
                        message: "manifest.json 必须位于书籍子目录，书库根的清单已忽略".to_string(),
                    });
                    continue;
                }
                manifests.push(entry.path());
            }
        } else if file_type.is_dir() {
            let name = entry.file_name();
            // Skip recycle bin and hidden control dirs so removed books stay off the shelf.
            if name == ".trash" || name.to_string_lossy().starts_with('.') {
                continue;
            }
            let before = manifests.len();
            let child_files =
                collect_manifests(&entry.path(), depth + 1, manifests, issues, self_is_book);
            if !self_is_book && manifests.len() == before && child_files > 0 {
                issues.push(LibraryIssue {
                    path: entry.path().to_string_lossy().into_owned(),
                    message: "目录内有文件但缺少 manifest.json——可能是不完整删除的残留".to_string(),
                });
            }
        }
    }
    own_files
}

fn ensure_book_inside_library(library_root: &Path, book_root: &Path) -> Result<(), String> {
    // P2-20: normalize before canonicalize — an un-prefixed path past
    // MAX_PATH fails to even open without longPathAware.
    let canonical_library = crate::atomic_file::long_path(library_root)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let canonical_book = crate::atomic_file::long_path(book_root)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !canonical_book.starts_with(&canonical_library) {
        return Err("Book resolves outside the library root".to_string());
    }
    if canonical_book == canonical_library {
        return Err("Refusing to operate on the library root".to_string());
    }
    Ok(())
}

pub fn remove_book(root: &Path, book_id: &str) -> Result<String, String> {
    let root = &crate::atomic_file::long_path(root);
    // 与 PUT /progress 互斥（F18）：在途写入若在 rename 后才落盘，会把已删
    // 书目录重建为只剩 `.reading.json` 的幽灵目录。拿到锁再删，排队的 PUT
    // 随后会因 manifest 不存在而拒绝写入。
    let lock = book_progress_lock(book_id)?;
    let _guard = lock
        .lock()
        .map_err(|_| "Progress lock is poisoned".to_string())?;
    let (book_root, manifest, _) = find_book(root, book_id)?;
    ensure_book_inside_library(root, &book_root)?;
    crate::trash::move_book(root, &book_root, &manifest)?;
    Ok(format!("已移出书架：{}（可在回收站恢复）", manifest.title))
}

/// Permanently delete a book directory from disk. Irreversible.
/// P-10-F3: route the delete through the trash pipeline so it is journaled
/// end to end — a bare `remove_dir_all` can die mid-walk and leave a
/// manifest-less orphan dir that the shelf silently hides. `move_book`
/// journals the rename into `.trash/<id>`; `permanently_delete` then journals
/// the final removal, and either phase is finished by the next reconcile.
pub fn delete_book(root: &Path, book_id: &str) -> Result<String, String> {
    let root = &crate::atomic_file::long_path(root);
    // 同 remove_book：持 progress 锁再动目录，挡住在途 PUT（F18）。
    let lock = book_progress_lock(book_id)?;
    let _guard = lock
        .lock()
        .map_err(|_| "Progress lock is poisoned".to_string())?;
    let (book_root, manifest, _) = find_book(root, book_id)?;
    ensure_book_inside_library(root, &book_root)?;
    let chapter_count = manifest.chapters.len();
    let entry = crate::trash::move_book(root, &book_root, &manifest)?;
    crate::trash::permanently_delete(root, &entry.trash_id, entry.revision)
        .map_err(|error| format!("书籍已移入回收站但最终删除失败：{error}"))?;
    Ok(format!(
        "已永久删除《{}》（{} 篇）",
        manifest.title, chapter_count
    ))
}

/// P2-18: metadata pre-check before slurping — a corrupt/hostile manifest
/// or provenance file must not pull an unbounded blob into memory. Shares
/// the reader's 64 MiB ceiling.
fn read_json_file_capped(path: &Path) -> Result<String, String> {
    let normalized = crate::atomic_file::long_path(path);
    let metadata = fs::metadata(&normalized).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Not a file".to_string());
    }
    if metadata.len() > crate::MAX_MARKDOWN_FILE_BYTES {
        return Err("FILE_TOO_LARGE".to_string());
    }
    fs::read_to_string(&normalized).map_err(|error| error.to_string())
}

pub(crate) fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let raw = read_json_file_capped(path)?;
    match serde_json::from_str::<Manifest>(&raw) {
        Ok(manifest) => {
            validate_manifest(&manifest)?;
            Ok(manifest)
        }
        Err(error) => repair_legacy_null_manifest(path, &raw, error),
    }
}

/// Manifests written before the explicit-null fix carry `"sourceId": null`
/// (and per-chapter `"date"/"metadataStatus": null`) — keys the schema
/// requires to be omitted. Rather than leaving those books unloadable,
/// strip nulls on the known optional-string fields, persist the canonical
/// serialization once, and re-validate. Any other parse error is reported
/// unchanged.
fn repair_legacy_null_manifest(
    path: &Path,
    raw: &str,
    original: serde_json::Error,
) -> Result<Manifest, String> {
    const OPTIONAL_STRING_FIELDS: &[&str] = &["sourceId", "date", "metadataStatus"];
    fn strip_nulls(object: &mut serde_json::Map<String, serde_json::Value>) -> bool {
        let mut changed = false;
        for key in OPTIONAL_STRING_FIELDS {
            if object.get(*key).is_some_and(|value| value.is_null()) {
                object.remove(*key);
                changed = true;
            }
        }
        changed
    }

    let mut value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| original.to_string())?;
    let object = value.as_object_mut().ok_or_else(|| original.to_string())?;
    let mut changed = strip_nulls(object);
    if let Some(chapters) = object.get_mut("chapters").and_then(|c| c.as_array_mut()) {
        for chapter in chapters.iter_mut().filter_map(|c| c.as_object_mut()) {
            changed |= strip_nulls(chapter);
        }
    }
    if !changed {
        return Err(original.to_string());
    }
    let manifest: Manifest = serde_json::from_value(value).map_err(|_| original.to_string())?;
    validate_manifest(&manifest)?;
    let canonical = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    crate::atomic_file::write(path, canonical.as_bytes())?;
    Ok(manifest)
}

fn read_provenance(
    book_root: &Path,
    expected_book_id: &str,
) -> Result<Option<BookProvenance>, String> {
    let path = crate::atomic_file::long_path(&book_root.join("provenance.json"));
    if !path.exists() {
        return Ok(None);
    }
    let raw = read_json_file_capped(&path)?;
    let provenance: BookProvenance =
        serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    if provenance.schema_version != 1 || provenance.book_id != expected_book_id {
        return Err("Book provenance does not match its manifest".to_string());
    }
    Ok(Some(provenance))
}

fn progress_value(manifest: &Manifest, progress: &ReadingProgress) -> f64 {
    if manifest.chapters.is_empty() {
        return 0.0;
    }
    // `read` is a set — count each known chapter id once even if a dirty
    // array repeats it (mirrors the TS `Set` semantics in
    // packages/contracts `progressValue`).
    let read_count = progress
        .read
        .iter()
        .filter(|id| manifest.chapters.iter().any(|chapter| &chapter.id == *id))
        .collect::<std::collections::HashSet<_>>()
        .len();
    let current_is_read = progress.read.contains(&progress.current);
    let current_exists = manifest
        .chapters
        .iter()
        .any(|chapter| chapter.id == progress.current);
    let fractional = if current_exists && !current_is_read {
        progress.position
    } else {
        0.0
    };
    ((read_count as f64 + fractional) / manifest.chapters.len() as f64).min(1.0)
}

fn load_book_at(manifest_path: &Path) -> Result<(Manifest, ReadingProgress), String> {
    let manifest = read_manifest(manifest_path)?;
    let root = manifest_path
        .parent()
        .ok_or_else(|| "Manifest has no book directory".to_string())?;
    let progress = load_progress(root, &manifest)?;
    Ok((manifest, progress))
}

pub fn scan_library(root: &Path) -> Result<LibraryScan, String> {
    // P2-20: normalize the root once — every path the walk derives inherits
    // the `\\?\` spelling when the library lives near/over MAX_PATH.
    let root = &crate::atomic_file::long_path(root);
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    // P3-17: unique probe name — a fixed `.write-test` could clobber a real
    // user file of the same name, and concurrent scans could race on it.
    // `create_new` never opens foreign content; the probe is removed after.
    let probe = root.join(format!(".write-test-{}", uuid::Uuid::new_v4().simple()));
    let writable = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
        .is_ok();
    if writable {
        let _ = fs::remove_file(&probe);
    }
    let mut paths = Vec::new();
    let mut issues = Vec::new();
    collect_manifests(root, 0, &mut paths, &mut issues, false);
    paths.sort();
    let mut books = Vec::new();
    for path in paths {
        match load_book_at(&path) {
            Ok((manifest, progress)) => {
                let title = manifest
                    .chapters
                    .iter()
                    .find(|chapter| chapter.id == progress.current)
                    .map(|chapter| chapter.title.clone());
                books.push(BookSummary {
                    book_id: manifest.book_id.clone(),
                    title: manifest.title.clone(),
                    source: manifest.source.clone(),
                    chapter_count: manifest.chapters.len(),
                    read_count: progress.read.len(),
                    progress: progress_value(&manifest, &progress),
                    current_chapter_title: title,
                    updated_at: manifest.updated_at,
                    last_read_at: (!progress.updated.is_empty()).then_some(progress.updated),
                });
            }
            Err(message) => issues.push(LibraryIssue {
                path: path.to_string_lossy().into_owned(),
                message,
            }),
        }
    }
    books.sort_by(|left, right| {
        right
            .last_read_at
            .cmp(&left.last_read_at)
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(LibraryScan {
        books,
        issues,
        writable,
    })
}

fn find_book(root: &Path, book_id: &str) -> Result<(PathBuf, Manifest, ReadingProgress), String> {
    // P2-17: a broken sibling directory must not hide a book — skip-and-scan
    // behaviour is already how unreadable manifests are handled below.
    let mut paths = Vec::new();
    collect_manifests(
        &crate::atomic_file::long_path(root),
        0,
        &mut paths,
        &mut Vec::new(),
        false,
    );
    // P3-17: when two shelf dirs carry the same book_id, resolve
    // deterministically — the first match in sorted (lexicographic) manifest
    // path order wins, independent of filesystem enumeration order.
    paths.sort();
    for path in paths {
        let Ok(manifest) = read_manifest(&path) else {
            continue;
        };
        if manifest.book_id == book_id {
            let book_root = path
                .parent()
                .ok_or_else(|| "Manifest has no book directory".to_string())?;
            let progress = load_progress(book_root, &manifest)?;
            return Ok((book_root.to_path_buf(), manifest, progress));
        }
    }
    Err(format!("Book not found: {book_id}"))
}

pub fn open_book(root: &Path, book_id: &str) -> Result<BookDetail, String> {
    let (book_root, manifest, progress) = find_book(root, book_id)?;
    // P3-17: provenance is advisory metadata — a corrupt or mismatched
    // provenance.json degrades to `None` instead of killing the open.
    let provenance = match read_provenance(&book_root, &manifest.book_id) {
        Ok(provenance) => provenance,
        Err(error) => {
            eprintln!(
                "open_book: ignoring unreadable provenance for {}: {error}",
                manifest.book_id
            );
            None
        }
    };
    Ok(BookDetail {
        manifest,
        progress,
        provenance,
        task_records: Vec::new(),
    })
}

pub fn find_book_by_source_id(root: &Path, source_id: &str) -> Result<Option<Manifest>, String> {
    let mut paths = Vec::new();
    collect_manifests(
        &crate::atomic_file::long_path(root),
        0,
        &mut paths,
        &mut Vec::new(),
        false,
    );
    // P3-17: same deterministic ordering as find_book — first match in
    // sorted manifest path order wins.
    paths.sort();
    for path in paths {
        let Ok(manifest) = read_manifest(&path) else {
            continue;
        };
        if manifest.source_id.as_deref() == Some(source_id) {
            return Ok(Some(manifest));
        }
    }
    Ok(None)
}

pub fn book_context(
    root: &Path,
    book_id: &str,
) -> Result<(PathBuf, Manifest, ReadingProgress), String> {
    find_book(root, book_id)
}

pub fn chapter_path(root: &Path, book_id: &str, chapter_id: &str) -> Result<PathBuf, String> {
    let (book_root, manifest, _) = find_book(root, book_id)?;
    let chapter = manifest
        .chapters
        .iter()
        .find(|item| item.id == chapter_id)
        .ok_or_else(|| format!("Chapter not found: {chapter_id}"))?;
    let candidate = book_root.join(chapter.path.replace('/', std::path::MAIN_SEPARATOR_STR));
    // P3-19: `chapter.path` was already checked by `validate_manifest`
    // (`is_safe_relative_path`) inside `read_manifest` — device-name or
    // normalizable segments are refused before this `canonicalize` can open
    // them.
    let canonical_root = crate::atomic_file::long_path(&book_root)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let canonical_file = crate::atomic_file::long_path(&candidate)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !canonical_file.starts_with(canonical_root) {
        return Err("Chapter resolves outside its book directory".to_string());
    }
    Ok(canonical_file)
}

/// P2-21 read-merge-write: two reading surfaces (the Svelte 精读 workspace
/// via `save_book_progress` and the tiny_http 连读 reader via
/// `reader_http`'s PUT /progress) write the same `.reading.json`. A naive
/// last-write-wins store drops concurrent updates — most visibly `read`
/// marks from the other surface. Merge before writing: the `read` set is a
/// union, and the cursor (current/position/updated) goes to whichever side
/// saved most recently (`updated` is the RFC-3339 ordering key).
/// `pub(crate)` so the reader_http half of this fix can share the same merge.
pub(crate) fn merge_progress(
    existing: &ReadingProgress,
    incoming: &ReadingProgress,
) -> ReadingProgress {
    let mut merged = incoming.clone();
    for id in &existing.read {
        if !merged.read.iter().any(|known| known == id) {
            merged.read.push(id.clone());
        }
    }
    // RFC-3339 strings do not order chronologically under plain string
    // compare: "…T01:00:00.1Z" < "…T01:00:00Z" lexicographically yet is newer,
    // and "+08:00"/"-08:00" offsets vs "Z" are not comparable at all. Parse
    // first; fall back to the lexical order only when a side is unparseable.
    let existing_wins = match (
        chrono::DateTime::parse_from_rfc3339(&existing.updated),
        chrono::DateTime::parse_from_rfc3339(&merged.updated),
    ) {
        (Ok(existing_at), Ok(merged_at)) => existing_at > merged_at,
        _ => existing.updated > merged.updated,
    };
    if existing_wins {
        merged.current = existing.current.clone();
        merged.position = existing.position;
        merged.updated = existing.updated.clone();
    }
    merged
}

static PROGRESS_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

/// P-10-F9: shared per-book mutex for the `.reading.json`
/// load→merge→write section. `merge_progress` alone is not enough: two
/// writers can interleave read-then-write and the later write still drops
/// the earlier one's `read` marks. `pub(crate)` so the 连读 reader's
/// PUT /progress (reader_http) serializes against this surface's
/// `save_book_progress` — both must hold the same lock to be effective.
pub(crate) fn book_progress_lock(book_id: &str) -> Result<Arc<Mutex<()>>, String> {
    let locks = PROGRESS_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .map_err(|_| "Progress lock registry is poisoned".to_string())?;
    Ok(locks
        .entry(book_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
}

pub fn save_book_progress(
    root: &Path,
    book_id: &str,
    progress: &ReadingProgress,
) -> Result<(), String> {
    let lock = book_progress_lock(book_id)?;
    let _guard = lock
        .lock()
        .map_err(|_| "Progress lock is poisoned".to_string())?;
    let (book_root, manifest, existing) = find_book(root, book_id)?;
    crate::progress::save_progress(&book_root, &manifest, &merge_progress(&existing, progress))
}

#[cfg(test)]
mod tests {
    use super::{delete_book, progress_value, remove_book, scan_library};
    use crate::contracts::{Manifest, ReadingProgress};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_library(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ir-lib-{name}-{nanos}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("library root");
        root
    }

    fn write_fixture_book(library: &Path, book_id: &str, folder: &str) {
        let book = library.join("手动").join(folder);
        fs::create_dir_all(&book).expect("book dir");
        let mut manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture");
        manifest.book_id = book_id.to_string();
        manifest.title = folder.to_string();
        fs::write(
            book.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("json"),
        )
        .expect("write manifest");
        // Empty chapters paths are fine for remove/delete; scan only needs valid manifest.
    }

    #[test]
    fn progress_counts_current_unread_fraction() {
        let manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture must deserialize");
        let mut progress: ReadingProgress = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/reading.valid.json"
        ))
        .expect("fixture must deserialize");
        assert_eq!(progress_value(&manifest, &progress), 0.5);
        progress.read.push(progress.current.clone());
        assert_eq!(progress_value(&manifest, &progress), 1.0);
    }

    #[test]
    fn remove_book_moves_to_trash_and_hides_from_scan() {
        let root = temp_library("remove");
        write_fixture_book(&root, "manual:remove-me", "可移出");
        let scan = scan_library(&root).expect("scan");
        assert_eq!(scan.books.len(), 1);
        remove_book(&root, "manual:remove-me").expect("remove");
        let after = scan_library(&root).expect("scan after");
        assert!(after.books.is_empty());
        assert_eq!(crate::trash::list(&root).expect("trash list").len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_book_removes_directory() {
        let root = temp_library("delete");
        write_fixture_book(&root, "manual:delete-me", "可删除");
        let book_path = root.join("手动").join("可删除");
        assert!(book_path.exists());
        delete_book(&root, "manual:delete-me").expect("delete");
        assert!(!book_path.exists());
        let after = scan_library(&root).expect("scan after");
        assert!(after.books.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_collects_per_book_issues_instead_of_failing() {
        // P2-17: one corrupt book must not blank the shelf — it lands in
        // `issues` while healthy books still list.
        let root = temp_library("issues");
        write_fixture_book(&root, "manual:healthy", "健康书");
        let broken = root.join("手动").join("坏书");
        fs::create_dir_all(&broken).expect("broken book dir");
        fs::write(broken.join("manifest.json"), "{not json").expect("write broken manifest");

        let scan = scan_library(&root).expect("scan must tolerate the bad book");
        assert_eq!(scan.books.len(), 1);
        assert_eq!(scan.books[0].book_id, "manual:healthy");
        assert_eq!(scan.issues.len(), 1);
        assert!(scan.issues[0].path.contains("坏书"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_null_manifest_heals_itself_on_scan() {
        // Pre-null-fix writers persisted `"sourceId": null` etc.; those books
        // must load (and the file must be rewritten canonically) instead of
        // sitting in the issue list forever.
        let root = temp_library("null-heal");
        let book = root.join("手动").join("旧书");
        fs::create_dir_all(&book).expect("book dir");
        let mut manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture");
        manifest.book_id = "manual:legacy-null".to_string();
        let mut value = serde_json::to_value(&manifest).expect("to value");
        let object = value.as_object_mut().expect("object");
        object.insert("sourceId".to_string(), serde_json::Value::Null);
        let chapters = object
            .get_mut("chapters")
            .and_then(|c| c.as_array_mut())
            .expect("chapters");
        chapters[0]
            .as_object_mut()
            .expect("chapter")
            .insert("date".to_string(), serde_json::Value::Null);
        let manifest_path = book.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&value).expect("json"),
        )
        .expect("write manifest");

        let scan = scan_library(&root).expect("scan");
        assert_eq!(scan.books.len(), 1);
        assert_eq!(scan.books[0].book_id, "manual:legacy-null");
        assert!(scan.issues.is_empty(), "issues: {:?}", scan.issues);
        // The on-disk file is healed — a second read sees no nulls.
        let healed = fs::read_to_string(&manifest_path).expect("read healed");
        assert!(!healed.contains(": null"), "healed manifest: {healed}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn save_progress_merges_read_marks_across_surfaces() {
        // P2-21: the 精读 command surface and the 连读 HTTP surface write the
        // same .reading.json — a last-write-wins save must not drop the other
        // surface's `read` marks or yank the cursor backwards.
        let root = temp_library("progress-merge");
        let book = root.join("手动").join("合并进度");
        fs::create_dir_all(&book).expect("book dir");
        let mut manifest: Manifest = serde_json::from_str(include_str!(
            "../../../../packages/contracts/fixtures/manifest.valid.json"
        ))
        .expect("fixture");
        manifest.book_id = "manual:merge".to_string();
        let mut second = manifest.chapters[0].clone();
        second.id = "answer:fixture-2".to_string();
        second.path = "002.md".to_string();
        manifest.chapters.push(second);
        fs::write(
            book.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("json"),
        )
        .expect("manifest");

        // Surface A marks chapter 1 read at t1.
        let mut first = ReadingProgress::empty("answer:fixture-1");
        first.read = vec!["answer:fixture-1".to_string()];
        first.updated = "2026-07-10T01:00:00.000Z".to_string();
        super::save_book_progress(&root, "manual:merge", &first).expect("save A");

        // Surface B marks chapter 2 read at a later time, cursor on ch2.
        let mut latest = ReadingProgress::empty("answer:fixture-2");
        latest.read = vec!["answer:fixture-2".to_string()];
        latest.position = 0.25;
        latest.updated = "2026-07-10T02:00:00.000Z".to_string();
        super::save_book_progress(&root, "manual:merge", &latest).expect("save B");

        let merged = crate::progress::load_progress(&book, &manifest).expect("load merged");
        assert!(merged.read.contains(&"answer:fixture-1".to_string()));
        assert!(merged.read.contains(&"answer:fixture-2".to_string()));
        assert_eq!(merged.current, "answer:fixture-2");

        // A stale write (older `updated`) keeps contributing its read marks
        // but must not move the cursor back.
        let mut stale = ReadingProgress::empty("answer:fixture-1");
        stale.read = vec!["answer:fixture-1".to_string()];
        stale.position = 0.9;
        stale.updated = "2026-07-10T00:30:00.000Z".to_string();
        super::save_book_progress(&root, "manual:merge", &stale).expect("save stale");
        let merged = crate::progress::load_progress(&book, &manifest).expect("load stale merge");
        assert_eq!(merged.current, "answer:fixture-2");
        assert_eq!(merged.position, 0.25);
        let _ = fs::remove_dir_all(&root);
    }
}
