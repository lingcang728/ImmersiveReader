use crate::contracts::{Chapter, Manifest};
use crate::library::LibraryIssue;
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32};
use uuid::Uuid;

/// P2-20: bound the source-tree walk — pathological nesting (or a junction
/// that slipped past the symlink check) must not recurse forever.
const MAX_IMPORT_DEPTH: usize = 64;

/// A4: non-chapter sibling files (images, attachments) copied alongside the
/// chapters. Beyond this they get an issue instead of a copy — a book dir
/// must not silently ingest gigabyte-sized binaries.
const MAX_SIBLING_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Cooperative cancellation flag check used by the import pipeline
/// (`import_ops`): the copy loop returns this error between files so a
/// cancelled operation stops instead of running to completion.
pub(crate) const IMPORT_CANCELLED: &str = "IMPORT_CANCELLED";

/// Chapter-bearing extensions. `.txt` is imported as a chapter file kept
/// under its real name — `get_readable_chapter` serves it as Markdown and
/// each one records a `TXT_AS_MARKDOWN` issue so the choice is auditable.
fn is_chapter_extension(extension: &str) -> bool {
    matches!(extension, "md" | "markdown" | "txt")
}

/// A4 natural ordering: digit runs compare numerically so `ch2` sorts before
/// `ch10` instead of after it lexically. Digit runs of equal numeric value
/// fall through to the next segment; overall equality defers to the plain
/// byte compare so ordering stays total and deterministic.
pub(crate) fn natural_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let (mut li, mut ri) = (0_usize, 0_usize);
    while li < left_bytes.len() && ri < right_bytes.len() {
        let (lb, rb) = (left_bytes[li], right_bytes[ri]);
        if lb.is_ascii_digit() && rb.is_ascii_digit() {
            let l_start = li;
            while li < left_bytes.len() && left_bytes[li].is_ascii_digit() {
                li += 1;
            }
            let r_start = ri;
            while ri < right_bytes.len() && right_bytes[ri].is_ascii_digit() {
                ri += 1;
            }
            // Compare as integers without overflow: strip leading zeros,
            // then longer digit runs win, then lexicographic order.
            let l_digits = left_bytes[l_start..li]
                .iter()
                .skip_while(|byte| **byte == b'0')
                .copied()
                .collect::<Vec<_>>();
            let r_digits = right_bytes[r_start..ri]
                .iter()
                .skip_while(|byte| **byte == b'0')
                .copied()
                .collect::<Vec<_>>();
            let ordering = l_digits
                .len()
                .cmp(&r_digits.len())
                .then_with(|| l_digits.cmp(&r_digits));
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
            continue;
        }
        if lb != rb {
            return lb.cmp(&rb);
        }
        li += 1;
        ri += 1;
    }
    (li < left_bytes.len())
        .cmp(&(ri < right_bytes.len()))
        .then_with(|| left.cmp(right))
}

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
/// A4: every regular file is collected (not just chapters) — non-chapter
/// siblings are copied into the book dir preserving relative structure so
/// images and attachments referenced by the Markdown keep resolving.
fn collect_files(
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
            collect_files(root, &entry.path(), output, issues, depth + 1);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        // Reparse-point files (cloud placeholders, dedup links) are not real
        // book content — copying one could fetch remote bytes or resolve
        // outside the tree. Same rule as the dir branch: skip silently.
        match entry.metadata() {
            Ok(metadata) if crate::atomic_file::is_reparse_point(&metadata) => continue,
            Ok(_) => {}
            Err(error) => {
                issue(
                    issues,
                    &entry.path(),
                    format!("条目元数据无法读取：{error}"),
                );
                continue;
            }
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

/// `pub(crate)` for the reading-bundle restore, which needs the same
/// shelf-name sanitizer + ` (N)` dedup under an arbitrary library dir.
pub(crate) fn unique_target(manual_root: &Path, title: &str) -> PathBuf {
    // P3-14: the shelf name must stay visible to the library scan and
    // deletable through `trash::parse_relative`. Strip characters Win32 would
    // silently normalize — leading dots hide the directory from the scan and
    // trailing dots/spaces are trimmed on create — and sidestep reserved
    // device names (a folder literally named `CON` cannot be created).
    // P-11-F11: the same sanitizer podcast publish uses — a source folder
    // named `a:b|c?` (producible via `\\?\`/WSL/cloud sync) used to die on
    // `create_dir_all` as ERROR_INVALID_NAME and fail the whole import.
    let mut base = crate::contracts::sanitize_shelf_name(title, "未命名书目");
    if crate::contracts::is_reserved_device_name(&base) {
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
    // Junction/mount-point guard: `is_dir()` follows reparse points, so a
    // `.incoming` swapped for a junction would make read_dir enumerate the
    // *target's* children and remove_dir_all delete them outside the library.
    // A reparse-point root gets unlinked (remove_dir drops the link, never
    // the target) instead of traversed; the same applies per entry.
    let root_meta = match fs::symlink_metadata(&staging_root) {
        Ok(meta) => meta,
        Err(_) => return,
    };
    if crate::atomic_file::is_reparse_point(&root_meta) {
        if let Err(error) = fs::remove_dir(&staging_root) {
            eprintln!(
                "import staging sweep could not unlink reparse point {}: {error}",
                staging_root.display()
            );
        }
        return;
    }
    if !root_meta.is_dir() {
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
        let is_reparse = entry
            .metadata()
            .map(|meta| crate::atomic_file::is_reparse_point(&meta))
            .unwrap_or(false);
        let result = if is_reparse {
            // Unlink the link itself; never recurse through it.
            fs::remove_dir(crate::atomic_file::long_path(&path))
        } else {
            fs::remove_dir_all(crate::atomic_file::long_path(&path))
        };
        if let Err(error) = result {
            eprintln!(
                "import staging sweep could not remove {}: {error}",
                path.display()
            );
        }
    }
}

pub fn import_markdown_folder(source: &Path, library_root: &Path) -> Result<ImportOutcome, String> {
    import_markdown_folder_titled(source, library_root, None)
}

/// `import_markdown_folder` with a caller-chosen title override (the import
/// pipeline uses it for staged/uri imports whose source dir name is an
/// opaque `import-<opid>`). `None` keeps the source directory name.
pub fn import_markdown_folder_titled(
    source: &Path,
    library_root: &Path,
    title: Option<&str>,
) -> Result<ImportOutcome, String> {
    import_markdown_folder_inner(source, library_root, title, None, None)
}

/// Shared import pipeline.
/// - `cancel`: checked between files; aborts with `IMPORT_CANCELLED` and the
///   staging dir is removed, so a cancelled op never shelves a partial book.
/// - `done_files`: bumped once per processed source file — the operation
///   status poller reads it without touching the copy loop.
pub(crate) fn import_markdown_folder_inner(
    source: &Path,
    library_root: &Path,
    title: Option<&str>,
    cancel: Option<&AtomicBool>,
    done_files: Option<&AtomicU32>,
) -> Result<ImportOutcome, String> {
    // P2-20: normalize the roots once — every path derived below inherits
    // the `\\?\` spelling when the tree lives near/over MAX_PATH.
    let source = &crate::atomic_file::long_path(source);
    if !source.is_dir() {
        return Err("Import source must be a folder".to_string());
    }
    let mut files = Vec::new();
    let mut issues = Vec::new();
    collect_files(source, source, &mut files, &mut issues, 0);
    // A4: natural ordering — `ch2` before `ch10` — instead of byte order.
    files.sort_by(|left, right| natural_cmp(&left.0, &right.0));
    if files.is_empty() {
        return Err("The selected folder contains no importable files".to_string());
    }
    let has_chapters = files.iter().any(|(_, path)| {
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| is_chapter_extension(&value.to_ascii_lowercase()))
            .unwrap_or(false)
    });
    if !has_chapters {
        return Err("The selected folder contains no Markdown files".to_string());
    }

    let title = title.map(str::to_string).unwrap_or_else(|| {
        source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("未命名书目")
            .to_string()
    });
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
            // A2: cooperative cancellation — the op pipeline's cancel flag
            // is polled between files so a cancelled import stops promptly.
            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
                return Err(IMPORT_CANCELLED.to_string());
            }
            if let Some(counter) = done_files {
                // Counts processed files (copied or skipped) — a skipped
                // file must not stall the operation's progress bar.
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let is_chapter = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| is_chapter_extension(&value.to_ascii_lowercase()))
                .unwrap_or(false);
            // Size gates differ per role: a chapter file is capped at the
            // reader's 64 MiB (P2-18 — a larger chapter could never open);
            // non-chapter siblings get the wider asset cap, then an issue.
            let file_len = fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            if is_chapter && file_len > crate::MAX_MARKDOWN_FILE_BYTES {
                issue(&mut issues, &path, "MARKDOWN_FILE_TOO_LARGE");
                continue;
            }
            if !is_chapter && file_len > MAX_SIBLING_FILE_BYTES {
                issue(&mut issues, &path, "IMPORT_FILE_TOO_LARGE");
                continue;
            }
            // A source file named `con.md`/`nul.md` (producible via `\\?\`,
            // WSL or zip) resolves to a Win32 device node at the destination
            // — the copy fails with a confusing OS error. Check the relative
            // path up front so the file is reported with a precise issue.
            // Every copied file needs this — not just chapters.
            if !crate::contracts::is_safe_relative_path(&relative) {
                issue(&mut issues, &path, "UNSAFE_RELATIVE_PATH");
                continue;
            }
            let destination = crate::atomic_file::long_path(
                &staging.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR)),
            );
            let copied = (|| -> Result<(), String> {
                // P-10-F21/F4: fs::copy leaves bytes in the write cache and
                // a source edited mid-copy yields a torn chapter under a
                // valid name — copy through the synced helper and reject a
                // byte-count mismatch instead of shelving it.
                let expected = fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .len();
                let written = crate::atomic_file::copy_file_synced(&path, &destination)?;
                if written != expected {
                    return Err(format!(
                        "torn copy ({written} of {expected} bytes — source changed during import)"
                    ));
                }
                Ok(())
            })();
            // P2-17: one unreadable/uncopyable file is an issue, not a
            // failed import.
            if let Err(error) = copied {
                issue(&mut issues, &path, format!("文件复制失败：{error}"));
                continue;
            }
            if !is_chapter {
                // A4: non-chapter sibling copied for its relative-path
                // references; it never becomes a chapter.
                continue;
            }
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("txt"))
            {
                issue(&mut issues, &path, "TXT_AS_MARKDOWN");
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
            return Err(
                "The selected folder contains no readable Markdown or TXT files".to_string(),
            );
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
        // Self-check before persisting: a manifest that fails validation
        // would land on the shelf as a permanently broken book.
        crate::contracts::validate_manifest(&manifest)
            .map_err(|error| format!("Import produced an invalid manifest: {error}"))?;
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
            assert!(
                chapter.get("metadataStatus").is_none(),
                "no null metadataStatus"
            );
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

    #[test]
    fn natural_cmp_orders_digit_runs_numerically() {
        use std::cmp::Ordering;
        assert_eq!(super::natural_cmp("ch2.md", "ch10.md"), Ordering::Less);
        assert_eq!(super::natural_cmp("ch10.md", "ch2.md"), Ordering::Greater);
        assert_eq!(super::natural_cmp("ch2.md", "ch2.md"), Ordering::Equal);
        // Equal numeric value with different zero padding falls to a stable
        // deterministic order (byte compare), never a panic or a tie.
        assert_ne!(super::natural_cmp("ch02.md", "ch2.md"), Ordering::Equal);
        // Segments after the digit run still compare.
        assert_eq!(super::natural_cmp("ch2a.md", "ch2b.md"), Ordering::Less);
        assert_eq!(super::natural_cmp("x.md", "ch10.md"), Ordering::Greater);
    }

    #[test]
    fn import_preserves_non_markdown_siblings_and_txt_chapters() {
        // A4: images/attachments ship inside the book dir so relative links
        // in the Markdown resolve; `.txt` files become chapters served as
        // Markdown (recorded via the TXT_AS_MARKDOWN issue).
        let root =
            std::env::temp_dir().join(format!("immersive-import-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source-book");
        let library = root.join("library");
        fs::create_dir_all(source.join("assets")).expect("source must be created");
        fs::write(source.join("02.md"), "second").expect("fixture must write");
        fs::write(source.join("10.txt"), "plain text chapter").expect("fixture must write");
        fs::write(source.join("assets/cover.png"), b"png").expect("fixture must write");
        fs::write(source.join("notes.pdf"), b"pdf").expect("fixture must write");

        let outcome = import_markdown_folder(&source, &library).expect("import must succeed");
        let book = library.join("手动/source-book");
        assert_eq!(outcome.manifest.chapters.len(), 2);
        // Natural order: 02.md then 10.txt.
        assert_eq!(outcome.manifest.chapters[0].path, "02.md");
        assert_eq!(outcome.manifest.chapters[1].path, "10.txt");
        assert!(book.join("assets/cover.png").exists(), "asset copied");
        assert!(book.join("notes.pdf").exists(), "sibling file copied");
        assert!(outcome
            .issues
            .iter()
            .any(|issue| issue.message == "TXT_AS_MARKDOWN"));
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn import_honours_cancel_flag_between_files() {
        let root =
            std::env::temp_dir().join(format!("immersive-import-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source-book");
        let library = root.join("library");
        fs::create_dir_all(&source).expect("source must be created");
        fs::write(source.join("01.md"), "first").expect("fixture must write");
        let cancel = std::sync::atomic::AtomicBool::new(true);

        let error =
            super::import_markdown_folder_inner(&source, &library, None, Some(&cancel), None)
                .expect_err("cancelled import must abort");

        assert_eq!(error, super::IMPORT_CANCELLED);
        // No half-built book on the shelf.
        assert!(!library.join("手动/source-book").exists());
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }

    #[test]
    fn titled_import_uses_the_callers_title() {
        let root =
            std::env::temp_dir().join(format!("immersive-import-title-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("staged-dir-name");
        let library = root.join("library");
        fs::create_dir_all(&source).expect("source must be created");
        fs::write(source.join("01.md"), "first").expect("fixture must write");

        let outcome = super::import_markdown_folder_titled(&source, &library, Some("自选书名"))
            .expect("import must succeed");

        assert_eq!(outcome.manifest.title, "自选书名");
        assert!(library.join("手动/自选书名/manifest.json").exists());
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }
}
