use crate::contracts::{validate_manifest, Manifest, ReadingProgress};
use crate::progress::load_progress;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetail {
    pub manifest: Manifest,
    pub progress: ReadingProgress,
}

fn collect_manifests(dir: &Path, depth: usize, manifests: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > 3 || !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_file() && entry.file_name() == "manifest.json" {
            manifests.push(entry.path());
        } else if file_type.is_dir() {
            let name = entry.file_name();
            // Skip recycle bin and hidden control dirs so removed books stay off the shelf.
            if name == ".trash" || name.to_string_lossy().starts_with('.') {
                continue;
            }
            collect_manifests(&entry.path(), depth + 1, manifests)?;
        }
    }
    Ok(())
}

fn ensure_book_inside_library(library_root: &Path, book_root: &Path) -> Result<(), String> {
    let canonical_library = library_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let canonical_book = book_root
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
    let (book_root, manifest, _) = find_book(root, book_id)?;
    ensure_book_inside_library(root, &book_root)?;
    crate::trash::move_book(root, &book_root, &manifest)?;
    Ok(format!("已移出书架：{}（可在回收站恢复）", manifest.title))
}

/// Permanently delete a book directory from disk. Irreversible.
pub fn delete_book(root: &Path, book_id: &str) -> Result<String, String> {
    let (book_root, manifest, _) = find_book(root, book_id)?;
    ensure_book_inside_library(root, &book_root)?;
    let chapter_count = manifest.chapters.len();
    fs::remove_dir_all(&book_root).map_err(|error| error.to_string())?;
    Ok(format!(
        "已永久删除《{}》（{} 篇）",
        manifest.title, chapter_count
    ))
}

fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let manifest: Manifest = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn progress_value(manifest: &Manifest, progress: &ReadingProgress) -> f64 {
    if manifest.chapters.is_empty() {
        return 0.0;
    }
    let read_count = progress
        .read
        .iter()
        .filter(|id| manifest.chapters.iter().any(|chapter| &chapter.id == *id))
        .count();
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
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let writable = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(root.join(".write-test"))
        .and_then(|_| fs::remove_file(root.join(".write-test")))
        .is_ok();
    let mut paths = Vec::new();
    collect_manifests(root, 0, &mut paths)?;
    paths.sort();
    let mut books = Vec::new();
    let mut issues = Vec::new();
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
    let mut paths = Vec::new();
    collect_manifests(root, 0, &mut paths)?;
    for path in paths {
        let Ok((manifest, progress)) = load_book_at(&path) else {
            continue;
        };
        if manifest.book_id == book_id {
            let book_root = path
                .parent()
                .ok_or_else(|| "Manifest has no book directory".to_string())?;
            return Ok((book_root.to_path_buf(), manifest, progress));
        }
    }
    Err(format!("Book not found: {book_id}"))
}

pub fn open_book(root: &Path, book_id: &str) -> Result<BookDetail, String> {
    let (_, manifest, progress) = find_book(root, book_id)?;
    Ok(BookDetail { manifest, progress })
}

pub fn find_book_by_source_id(root: &Path, source_id: &str) -> Result<Option<Manifest>, String> {
    let mut paths = Vec::new();
    collect_manifests(root, 0, &mut paths)?;
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
    let canonical_root = book_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let canonical_file = candidate
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !canonical_file.starts_with(canonical_root) {
        return Err("Chapter resolves outside its book directory".to_string());
    }
    Ok(canonical_file)
}

pub fn save_book_progress(
    root: &Path,
    book_id: &str,
    progress: &ReadingProgress,
) -> Result<(), String> {
    let (book_root, manifest, _) = find_book(root, book_id)?;
    crate::progress::save_progress(&book_root, &manifest, progress)
}

#[cfg(test)]
mod tests {
    use super::{delete_book, progress_value, remove_book, scan_library};
    use crate::contracts::{Manifest, ReadingProgress};
    use std::fs;
    use std::path::PathBuf;
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

    fn write_fixture_book(library: &PathBuf, book_id: &str, folder: &str) {
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
}
