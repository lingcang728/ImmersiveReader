use super::StorageLocations;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

fn canonicalize_for_comparison(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    let mut cursor = path;
    let mut missing = Vec::<OsString>::new();
    while let Some(parent) = cursor.parent() {
        if cursor.exists() {
            break;
        }
        if let Some(name) = cursor.file_name() {
            missing.push(name.to_os_string());
        }
        cursor = parent;
    }

    let mut resolved = fs::canonicalize(cursor).unwrap_or_else(|_| cursor.to_path_buf());
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    resolved
}

fn normalized(path: &Path) -> String {
    canonicalize_for_comparison(path)
        .to_string_lossy()
        .replace('/', r"\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn same_or_descendant(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

pub fn validate_library_root(
    candidate: &Path,
    locations: &StorageLocations,
) -> Result<PathBuf, String> {
    if !candidate.is_absolute() {
        return Err("Library root must be absolute".to_string());
    }
    if candidate.parent().is_none() {
        return Err("Library root cannot be a filesystem root".to_string());
    }

    let resolved = canonicalize_for_comparison(candidate);
    let candidate_key = normalized(&resolved);
    let reserved_names = [".incoming", ".transactions", ".revisions", ".trash"];
    if resolved.components().any(|component| {
        reserved_names
            .iter()
            .any(|name| component.as_os_str().eq_ignore_ascii_case(name))
    }) {
        return Err("Library root cannot be a Library control directory".to_string());
    }

    let managed_roots = [
        &locations.data_root,
        &locations.cache_root,
        &locations.logs_root,
        &locations.runtime_state_root,
        &locations.backups_root,
        &locations.runtime_root,
    ];
    for managed_root in managed_roots {
        let managed_key = normalized(managed_root);
        if same_or_descendant(&candidate_key, &managed_key)
            || same_or_descendant(&managed_key, &candidate_key)
        {
            return Err(format!(
                "Library root overlaps managed location: {}",
                managed_root.display()
            ));
        }
    }

    let temp_key = normalized(&std::env::temp_dir());
    if same_or_descendant(&candidate_key, &temp_key)
        || same_or_descendant(&temp_key, &candidate_key)
    {
        return Err("Library root cannot overlap the temporary directory".to_string());
    }

    Ok(resolved)
}
