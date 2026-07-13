use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationIssue {
    pub kind: String,
    pub source_item_id: Option<String>,
    pub source_url: Option<String>,
    pub candidate_paths: Vec<String>,
    pub sha256: Option<String>,
    pub suggestion: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationReport {
    pub generated_at: String,
    pub database_path: String,
    pub output_root: String,
    pub database_success_rows: u64,
    pub markdown_files: u64,
    pub unresolved_count: u64,
    pub category_counts: BTreeMap<String, u64>,
    pub issues: Vec<ReconciliationIssue>,
}

#[derive(Clone, Debug, Deserialize)]
struct SuccessRow {
    item_id: String,
    url: String,
    output_path: Option<String>,
}

fn success_rows(executable: &Path, database: &Path) -> Result<Vec<SuccessRow>, String> {
    let archive_check = Command::new(executable)
        .arg("-batch")
        .arg("-noheader")
        .arg(database)
        .arg("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='archive_revisions';")
        .output()
        .map_err(|error| format!("SQLite executable failed to start: {error}"))?;
    if !archive_check.status.success() {
        return Err(format!(
            "SQLite catalog check failed: {}",
            String::from_utf8_lossy(&archive_check.stderr).trim()
        ));
    }
    let has_archive = String::from_utf8_lossy(&archive_check.stdout).trim() == "1";
    let sql = if has_archive {
        "SELECT ai.item_id, ai.source_url AS url, ar.output_path FROM archive_items ai JOIN archive_revisions ar ON ar.item_id = ai.item_id AND ar.revision = ai.current_revision ORDER BY ai.item_id;"
    } else {
        "SELECT ti.item_id, i.url, ti.output_path FROM task_items ti JOIN items i ON i.id = ti.item_id WHERE ti.status = 'success' ORDER BY ti.item_id, ti.updated_at;"
    };
    let result = Command::new(executable)
        .arg("-batch")
        .arg("-json")
        .arg(database)
        .arg(sql)
        .output()
        .map_err(|error| format!("SQLite executable failed to start: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "SQLite reconciliation query failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    let raw = String::from_utf8(result.stdout).map_err(|error| error.to_string())?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

fn markdown_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        if root
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("md"))
            && !root
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("index.md"))
        {
            files.push(root.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        markdown_files(&entry.map_err(|error| error.to_string())?.path(), files)?;
    }
    Ok(())
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('/', r"\").to_lowercase()
}

fn resolve_output(root: &Path, stored: &str) -> Result<PathBuf, String> {
    let path = Path::new(stored);
    if path.is_absolute() {
        let root_key = normalized(root);
        let path_key = normalized(path);
        if path_key == root_key
            || path_key
                .strip_prefix(&root_key)
                .is_some_and(|suffix| suffix.starts_with('\\'))
        {
            return Ok(path.to_path_buf());
        }
        return Err("absolute output path is outside the archive root".to_string());
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("relative output path escapes the archive root".to_string());
    }
    Ok(root.join(path))
}

fn issue(
    kind: &str,
    row: Option<&SuccessRow>,
    paths: Vec<String>,
    suggestion: &str,
) -> ReconciliationIssue {
    let sha256 = paths
        .first()
        .and_then(|value| crate::publish::hash_file(Path::new(value)).ok());
    ReconciliationIssue {
        kind: kind.to_string(),
        source_item_id: row.map(|value| value.item_id.clone()),
        source_url: row.map(|value| value.url.clone()),
        candidate_paths: paths,
        sha256,
        suggestion: suggestion.to_string(),
    }
}

pub fn reconcile_zhihu_archive(
    executable: &Path,
    database: &Path,
    output_root: &Path,
) -> Result<ReconciliationReport, String> {
    let rows = success_rows(executable, database)?;
    let mut files = Vec::new();
    markdown_files(output_root, &mut files)?;
    files.sort();
    let mut issues = Vec::new();
    let mut referenced = BTreeSet::new();
    let mut rows_by_item = BTreeMap::<String, Vec<&SuccessRow>>::new();
    let mut rows_by_path = BTreeMap::<String, Vec<&SuccessRow>>::new();
    for row in &rows {
        rows_by_item
            .entry(row.item_id.clone())
            .or_default()
            .push(row);
        let Some(stored) = row.output_path.as_deref() else {
            issues.push(issue(
                "db-only",
                Some(row),
                Vec::new(),
                "keep the database record unresolved until a file is selected",
            ));
            continue;
        };
        match resolve_output(output_root, stored) {
            Ok(path) => {
                let key = normalized(&path);
                referenced.insert(key.clone());
                rows_by_path.entry(key).or_default().push(row);
                if !path.is_file() {
                    issues.push(issue(
                        "missing-file",
                        Some(row),
                        vec![path.to_string_lossy().into_owned()],
                        "retain the last database record and locate or regenerate the file",
                    ));
                }
            }
            Err(error) => issues.push(issue(
                "path-conflict",
                Some(row),
                vec![stored.to_string()],
                &error,
            )),
        }
    }
    for candidate_rows in rows_by_item.values().filter(|values| values.len() > 1) {
        issues.push(issue(
            "multiple-candidates-for-source-item",
            candidate_rows.first().copied(),
            candidate_rows
                .iter()
                .filter_map(|row| row.output_path.clone())
                .collect(),
            "require an explicit candidate choice",
        ));
    }
    for candidate_rows in rows_by_path.values().filter(|values| values.len() > 1) {
        issues.push(issue(
            "duplicate-success-path",
            candidate_rows.first().copied(),
            candidate_rows
                .iter()
                .filter_map(|row| row.output_path.clone())
                .collect(),
            "do not merge or delete duplicate records automatically",
        ));
    }
    let mut author_directories = BTreeSet::new();
    for file in &files {
        let key = normalized(file);
        if !referenced.contains(&key) {
            issues.push(issue(
                "file-only",
                None,
                vec![file.to_string_lossy().into_owned()],
                "retain the file and request source matching",
            ));
        }
        if fs::read_to_string(file).is_err() {
            issues.push(issue(
                "unparseable-markdown",
                None,
                vec![file.to_string_lossy().into_owned()],
                "retain bytes and request encoding or content repair",
            ));
        }
        if let Some(parent) = file.parent() {
            author_directories.insert(parent.to_path_buf());
        }
    }
    for directory in author_directories {
        if !directory.join("manifest.json").is_file() {
            issues.push(issue(
                "manifest-missing",
                None,
                vec![directory.to_string_lossy().into_owned()],
                "generate metadata only after source conflicts are resolved",
            ));
        }
        if !directory.join("provenance.json").is_file() {
            issues.push(issue(
                "provenance-missing",
                None,
                vec![directory.to_string_lossy().into_owned()],
                "preserve legacy content until provenance is confirmed",
            ));
        }
    }
    let mut category_counts = BTreeMap::new();
    for value in &issues {
        *category_counts.entry(value.kind.clone()).or_insert(0) += 1;
    }
    Ok(ReconciliationReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        database_path: database.to_string_lossy().into_owned(),
        output_root: output_root.to_string_lossy().into_owned(),
        database_success_rows: rows.len() as u64,
        markdown_files: files.len() as u64,
        unresolved_count: issues.len() as u64,
        category_counts,
        issues,
    })
}
