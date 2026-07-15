use crate::contracts::Manifest;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

mod commands;
pub use commands::{delete_idempotent, restore_idempotent};

const ENTRY_FILE: &str = "trash-entry.json";
const JOURNAL_DIR: &str = ".journal";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    pub schema_version: u32,
    pub trash_id: String,
    pub book_id: String,
    pub title: String,
    pub original_relative_path: String,
    pub trash_relative_path: String,
    pub deleted_at: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashRestoreResult {
    pub book_id: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashDeleteResult {
    pub deleted_items: u64,
    pub released_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrashJournal {
    schema_version: u32,
    operation: String,
    trash_id: String,
    phase: String,
    item: TrashItem,
}

fn validate_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("INVALID_ARGUMENT".to_string());
    }
    Ok(())
}

fn parse_relative(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) if !part.to_string_lossy().starts_with('.') => safe.push(part),
            Component::Normal(_) | Component::CurDir => {
                return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string())
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string())
            }
        }
    }
    if safe.as_os_str().is_empty() {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    Ok(safe)
}

fn normalized_relative(root: &Path, target: &Path) -> Result<String, String> {
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let canonical_target = target.canonicalize().map_err(|error| error.to_string())?;
    let relative = canonical_target
        .strip_prefix(&canonical_root)
        .map_err(|_| "PATH_OUTSIDE_MANAGED_ROOT".to_string())?;
    let safe = parse_relative(&relative.to_string_lossy().replace('\\', "/"))?;
    Ok(safe.to_string_lossy().replace('\\', "/"))
}

fn item_root(root: &Path, trash_id: &str) -> Result<PathBuf, String> {
    validate_id(trash_id)?;
    Ok(root.join(".trash").join(trash_id))
}

fn journal_root(root: &Path) -> Result<PathBuf, String> {
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let journal = canonical_root.join(".trash").join(JOURNAL_DIR);
    fs::create_dir_all(&journal).map_err(|error| error.to_string())?;
    let canonical_journal = journal.canonicalize().map_err(|error| error.to_string())?;
    if canonical_journal != canonical_root.join(".trash").join(JOURNAL_DIR) {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    Ok(canonical_journal)
}

fn journal_path(root: &Path, trash_id: &str) -> Result<PathBuf, String> {
    validate_id(trash_id)?;
    Ok(journal_root(root)?.join(format!("{trash_id}.json")))
}

fn write_journal(root: &Path, journal: &TrashJournal) -> Result<(), String> {
    let path = journal_path(root, &journal.trash_id)?;
    let data = serde_json::to_vec_pretty(journal).map_err(|error| error.to_string())?;
    crate::atomic_file::write(&path, &data)
}

fn remove_journal(root: &Path, trash_id: &str) {
    if let Ok(path) = journal_path(root, trash_id) {
        let _ = fs::remove_file(path);
    }
}

fn write_entry(destination: &Path, item: &TrashItem) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(item).map_err(|error| error.to_string())?;
    crate::atomic_file::write(&destination.join(ENTRY_FILE), &data)
}

