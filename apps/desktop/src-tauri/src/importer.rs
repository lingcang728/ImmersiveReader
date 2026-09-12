use crate::contracts::{Chapter, Manifest};
use crate::library::LibraryIssue;
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// P2-20: bound the source-tree walk — pathological nesting (or a junction
/// that slipped past the symlink check) must not recurse forever.
const MAX_IMPORT_DEPTH: usize = 64;

/// Staging directory for in-flight imports, under `手动/.incoming`. Hidden
/// (dot-prefixed) so `library::collect_manifests` never shelves a half-built
/// book, and swept at startup so crash residue disappears instead of
/// forcing the next import onto a `title (2)` duplicate (P2-19).
const IMPORT_STAGING_DIR: &str = ".incoming";

/// Import result: the manifest stays at the top level (flattened) so the
/// existing frontend — which only reads `bookId` — is unaffected, while
/// per-file problems surface in `issues` instead of failing the import
/// wholesale (P2-17).
#[derive(Clone, Debug, Serialize)]
pub struct ImportOutcome {
    #[serde(flatten)]
    pub manifest: Manifest,
    pub issues: Vec<LibraryIssue>,
}

fn issue(issues: &mut Vec<LibraryIssue>, path: &Path, message: impl Into<String>) {
    issues.push(LibraryIssue {
        path: path.to_string_lossy().into_owned(),
        message: message.into(),
    });
}

/// P2-17: per-entry tolerance — an unreadable directory/entry is recorded in
/// `issues` and skipped; it no longer fails the entire scan.
fn collect_markdown(
    root: &Path,
    dir: &Path,
    output: &mut Vec<(String, PathBuf)>,
    issues: &mut Vec<LibraryIssue>,
    depth: usize,
) {
    if depth > MAX_IMPORT_DEPTH {
        issue(issues, dir, "IMPORT_DEPTH_LIMIT");
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            issue(issues, dir, format!("目录无法读取：{error}"));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issue(issues, dir, format!("目录条目无法读取：{error}"));
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                issue(issues, &entry.path(), format!("条目类型无法读取：{error}"));
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        // P3-14: dot-prefixed entries are hidden from the library scan
        // (`collect_manifests` skips them) and `trash::parse_relative`
        // refuses dot-leading segments — importing one would create an
        // invisible, undeletable book. Hidden files are not book content.
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if file_type.is_dir() {
            // P2-20: junctions/mount points are reparse points that
            // `is_symlink` misses — a junction loop would otherwise recurse
            // up to the depth cap through the same tree.
            let reparse = match entry.metadata() {
                Ok(metadata) => crate::atomic_file::is_reparse_point(&metadata),
                Err(error) => {
                    issue(
                        issues,
                        &entry.path(),
                        format!("条目元数据无法读取：{error}"),
                    );
                    continue;
                }
            };
            if reparse {
                continue;
            }
            collect_markdown(root, &entry.path(), output, issues, depth + 1);
            continue;
        }
        let is_markdown = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| matches!(value.to_ascii_lowercase().as_str(), "md" | "markdown"))
            .unwrap_or(false);
        if !is_markdown {
            continue;
        }
        let relative = match entry.path().strip_prefix(root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(error) => {
                issue(issues, &entry.path(), format!("相对路径解析失败：{error}"));
                continue;
            }
        };
        output.push((relative, entry.path()));
    }
}

fn stable_chapter_id(relative: &str) -> String {
    let normalized = relative.to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("manual:{digest:x}")
}

fn unique_target(manual_root: &Path, title: &str) -> PathBuf {
    // P3-14: the shelf name must stay visible to the library scan and
    // deletable through `trash::parse_relative`. Strip characters Win32 would
    // silently normalize — leading dots hide the directory from the scan and
    // trailing dots/spaces are trimmed on create — and sidestep reserved
    // device names (a folder literally named `CON` cannot be created).
    let mut base = title
        .trim_matches(|c: char| c == '.' || c == ' ')
        .to_string();
    if base.is_empty() {
        base = "未命名书目".to_string();
    } else if crate::contracts::is_reserved_device_name(&base) {
        base.push('_');
    }
    let direct = manual_root.join(&base);
    if !crate::atomic_file::long_path(&direct).exists() {
        return direct;
    }
    for suffix in 2..10_000 {
        let candidate = manual_root.join(format!("{base} ({suffix})"));
        if !crate::atomic_file::long_path(&candidate).exists() {
            return candidate;
        }
    }
    manual_root.join(format!("{base}-{}", Uuid::new_v4()))
}

fn title_from_path(relative: &str) -> String {
    Path::new(relative)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(relative)
        .to_string()
}

/// P2-19: remove leftover `手动/.incoming` staging directories. Runs at
/// startup — before any import can be in flight — so the only residue it can
/// delete is from a previous crash.
pub fn sweep_staging_dirs(library_root: &Path) {
    let staging_root =
        crate::atomic_file::long_path(&library_root.join("手动").join(IMPORT_STAGING_DIR));
    if !staging_root.is_dir() {
        return;
    }
    let entries = match fs::read_dir(&staging_root) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!(
                "import staging sweep could not list {}: {error}",
                staging_root.display()
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Err(error) = fs::remove_dir_all(crate::atomic_file::long_path(&path)) {
            eprintln!(
                "import staging sweep could not remove {}: {error}",
                path.display()
            );
        }
    }
}

