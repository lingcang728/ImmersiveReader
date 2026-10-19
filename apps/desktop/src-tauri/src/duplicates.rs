//! Conservative duplicate repair: equal chapter structure and payload bytes only.
use crate::{contracts::Manifest, library, reader_db::ReaderDb};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepairReport {
    pub groups: Vec<String>,
    pub duplicate_count: usize,
    pub merged_count: usize,
}
fn collect_payload(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, PathBuf)>,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 || files.len() > 10000 {
        return Err("书目资源过多，跳过合并".into());
    }
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || crate::atomic_file::is_reparse_point(&metadata) {
            return Err("含链接资源的书目保留为独立副本".into());
        }
        if metadata.is_dir() {
            collect_payload(root, &path, files, depth + 1)?;
        } else if metadata.is_file() {
            files.push((
                path.strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/"),
                path,
            ));
        }
    }
    Ok(())
}
fn signature(dir: &Path, manifest: &Manifest) -> Result<String, String> {
    let mut files = Vec::new();
    collect_payload(dir, dir, &mut files, 0)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hash = Sha256::new();
    for chapter in &manifest.chapters {
        hash.update(chapter.id.as_bytes());
        hash.update([0]);
        hash.update(chapter.path.as_bytes());
    }
    for (relative, path) in files {
        if matches!(
            relative.as_str(),
            "manifest.json"
                | ".reading.json"
                | ".source-sha256"
                | "provenance.json"
                | "trash-entry.json"
        ) {
            continue;
        }
        hash.update([0]);
        hash.update(relative.as_bytes());
        hash.update([0]);
        let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
        std::io::copy(&mut file, &mut hash).map_err(|e| e.to_string())?;
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn stamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|at| at.timestamp_millis())
        .unwrap_or(0)
}
pub(crate) fn repair(root: &Path, apply: bool) -> Result<RepairReport, String> {
    let _imports = crate::open_inputs::import_lock()
        .lock()
        .map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    library::collect_manifests(root, 0, &mut paths, &mut Vec::new(), false);
    paths.sort();
    let db = ReaderDb::open_current()?;
    let mut groups: BTreeMap<String, Vec<(PathBuf, Manifest, i64)>> = BTreeMap::new();
    for path in paths {
        let Some(dir) = path.parent() else {
            continue;
        };
        let Ok(manifest) = library::read_manifest(&path) else {
            continue;
        };
        let Ok(key) = signature(dir, &manifest) else {
            continue;
        };
        let progress = crate::progress::load_progress(dir, &manifest)?;
        let locator_stamp = db
            .load_locator(&manifest.book_id)?
            .and_then(|raw| serde_json::from_str::<crate::contracts::ReaderLocator>(&raw).ok())
            .map(|loc| stamp(&loc.updated))
            .unwrap_or(0);
        groups.entry(key).or_default().push((
            dir.into(),
            manifest,
            stamp(&progress.updated).max(locator_stamp),
        ));
    }
    let mut report = RepairReport {
        groups: Vec::new(),
        duplicate_count: 0,
        merged_count: 0,
    };
    for group in groups.values_mut().filter(|group| group.len() > 1) {
        group.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        let (canonical_dir, canonical, _) = &group[0];
        report.groups.push(canonical.title.clone());
        report.duplicate_count += group.len() - 1;
        if !apply {
            continue;
        }
        let mut progress = crate::progress::load_progress(canonical_dir, canonical)?;
        let mut locator: Option<crate::contracts::ReaderLocator> = None;
        for (dir, manifest, _) in group.iter() {
            progress =
                library::merge_progress(&progress, &crate::progress::load_progress(dir, manifest)?);
            if let Some(raw) = db.load_locator(&manifest.book_id)? {
                if let Ok(mut found) = serde_json::from_str::<crate::contracts::ReaderLocator>(&raw)
                {
                    if crate::contracts::validate_reader_locator(&found).is_ok()
                        && canonical
                            .chapters
                            .iter()
                            .any(|ch| ch.id == found.chapter_id)
                        && match locator.as_ref() {
                            None => true,
                            Some(old) => stamp(&found.updated) > stamp(&old.updated),
                        }
                    {
                        found.book_id = canonical.book_id.clone();
                        locator = Some(found);
                    }
                }
            }
            if manifest.book_id != canonical.book_id {
                for bookmark in db.list_bookmarks(&manifest.book_id)? {
                    let Ok(mut saved) = serde_json::from_str::<crate::contracts::ReaderLocator>(
                        &bookmark.locator_json,
                    ) else {
                        continue;
                    };
                    if crate::contracts::validate_reader_locator(&saved).is_err() {
                        continue;
                    }
                    saved.book_id = canonical.book_id.clone();
                    let id = format!(
                        "merged-{:x}",
                        Sha256::digest(
                            format!("{}:{}", manifest.book_id, bookmark.bookmark_id).as_bytes()
                        )
                    );
                    db.add_bookmark(
                        &canonical.book_id,
                        &id,
                        &serde_json::to_string(&saved).map_err(|e| e.to_string())?,
                        &bookmark.label,
                    )?;
                }
            }
        }
        if !progress.updated.is_empty() {
            crate::progress::save_progress(canonical_dir, canonical, &progress)?;
        }
        if let Some(locator) = locator {
            db.save_locator(
                &canonical.book_id,
                &serde_json::to_string(&locator).map_err(|e| e.to_string())?,
            )?;
        }
        for (dir, manifest, _) in group.iter().skip(1) {
            // Recheck immediately before the recoverable move; modified user content is retained as a separate book.
            if signature(dir, manifest)? != signature(canonical_dir, canonical)? {
                continue;
            }
            crate::trash::move_book(root, dir, manifest)?;
            report.merged_count += 1;
        }
    }
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_books_merge_latest_progress_and_keep_recoverable_originals() {
        let root = std::env::temp_dir().join(format!("ir-dedupe-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let library = root.join("Library");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("one.md"), "# 合成重复书\n正文。").unwrap();
        let first =
            crate::importer::import_markdown_folder_titled(&source, &library, Some("第一份"))
                .unwrap();
        let second =
            crate::importer::import_markdown_folder_titled(&source, &library, Some("第二份"))
                .unwrap();
        let (dir, manifest, mut progress) =
            library::book_context(&library, &second.manifest.book_id).unwrap();
        progress.position = 0.73;
        progress.updated = "2026-10-18T13:00:00Z".into();
        crate::progress::save_progress(&dir, &manifest, &progress).unwrap();
        assert_eq!(repair(&library, false).unwrap().duplicate_count, 1);
        assert_eq!(repair(&library, true).unwrap().merged_count, 1);
        let scan = library::scan_library(&library).unwrap();
        assert_eq!(scan.books.len(), 1);
        assert_eq!(scan.books[0].book_id, second.manifest.book_id);
        assert_eq!(scan.books[0].progress, 0.73);
        assert_eq!(crate::trash::list(&library).unwrap().len(), 1);
        let _ = ReaderDb::open_current()
            .unwrap()
            .remove_book(&first.manifest.book_id);
        let _ = ReaderDb::open_current()
            .unwrap()
            .remove_book(&second.manifest.book_id);
        fs::remove_dir_all(root).unwrap();
    }
}