pub fn reconcile(root: &Path) -> Result<(), String> {
    let trash_root = root.join(".trash");
    if !trash_root.exists() {
        return Ok(());
    }
    let journal_root = journal_root(root)?;
    for entry in fs::read_dir(&journal_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry.file_type().map_err(|error| error.to_string())?.is_file() {
            continue;
        }
        let journal: TrashJournal = match fs::read_to_string(entry.path())
            .ok()
            .and_then(|raw| serde_json::from_str::<TrashJournal>(&raw).ok())
        {
            Some(value) if value.schema_version == 1 => value,
            _ => continue,
        };
        let item_root = trash_root.join(&journal.trash_id);
        match journal.operation.as_str() {
            "move" => {
                if item_root.is_dir() {
                    write_entry(&item_root, &journal.item)?;
                    remove_journal(root, &journal.trash_id);
                } else if root.join(parse_relative(&journal.item.original_relative_path)?).exists() {
                    remove_journal(root, &journal.trash_id);
                }
            }
            "restore" => {
                let destination = root.join(parse_relative(&journal.item.original_relative_path)?);
                if destination.exists() {
                    remove_journal(root, &journal.trash_id);
                } else if item_root.is_dir() {
                    let entry_path = item_root.join(ENTRY_FILE);
                    if !entry_path.exists() {
                        write_entry(&item_root, &journal.item)?;
                    }
                    remove_journal(root, &journal.trash_id);
                }
            }
            "permanent_delete" => {
                if !item_root.exists() {
                    remove_journal(root, &journal.trash_id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn managed_trash_root(root: &Path) -> Result<PathBuf, String> {
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let trash_root = root.join(".trash");
    let canonical_trash = trash_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if canonical_trash != canonical_root.join(".trash") {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    Ok(canonical_trash)
}

fn load(root: &Path, trash_id: &str) -> Result<(PathBuf, TrashItem), String> {
    let item_root = item_root(root, trash_id)?;
    let canonical_item = item_root
        .canonicalize()
        .map_err(|_| "NOT_FOUND".to_string())?;
    if canonical_item != managed_trash_root(root)?.join(trash_id) {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    let raw =
        fs::read_to_string(canonical_item.join(ENTRY_FILE)).map_err(|_| "NOT_FOUND".to_string())?;
    let item: TrashItem =
        serde_json::from_str(&raw).map_err(|_| "INVALID_TRASH_ENTRY".to_string())?;
    if item.schema_version != 1
        || item.trash_id != trash_id
        || item.trash_relative_path != format!(".trash/{trash_id}")
    {
        return Err("INVALID_TRASH_ENTRY".to_string());
    }
    parse_relative(&item.original_relative_path)?;
    Ok((canonical_item, item))
}

pub fn move_book(root: &Path, book_root: &Path, manifest: &Manifest) -> Result<TrashItem, String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    reconcile(root)?;
    let original_relative_path = normalized_relative(root, book_root)?;
    let trash_id = Uuid::new_v4().simple().to_string();
    let trash_root = root.join(".trash");
    fs::create_dir_all(&trash_root).map_err(|error| error.to_string())?;
    let trash_root = managed_trash_root(root)?;
    let destination = trash_root.join(&trash_id);
    let item = TrashItem {
        schema_version: 1,
        trash_id: trash_id.clone(),
        book_id: manifest.book_id.clone(),
        title: manifest.title.clone(),
        original_relative_path,
        trash_relative_path: format!(".trash/{trash_id}"),
        deleted_at: chrono::Utc::now().to_rfc3339(),
        revision: 1,
    };
    write_journal(
        root,
        &TrashJournal {
            schema_version: 1,
            operation: "move".to_string(),
            trash_id: trash_id.clone(),
            phase: "prepared".to_string(),
            item: item.clone(),
        },
    )?;
    fs::rename(book_root, &destination).map_err(|error| error.to_string())?;
    let _ = write_journal(
        root,
        &TrashJournal {
            schema_version: 1,
            operation: "move".to_string(),
            trash_id: trash_id.clone(),
            phase: "renamed".to_string(),
            item: item.clone(),
        },
    );
    write_entry(&destination, &item)?;
    remove_journal(root, &trash_id);
    Ok(item)
}

pub fn list(root: &Path) -> Result<Vec<TrashItem>, String> {
    let trash_root = root.join(".trash");
    if !trash_root.exists() {
        return Ok(Vec::new());
    }
    reconcile(root)?;
    let trash_root = managed_trash_root(root)?;
    let mut items = Vec::new();
    for entry in fs::read_dir(trash_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_symlink()
        {
            continue;
        }
        let trash_id = entry.file_name().to_string_lossy().into_owned();
        if let Ok((_, item)) = load(root, &trash_id) {
            items.push(item);
        }
    }
    items.sort_by(|left, right| right.deleted_at.cmp(&left.deleted_at));
    Ok(items)
}

pub fn restore(
    root: &Path,
    trash_id: &str,
    expected_revision: u64,
) -> Result<TrashRestoreResult, String> {
    reconcile(root)?;
    let (source, item) = load(root, trash_id)?;
    if item.revision != expected_revision {
        return Err("REVISION_CONFLICT".to_string());
    }
    let destination = root.join(parse_relative(&item.original_relative_path)?);
    if destination.exists() {
        return Err("CONFLICT".to_string());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "PATH_OUTSIDE_MANAGED_ROOT".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let canonical_parent = parent.canonicalize().map_err(|error| error.to_string())?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err("PATH_OUTSIDE_MANAGED_ROOT".to_string());
    }
    write_journal(
        root,
        &TrashJournal {
            schema_version: 1,
            operation: "restore".to_string(),
            trash_id: trash_id.to_string(),
            phase: "prepared".to_string(),
            item: item.clone(),
        },
    )?;
    let metadata_path = source.join(ENTRY_FILE);
    let metadata = fs::read(&metadata_path).map_err(|error| error.to_string())?;
    fs::remove_file(&metadata_path).map_err(|error| error.to_string())?;
    let _ = write_journal(
        root,
        &TrashJournal {
            schema_version: 1,
            operation: "restore".to_string(),
            trash_id: trash_id.to_string(),
            phase: "metadata_removed".to_string(),
            item: item.clone(),
        },
    );
    if let Err(error) = fs::rename(&source, &destination) {
        crate::atomic_file::write(&metadata_path, &metadata)?;
        remove_journal(root, trash_id);
        return Err(error.to_string());
    }
    remove_journal(root, trash_id);
    Ok(TrashRestoreResult {
        book_id: item.book_id,
        relative_path: item.original_relative_path,
    })
}

fn measure(path: &Path) -> Result<(u64, u64), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return Ok((1, metadata.len()));
    }
    let mut totals = (1_u64, 0_u64);
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let measured = measure(&entry.path())?;
        totals.0 = totals.0.saturating_add(measured.0);
        totals.1 = totals.1.saturating_add(measured.1);
    }
    Ok(totals)
}

pub fn permanently_delete(
    root: &Path,
    trash_id: &str,
    expected_revision: u64,
) -> Result<TrashDeleteResult, String> {
    reconcile(root)?;
    let (source, item) = load(root, trash_id)?;
    if item.revision != expected_revision {
        return Err("REVISION_CONFLICT".to_string());
    }
    write_journal(
        root,
        &TrashJournal {
            schema_version: 1,
            operation: "permanent_delete".to_string(),
            trash_id: trash_id.to_string(),
            phase: "prepared".to_string(),
            item,
        },
    )?;
    let (deleted_items, released_bytes) = measure(&source)?;
    fs::remove_dir_all(source).map_err(|error| error.to_string())?;
    remove_journal(root, trash_id);
    Ok(TrashDeleteResult {
        deleted_items,
        released_bytes,
    })
}

#[cfg(test)]
mod tests;