pub fn import_markdown_folder(source: &Path, library_root: &Path) -> Result<ImportOutcome, String> {
    // P2-20: normalize the roots once — every path derived below inherits
    // the `\\?\` spelling when the tree lives near/over MAX_PATH.
    let source = &crate::atomic_file::long_path(source);
    if !source.is_dir() {
        return Err("Import source must be a folder".to_string());
    }
    let mut files = Vec::new();
    let mut issues = Vec::new();
    collect_markdown(source, source, &mut files, &mut issues, 0);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err("The selected folder contains no Markdown files".to_string());
    }

    let title = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("未命名书目")
        .to_string();
    let manual_root = crate::atomic_file::long_path(&library_root.join("手动"));
    fs::create_dir_all(&manual_root).map_err(|error| error.to_string())?;
    let target = unique_target(&manual_root, &title);

    // P2-19: stage under 手动/.incoming/<uuid>, manifest written last, then
    // finalize with a same-directory rename. A crash mid-import leaves a
    // hidden, sweepable staging dir — never an invisible manifest-less
    // orphan on the shelf path.
    let staging_root = crate::atomic_file::long_path(&manual_root.join(IMPORT_STAGING_DIR));
    fs::create_dir_all(&staging_root).map_err(|error| error.to_string())?;
    let staging = crate::atomic_file::long_path(&staging_root.join(Uuid::new_v4().to_string()));
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;

    let staged = (|| -> Result<Manifest, String> {
        let mut chapters = Vec::with_capacity(files.len());
        for (relative, path) in files {
            // P2-18: the reader caps Markdown at 64 MiB — importing a larger
            // file would create a chapter that can never open, so skip it
            // with an issue instead of failing the whole import.
            let oversized = fs::metadata(&path)
                .map(|metadata| metadata.len() > crate::MAX_MARKDOWN_FILE_BYTES)
                .unwrap_or(false);
            if oversized {
                issue(&mut issues, &path, "MARKDOWN_FILE_TOO_LARGE");
                continue;
            }
            let destination = crate::atomic_file::long_path(
                &staging.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR)),
            );
            let copied = (|| -> Result<(), String> {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::copy(&path, &destination).map_err(|error| error.to_string())?;
                Ok(())
            })();
            // P2-17: one unreadable/uncopyable file is an issue, not a
            // failed import.
            if let Err(error) = copied {
                issue(&mut issues, &path, format!("文件复制失败：{error}"));
                continue;
            }
            let word_count = fs::read_to_string(&path)
                .map(|content| {
                    content
                        .chars()
                        .filter(|value| !value.is_whitespace())
                        .count() as u64
                })
                .unwrap_or(0);
            chapters.push(Chapter {
                id: stable_chapter_id(&relative),
                path: relative.clone(),
                title: title_from_path(&relative),
                date: None,
                vote_count: 0,
                word_count,
                metadata_status: None,
            });
        }
        if chapters.is_empty() {
            return Err("The selected folder contains no readable Markdown files".to_string());
        }
        let now = Utc::now().to_rfc3339();
        let manifest = Manifest {
            schema_version: 1,
            book_id: Uuid::new_v4().to_string(),
            title,
            source: "manual".to_string(),
            source_id: None,
            generated_at: now.clone(),
            updated_at: now,
            chapters,
        };
        let data = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
        // atomic_file::write normalizes the path itself; still cheap.
        crate::atomic_write_file(&staging.join("manifest.json"), &data)?;
        Ok(manifest)
    })();

    match staged {
        Ok(manifest) => {
            // Finalize: same-volume rename onto the shelf is atomic — the
            // book either appears complete with its manifest or not at all.
            let final_target = crate::atomic_file::long_path(&target);
            if let Err(error) = fs::rename(&staging, &final_target) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("Import finalize failed: {error}"));
            }
            Ok(ImportOutcome { manifest, issues })
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::import_markdown_folder;
    use std::fs;

    #[test]
    fn imports_without_moving_source_files() {
        let root = std::env::temp_dir().join(format!("immersive-import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source-book");
        let library = root.join("library");
        fs::create_dir_all(source.join("part")).expect("source must be created");
        fs::write(source.join("02.md"), "second").expect("fixture must write");
        fs::write(source.join("part/01.md"), "first").expect("fixture must write");
        let outcome = import_markdown_folder(&source, &library).expect("import must succeed");
        assert_eq!(outcome.manifest.chapters.len(), 2);
        assert!(outcome.issues.is_empty());
        assert!(source.join("02.md").exists());
        assert!(library.join("手动/source-book/manifest.json").exists());
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn written_manifest_satisfies_the_shared_contract() {
        // P1-21 regression: the manifest.json Rust writes must validate under
        // the shared schema and TS parseManifest — optional fields are omitted,
        // never serialized as explicit nulls.
        let root =
            std::env::temp_dir().join(format!("immersive-import-contract-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source-book");
        let library = root.join("library");
        fs::create_dir_all(&source).expect("source must be created");
        fs::write(source.join("01.md"), "first").expect("fixture must write");
        import_markdown_folder(&source, &library).expect("import must succeed");

        let raw = fs::read_to_string(library.join("手动/source-book/manifest.json"))
            .expect("manifest must be written");
        let value: serde_json::Value =
            serde_json::from_str(&raw).expect("written manifest must be JSON");
        assert!(value.get("sourceId").is_none(), "no null sourceId");
        let chapters = value["chapters"].as_array().expect("chapters array");
        assert!(!chapters.is_empty());
        for chapter in chapters {
            assert!(chapter.get("date").is_none(), "no null date");
            assert!(chapter.get("metadataStatus").is_none(), "no null metadataStatus");
            assert!(chapter.get("voteCount").is_some());
            assert!(chapter.get("wordCount").is_some());
        }
        // The written JSON must deserialize and pass the shared validator.
        let manifest: crate::contracts::Manifest =
            serde_json::from_str(&raw).expect("written manifest must deserialize");
        crate::contracts::validate_manifest(&manifest)
            .expect("written manifest must satisfy the shared contract");
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn staging_directory_is_finalized_and_leaves_no_incoming_residue() {
        // P2-19: a finished import must leave no `.incoming` staging behind,
        // and the staging area must be hidden from the shelf.
        let root =
            std::env::temp_dir().join(format!("immersive-import-staging-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source-book");
        let library = root.join("library");
        fs::create_dir_all(&source).expect("source must be created");
        fs::write(source.join("01.md"), "first").expect("fixture must write");

        import_markdown_folder(&source, &library).expect("import must succeed");

        let staging_root = library.join("手动/.incoming");
        let leftover = fs::read_dir(&staging_root)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "staging must be moved or removed");
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn import_skips_overly_deep_files_instead_of_failing() {
        // P2-17 + P2-20 depth cap: a file nested past MAX_IMPORT_DEPTH is
        // reported as an issue and skipped — the import itself succeeds.
        let root =
            std::env::temp_dir().join(format!("immersive-import-tolerant-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source-book");
        let library = root.join("library");
        fs::create_dir_all(&source).expect("source must be created");
        fs::write(source.join("01.md"), "first").expect("fixture must write");
        let mut deep = source.clone();
        for _ in 0..(super::MAX_IMPORT_DEPTH + 2) {
            deep = deep.join("d");
        }
        fs::create_dir_all(&deep).expect("deep tree must be created");
        fs::write(deep.join("buried.md"), "too deep").expect("fixture must write");

        let outcome = import_markdown_folder(&source, &library).expect("import must succeed");
        assert_eq!(outcome.manifest.chapters.len(), 1);
        assert!(outcome
            .issues
            .iter()
            .any(|issue| issue.message == "IMPORT_DEPTH_LIMIT"));
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn dot_prefixed_source_and_entries_stay_shelvable_and_deletable() {
        // P3-14: a `.`-prefixed source dir used to land as `手动/.foo` —
        // invisible to the scan and undeletable via trash::parse_relative.
        // Now the shelf name is sanitized and dot entries inside are skipped.
        let root =
            std::env::temp_dir().join(format!("immersive-import-dot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join(".hidden-book");
        let library = root.join("library");
        fs::create_dir_all(source.join(".git")).expect("source must be created");
        fs::write(source.join("01.md"), "visible").expect("fixture must write");
        fs::write(source.join(".git/x.md"), "hidden").expect("fixture must write");
        fs::write(source.join(".dotfile.md"), "hidden file").expect("fixture must write");

        let outcome = import_markdown_folder(&source, &library).expect("import must succeed");

        assert_eq!(outcome.manifest.chapters.len(), 1);
        assert_eq!(outcome.manifest.chapters[0].path, "01.md");
        // The sanitized shelf dir is visible to the scan and deletable.
        let shelved = library.join("手动/hidden-book");
        assert!(shelved.join("manifest.json").exists());
        let scan = crate::library::scan_library(&library).expect("scan");
        assert_eq!(scan.books.len(), 1);
        crate::trash::move_book(&library, &shelved, &outcome.manifest)
            .expect("imported book must be trashable");
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn sweep_removes_staging_residue() {
        // P2-19: startup sweep deletes abandoned `.incoming` trees but leaves
        // finished books untouched.
        let root =
            std::env::temp_dir().join(format!("immersive-import-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        let staging = library.join("手动/.incoming/orphaned-staging");
        fs::create_dir_all(&staging).expect("staging fixture must be created");
        fs::write(staging.join("partial.md"), "half-copied").expect("fixture must write");
        let book = library.join("手动/finished-book");
        fs::create_dir_all(&book).expect("book fixture must be created");
        fs::write(book.join("manifest.json"), "{}").expect("fixture must write");

        super::sweep_staging_dirs(&library);

        assert!(!library.join("手动/.incoming/orphaned-staging").exists());
        assert!(book.join("manifest.json").exists());
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }
}
