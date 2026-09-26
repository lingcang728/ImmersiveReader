use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// SAF content-URI staging lives in the Android app-private cache under
/// `<prefix><op-id>` — the prefix set `sweep_app_cache_staging` and
/// `items` share so residue is both listed and cleaned on startup.
pub(crate) const ANDROID_STAGING_PREFIXES: [&str; 2] = ["import-", "mobile-import-"];
/// Fixed-name staging dir holding per-pick `.md*` copies from SAF.
pub(crate) const ANDROID_TEMP_PICK_DIR: &str = "临时";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryItem {
    pub source: String,
    pub title: String,
    pub path: String,
    pub modified_at: Option<String>,
}

/// Enumerate "temporary" Markdown for the shelf's temporary row:
/// the two Podcast output dirs plus — on Android — the app-cache SAF
/// staging dirs (`临时`, `import-*`, `mobile-import-*`) so a pending
/// mobile pick is visible instead of invisible. `app_cache` is `None`
/// everywhere except Android.
pub fn items(
    locations: &crate::storage::StorageLocations,
    app_cache: Option<&Path>,
) -> Result<Vec<TemporaryItem>, String> {
    let mut items = Vec::new();
    let roots = [
        locations.data_root.join("Podcast").join("LegacyOutput"),
        locations.cache_root.join("Podcast").join("output"),
    ];
    for path in roots.into_iter().filter(|path| path.is_dir()) {
        append_markdown_items(&mut items, &path, "podcast")?;
    }
    if let Some(app_cache) = app_cache {
        for dir in android_staging_dirs(app_cache) {
            append_staging_items(&mut items, &dir)?;
        }
    }
    items.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
    Ok(items)
}

/// The staging dirs under the Android app cache that may hold picked
/// content right now: the fixed 临时 dir plus every `import-*` /
/// `mobile-import-*` operation dir. `pub(crate)` so `cache::safe_cleanup`
/// can measure + remove the same set under the storage-usage umbrella.
pub(crate) fn android_staging_dirs(app_cache: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let fixed = app_cache.join(ANDROID_TEMP_PICK_DIR);
    if fixed.is_dir() {
        dirs.push(fixed);
    }
    if let Ok(entries) = fs::read_dir(app_cache) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if ANDROID_STAGING_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
                && entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false)
            {
                dirs.push(entry.path());
            }
        }
    }
    dirs
}

/// Android app-cache staging is per-operation and per-process: a picked
/// SAF file staged under `import-*`/`mobile-import-*`/`临时` cannot outlive
/// the operation that created it (the URI grant does not survive process
/// death either), so startup sweeps them all. Returns the removed dir
/// count for the log line.
#[cfg(target_os = "android")]
pub(crate) fn sweep_app_cache_staging(app_cache: &Path) -> usize {
    let mut removed = 0_usize;
    for dir in android_staging_dirs(app_cache) {
        if fs::remove_dir_all(&dir).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Non-Android builds never stage SAF picks — the sweep exists so the
/// shared cleanup code compiles without `#[cfg]` at the call site.
#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
pub(crate) fn sweep_app_cache_staging(_app_cache: &Path) -> usize {
    0
}

fn append_markdown_items(
    items: &mut Vec<TemporaryItem>,
    path: &Path,
    source: &str,
) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
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
        items.push(TemporaryItem {
            source: source.to_string(),
            title: entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("临时内容")
                .to_string(),
            path: entry.path().to_string_lossy().into_owned(),
            modified_at: modified_at(&entry),
        });
    }
    Ok(())
}

/// Staging dirs carry not just `.md` — a picked EPUB or archive is
/// temporary content too, so every regular file is listed (recursively one
/// level at a time; staging trees stay shallow).
fn append_staging_items(items: &mut Vec<TemporaryItem>, path: &Path) -> Result<(), String> {
    let mut stack = vec![path.to_path_buf()];
    let mut depth_budget = 8_usize;
    while let Some(dir) = stack.pop() {
        if depth_budget == 0 {
            break;
        }
        depth_budget -= 1;
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            items.push(TemporaryItem {
                source: "mobile-staging".to_string(),
                title: entry
                    .path()
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("临时内容")
                    .to_string(),
                path: entry.path().to_string_lossy().into_owned(),
                modified_at: modified_at(&entry),
            });
        }
    }
    Ok(())
}

fn modified_at(entry: &fs::DirEntry) -> Option<String> {
    entry
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(|time| DateTime::<Utc>::from(time).to_rfc3339())
}
