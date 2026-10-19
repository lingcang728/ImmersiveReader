//! One managed-copy path for file pickers, directory pickers and open-with.
use crate::{atomic_file, contracts::Manifest, import_ops::OpProgress, storage::StorageLocations};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub(crate) fn import_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hash.finalize()))
}
fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn markdown_title(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = fs::File::open(path) {
        let _ = file.take(64 * 1024).read_to_end(&mut bytes);
    }
    if let Ok((text, _)) = crate::decode_markdown_bytes(bytes) {
        if let Some(title) = text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("# ")
                .map(str::trim)
                .filter(|line| !line.is_empty())
        }) {
            return title.chars().take(180).collect();
        }
    }
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("未命名书目")
        .to_string()
}
fn markdown_assets(path: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut assets = Vec::new();
    let Some(parent) = path.parent() else {
        return Ok(assets);
    };
    let base = parent.canonicalize().map_err(|e| e.to_string())?;
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let (text, _) = crate::decode_markdown_bytes(bytes)?;
    let links = text
        .split("](")
        .skip(1)
        .filter_map(|part| part.split(')').next())
        .chain(
            text.split("src=\"")
                .skip(1)
                .filter_map(|part| part.split('"').next()),
        );
    for link in links.take(1024) {
        let link = link
            .trim()
            .trim_matches(['<', '>'])
            .split_whitespace()
            .next()
            .unwrap_or("");
        let decoded = percent_encoding::percent_decode_str(link).decode_utf8_lossy();
        let relative = decoded.split(['#', '?']).next().unwrap_or("");
        if !crate::contracts::is_safe_relative_path(relative) {
            continue;
        }
        let source = parent.join(relative);
        if !matches!(
            extension(&source).as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif"
        ) {
            continue;
        }
        let Ok(resolved) = source.canonicalize() else {
            continue;
        };
        let Ok(metadata) = fs::symlink_metadata(&source) else {
            continue;
        };
        if !resolved.starts_with(&base)
            || !metadata.is_file()
            || metadata.file_type().is_symlink()
            || atomic_file::is_reparse_point(&metadata)
            || metadata.len() > 256 * 1024 * 1024
        {
            continue;
        }
        assets.push((relative.to_string(), source));
    }
    assets.sort_by(|a, b| a.0.cmp(&b.0));
    assets.dedup_by(|a, b| a.0 == b.0);
    Ok(assets)
}
fn copy_markdown_assets(path: &Path, target: &Path) -> Result<(), String> {
    for (relative, source) in markdown_assets(path)? {
        let destination = target.join(relative);
        if let Some(dir) = destination.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        atomic_file::copy_file_synced(&source, &destination)?;
    }
    Ok(())
}
fn markdown_hash(path: &Path) -> Result<String, String> {
    let plain = file_hash(path)?;
    let assets = markdown_assets(path)?;
    if assets.is_empty() {
        return Ok(plain);
    }
    let mut hash = Sha256::new();
    hash.update(plain.as_bytes());
    for (relative, source) in assets {
        hash.update([0]);
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update(file_hash(&source)?.as_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn duplicate(root: &Path, hash: &str, single_markdown: bool) -> Option<(PathBuf, Manifest)> {
    let mut paths = Vec::new();
    crate::library::collect_manifests(root, 0, &mut paths, &mut Vec::new(), false);
    paths.sort();
    for path in paths {
        let dir = path.parent()?;
        let Ok(manifest) = crate::library::read_manifest(&path) else {
            continue;
        };
        let known = fs::read_to_string(dir.join(".source-sha256")).ok();
        let matches = known.as_deref() == Some(hash)
            || (known.is_none()
                && dir.join("source.epub").is_file()
                && file_hash(&dir.join("source.epub")).ok().as_deref() == Some(hash))
            || (known.is_none()
                && single_markdown
                && manifest.chapters.len() == 1
                && !dir.join("publication.json").exists()
                && markdown_hash(&dir.join(&manifest.chapters[0].path))
                    .ok()
                    .as_deref()
                    == Some(hash));
        if matches {
            return Some((dir.to_path_buf(), manifest));
        }
    }
    None
}

pub(crate) fn import_one_file(
    path: &Path,
    title: Option<&str>,
    locations: &StorageLocations,
    progress: &OpProgress,
) -> Result<serde_json::Value, String> {
    if progress.is_cancelled() {
        return Err(crate::importer::IMPORT_CANCELLED.to_string());
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || atomic_file::is_reparse_point(&metadata)
    {
        return Err("所选路径不是普通文件".to_string());
    }
    let _lock = import_lock().lock().map_err(|e| e.to_string())?;
    let ext = extension(path);
    let is_md = matches!(ext.as_str(), "md" | "markdown" | "txt");
    if !is_md && ext != "epub" {
        return Err("不支持的文件格式".to_string());
    }
    if is_md && metadata.len() > 64 * 1024 * 1024 {
        return Err("Markdown 文件超过 64 MiB".to_string());
    }
    let hash = if is_md {
        markdown_hash(path)?
    } else {
        file_hash(path)?
    };
    if let Some((dir, mut manifest)) = duplicate(&locations.library_root, &hash, is_md) {
        if is_md
            && (manifest.title.starts_with("mobile-import-")
                || manifest.title.starts_with("import-"))
        {
            manifest.title = title
                .map(str::to_string)
                .unwrap_or_else(|| markdown_title(path));
            atomic_file::write(
                &dir.join("manifest.json"),
                &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
            )?;
        }
        atomic_file::write(&dir.join(".source-sha256"), hash.as_bytes())?;
        return Ok(
            serde_json::json!({"bookId": manifest.book_id, "title": manifest.title, "reused": true}),
        );
    }
    let result = if ext == "epub" {
        serde_json::to_value(crate::epub::import_epub_file(
            path,
            &locations.library_root,
            title,
        )?)
        .map_err(|e| e.to_string())?
    } else {
        let isolated = locations
            .cache_root
            .join("imports")
            .join(format!(".single-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&isolated).map_err(|e| e.to_string())?;
        let result = (|| {
            atomic_file::copy_file_synced(
                path,
                &isolated.join(path.file_name().ok_or("文件名无效")?),
            )?;
            copy_markdown_assets(path, &isolated)?;
            let title = title
                .map(str::to_string)
                .unwrap_or_else(|| markdown_title(path));
            crate::importer::import_markdown_folder_titled(
                &isolated,
                &locations.library_root,
                Some(&title),
            )
        })();
        let _ = fs::remove_dir_all(&isolated);
        serde_json::to_value(result?).map_err(|e| e.to_string())?
    };
    if let Some(id) = result.get("bookId").and_then(|v| v.as_str()) {
        let (dir, manifest, _) = crate::library::book_context(&locations.library_root, id)?;
        if is_md {
            migrate_legacy_progress(path, &dir, &manifest)?;
        }
        atomic_file::write(&dir.join(".source-sha256"), hash.as_bytes())?;
    }
    Ok(result)
}

fn migrate_legacy_progress(source: &Path, dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let path = source
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_string();
    let candidates = [
        crate::state_path_for(&path),
        crate::legacy_hash_state_path_in_dir(&crate::state_dir(), &path),
        crate::legacy_hash_state_path_in_dir(&crate::legacy_state_dir(), &path),
    ];
    let Some(raw) = candidates
        .iter()
        .find_map(|p| crate::read_state_json_capped(p).ok())
    else {
        return Ok(());
    };
    let Ok(state) = serde_json::from_str::<crate::ReadingState>(&raw) else {
        return Ok(());
    };
    if !state.progress.is_finite() || !(0.0..=1.0).contains(&state.progress) {
        return Ok(());
    }
    let Some(chapter) = manifest.chapters.first() else {
        return Ok(());
    };
    let progress = crate::contracts::ReadingProgress {
        schema_version: 1,
        current: chapter.id.clone(),
        position: state.progress,
        read: Vec::new(),
        updated: chrono::Utc::now().to_rfc3339(),
    };
    crate::progress::save_progress(dir, manifest, &progress)?;
    let copied = dir
        .join(&chapter.path)
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_string();
    atomic_file::write(&crate::state_path_for(&copied), raw.as_bytes())?;
    Ok(())
}

pub(crate) fn import_batch(
    files: &[PathBuf],
    locations: &StorageLocations,
    progress: &OpProgress,
) -> Result<serde_json::Value, String> {
    let mut results = Vec::new();
    let mut ids = Vec::new();
    let mut issues = Vec::new();
    progress.set_total(files.len() as u32);
    for path in files {
        if progress.is_cancelled() {
            return Err(crate::importer::IMPORT_CANCELLED.to_string());
        }
        let imported = if path.is_dir() {
            import_directory(path, None, locations, progress)
        } else {
            import_one_file(path, None, locations, progress)
        };
        match imported {
            Ok(result) => {
                if let Some(id) = result.get("bookId").and_then(|v| v.as_str()) { ids.push(id.to_string()); }
                results.push(result);
            }
            Err(message) => issues.push(serde_json::json!({"path": path.file_name().map(|name| name.to_string_lossy()), "message": message})),
        }
        progress.add_done(1);
    }
    if ids.is_empty() {
        return Err(issues
            .first()
            .and_then(|v| v["message"].as_str())
            .unwrap_or("没有可阅读的文件")
            .to_string());
    }
    Ok(
        serde_json::json!({"bookId": if ids.len() == 1 { ids.first() } else { None }, "bookIds": ids, "results": results, "issues": issues}),
    )
}

pub(crate) fn import_directory(
    source: &Path,
    title: Option<&str>,
    locations: &StorageLocations,
    progress: &OpProgress,
) -> Result<serde_json::Value, String> {
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || atomic_file::is_reparse_point(&metadata)
    {
        return Err("所选路径不是普通文件夹".to_string());
    }
    let mut files = Vec::new();
    let mut issues = Vec::new();
    crate::importer::collect_files(source, source, &mut files, &mut issues, 0);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let epub_files: Vec<PathBuf> = files
        .iter()
        .filter(|(_, path)| extension(path) == "epub")
        .map(|(_, path)| path.clone())
        .collect();
    let has_markdown = files
        .iter()
        .any(|(_, path)| matches!(extension(path).as_str(), "md" | "markdown" | "txt"));
    if !has_markdown {
        return import_batch(&epub_files, locations, progress);
    }
    let _lock = import_lock().lock().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    for (relative, path) in files.iter().filter(|(_, path)| extension(path) != "epub") {
        if progress.is_cancelled() {
            return Err(crate::importer::IMPORT_CANCELLED.to_string());
        }
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update(file_hash(path)?.as_bytes());
    }
    let hash = format!("{:x}", hash.finalize());
    let mut result = if let Some((_, manifest)) = duplicate(&locations.library_root, &hash, false) {
        serde_json::json!({"bookId": manifest.book_id, "title": manifest.title, "reused": true})
    } else {
        let (cancel, done) = progress.counters();
        let outcome = crate::importer::import_markdown_folder_inner(
            source,
            &locations.library_root,
            title,
            Some(cancel),
            Some(done),
        )?;
        let (dir, _, _) =
            crate::library::book_context(&locations.library_root, &outcome.manifest.book_id)?;
        atomic_file::write(&dir.join(".source-sha256"), hash.as_bytes())?;
        serde_json::to_value(outcome).map_err(|e| e.to_string())?
    };
    drop(_lock);
    if !epub_files.is_empty() {
        let epub = match import_batch(&epub_files, locations, progress) {
            Ok(value) => value,
            Err(message) => serde_json::json!({"bookIds": [], "issues": [{"message": message}]}),
        };
        let mut ids = vec![result["bookId"].clone()];
        ids.extend(epub["bookIds"].as_array().cloned().unwrap_or_default());
        let mut all_issues = serde_json::to_value(&issues)
            .unwrap_or_default()
            .as_array()
            .cloned()
            .unwrap_or_default();
        all_issues.extend(epub["issues"].as_array().cloned().unwrap_or_default());
        result = serde_json::json!({"bookId": if ids.len() == 1 { ids.first() } else { None }, "bookIds": ids, "results": [result, epub], "issues": all_issues});
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn locations(base: &Path) -> StorageLocations {
        let value = StorageLocations::resolve_for(
            &crate::settings::AppChannel::Qa("open-tests".into()),
            base,
            base,
            base,
            &base.join("runtime"),
        );
        fs::create_dir_all(&value.library_root).unwrap();
        value
    }
    #[test]
    fn managed_markdown_deduplicates_preserves_source_and_relative_images() {
        let base = std::env::temp_dir().join(format!("ir-open-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(base.join("images")).unwrap();
        let source = base.join("notes.md");
        fs::write(&source, "# 合成笔记\n\n![图](images/one.png)\n正文。").unwrap();
        fs::write(base.join("images/one.png"), [137, 80, 78, 71]).unwrap();
        let locations = locations(&base);
        let progress = OpProgress::fixture();
        let first = import_one_file(&source, None, &locations, &progress).unwrap();
        let id = first["bookId"].as_str().unwrap();
        let (dir, manifest, mut saved) =
            crate::library::book_context(&locations.library_root, id).unwrap();
        assert_eq!(manifest.title, "合成笔记");
        assert!(dir.join("images/one.png").is_file());
        saved.position = 0.61;
        saved.updated = chrono::Utc::now().to_rfc3339();
        crate::progress::save_progress(&dir, &manifest, &saved).unwrap();
        let again = import_one_file(&source, None, &locations, &progress).unwrap();
        assert_eq!(first["bookId"], again["bookId"]);
        assert_eq!(again["reused"], true);
        assert_eq!(
            crate::library::book_context(&locations.library_root, id)
                .unwrap()
                .2
                .position,
            0.61
        );
        assert!(source.exists());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn temporary_file_progress_migrates_without_removing_the_original() {
        let base = std::env::temp_dir().join(format!("ir-legacy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let source = base.join("old.md");
        fs::write(&source, "# 旧文件\n正文").unwrap();
        let state_path = crate::state_path_for(&source.to_string_lossy());
        let raw =
            serde_json::json!({"scroll_position": 240.0, "bookmarks": [2,4], "progress": 0.62})
                .to_string();
        atomic_file::write(&state_path, raw.as_bytes()).unwrap();
        let locations = locations(&base);
        let result = import_one_file(&source, None, &locations, &OpProgress::fixture()).unwrap();
        let (dir, manifest, progress) = crate::library::book_context(
            &locations.library_root,
            result["bookId"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(progress.position, 0.62);
        assert!(state_path.exists());
        assert!(source.exists());
        let destination =
            crate::state_path_for(&dir.join(&manifest.chapters[0].path).to_string_lossy());
        assert_eq!(fs::read_to_string(&destination).unwrap(), raw);
        let _ = fs::remove_file(destination);
        let _ = fs::remove_file(state_path);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn multi_epub_is_not_sent_to_markdown_and_reports_partial_failures() {
        let base = std::env::temp_dir().join(format!("ir-open-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let epub = base.join("one.epub");
        let duplicate = base.join("two.epub");
        let bytes =
            include_bytes!("../../../../docs/audit/2026-10-18/fixtures/sample-book-epub.epub");
        fs::write(&epub, bytes).unwrap();
        fs::write(&duplicate, bytes).unwrap();
        let bad = base.join("broken.epub");
        fs::write(&bad, "bad zip").unwrap();
        let locations = locations(&base);
        let progress = OpProgress::fixture();
        let result = import_batch(&[epub, duplicate, bad], &locations, &progress).unwrap();
        assert_eq!(result["bookIds"].as_array().unwrap().len(), 2);
        assert_eq!(result["bookIds"][0], result["bookIds"][1]);
        assert_eq!(result["issues"].as_array().unwrap().len(), 1);
        assert_eq!(
            crate::library::scan_library(&locations.library_root)
                .unwrap()
                .books
                .len(),
            1
        );
        fs::remove_dir_all(base).unwrap();
    }
}
