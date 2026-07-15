use crate::contracts::{Chapter, Manifest};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn collect_markdown(
    root: &Path,
    dir: &Path,
    output: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_markdown(root, &entry.path(), output)?;
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
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        output.push((relative, entry.path()));
    }
    Ok(())
}

fn stable_chapter_id(relative: &str) -> String {
    let normalized = relative.to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("manual:{digest:x}")
}

fn unique_target(manual_root: &Path, title: &str) -> PathBuf {
    let base = if title.trim().is_empty() {
        "未命名书目"
    } else {
        title
    };
    let direct = manual_root.join(base);
    if !direct.exists() {
        return direct;
    }
    for suffix in 2..10_000 {
        let candidate = manual_root.join(format!("{base} ({suffix})"));
        if !candidate.exists() {
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

pub fn import_markdown_folder(source: &Path, library_root: &Path) -> Result<Manifest, String> {
    if !source.is_dir() {
        return Err("Import source must be a folder".to_string());
    }
    let mut files = Vec::new();
    collect_markdown(source, source, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err("The selected folder contains no Markdown files".to_string());
    }

    let title = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("未命名书目")
        .to_string();
    let manual_root = library_root.join("手动");
    fs::create_dir_all(&manual_root).map_err(|error| error.to_string())?;
    let target = unique_target(&manual_root, &title);
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;

    let result = (|| {
        let mut chapters = Vec::with_capacity(files.len());
        for (relative, path) in files {
            let destination = target.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::copy(&path, &destination).map_err(|error| error.to_string())?;
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
        crate::atomic_write_file(&target.join("manifest.json"), &data)?;
        Ok(manifest)
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
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
        let manifest = import_markdown_folder(&source, &library).expect("import must succeed");
        assert_eq!(manifest.chapters.len(), 2);
        assert!(source.join("02.md").exists());
        assert!(library.join("手动/source-book/manifest.json").exists());
        fs::remove_dir_all(root).expect("temp directory must be removed");
    }
}
