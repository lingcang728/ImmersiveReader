use super::super::cache::release_podcast_cache_lease;
use super::super::contracts::{Chapter, Manifest};
use super::super::control::ControlDb;
use super::super::publish::{
    commit_transaction, load_transaction, PublishPhase, PublishTransaction,
};
use super::super::storage::StorageLocations;
use super::validate_task_id;
use crate::{atomic_file, publish};
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn required_string(value: &Value, path: &str) -> Result<String, String> {
    value
        .get(path)
        .and_then(Value::as_str)
        .filter(|item| !item.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("PUBLISH_FAILED: missing {path}"))
}

fn safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.chars().any(|item| item == '/' || item == '\\')
        && value
            .chars()
            .all(|item| item.is_ascii_alphanumeric() || matches!(item, '-' | '_' | ':'))
}

/// Folder name for a podcast book under `播客/` and Desktop `互动书架/播客/`.
/// Strips Windows-illegal characters and trims length while keeping CJK titles readable.
pub(crate) fn sanitize_podcast_folder_name(raw: &str) -> String {
    let mut name = raw
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>();
    while name.contains("  ") {
        name = name.replace("  ", " ");
    }
    let name = name.trim().trim_matches('.').to_string();
    let mut name = if name.is_empty() {
        "未命名播客".to_string()
    } else {
        name
    };
    // Keep path segments short enough for Windows MAX_PATH comfort.
    if name.chars().count() > 80 {
        name = name.chars().take(80).collect::<String>();
        name = name.trim().trim_matches('.').to_string();
    }
    if name.is_empty() {
        "未命名播客".to_string()
    } else {
        name
    }
}

fn podcast_library_relative_path(display_name: &str) -> String {
    format!("播客/{}", sanitize_podcast_folder_name(display_name))
}

/// Durable user-facing shelf: Desktop/互动书架/播客/{文件名}/ — markdown only.
fn desktop_podcast_export_dir(display_name: &str) -> Result<PathBuf, String> {
    let desktop = dirs::desktop_dir().ok_or_else(|| "DESKTOP_UNAVAILABLE".to_string())?;
    Ok(desktop
        .join("互动书架")
        .join("播客")
        .join(sanitize_podcast_folder_name(display_name)))
}

/// Copy only Markdown chapters into the desktop shelf folder (no manifest / provenance).
fn export_markdown_to_desktop_shelf(
    display_name: &str,
    chapter_files: &[(String, PathBuf)],
) -> Result<PathBuf, String> {
    let dest_dir = desktop_podcast_export_dir(display_name)?;
    fs::create_dir_all(&dest_dir).map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
    // Clear previous markdown in this folder so re-publish stays tidy.
    if dest_dir.is_dir() {
        for entry in fs::read_dir(&dest_dir).map_err(|error| format!("PUBLISH_FAILED: {error}"))? {
            let entry = entry.map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
            let path = entry.path();
            let is_md = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| matches!(value.to_ascii_lowercase().as_str(), "md" | "markdown"))
                .unwrap_or(false);
            if is_md {
                let _ = fs::remove_file(&path);
            }
        }
    }
    for (relative, source) in chapter_files {
        let file_name = Path::new(relative)
            .file_name()
            .map(|value| value.to_os_string())
            .unwrap_or_else(|| std::ffi::OsString::from("transcript.md"));
        let destination = dest_dir.join(file_name);
        fs::copy(source, &destination).map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
    }
    Ok(dest_dir)
}

fn collect_markdown(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|error| format!("PUBLISH_FAILED: {error}"))? {
        let entry = entry.map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
        if file_type.is_symlink() {
            return Err("PUBLISH_FAILED: output contains a symlink".to_string());
        }
        if file_type.is_dir() {
            collect_markdown(root, &entry.path(), files)?;
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
            .map_err(|error| format!("PUBLISH_FAILED: {error}"))?
            .to_string_lossy()
            .replace('\\', "/");
        if relative.is_empty()
            || relative
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err("PUBLISH_FAILED: output path is unsafe".to_string());
        }
        files.push((relative, entry.path()));
    }
    Ok(())
}

/// Prefer final polished output/, then fall back to worker internal markdown roots
/// (raw / bilingual) so a misplaced polish path still publishes usable transcript.
fn resolve_markdown_roots(task_cache_root: &Path) -> Vec<PathBuf> {
    vec![
        task_cache_root.join("output"),
        task_cache_root
            .join("work")
            .join("internal")
            .join("markdown_bilingual"),
        task_cache_root
            .join("work")
            .join("internal")
            .join("markdown_raw"),
    ]
}

fn copy_outputs(
    output_root: &Path,
    incoming: &Path,
    preferred_stem: &str,
) -> Result<(Vec<Chapter>, Vec<(String, PathBuf)>), String> {
    let task_cache_root = output_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| output_root.to_path_buf());
    let mut files = Vec::new();
    let mut source_root: Option<PathBuf> = None;
    for root in resolve_markdown_roots(&task_cache_root) {
        if !root.is_dir() {
            continue;
        }
        let mut candidate = Vec::new();
        collect_markdown(&root, &root, &mut candidate)?;
        if !candidate.is_empty() {
            files = candidate;
            source_root = Some(root);
            break;
        }
    }
    let Some(_source_root) = source_root else {
        if !output_root.is_dir() {
            return Err("PUBLISH_FAILED: worker output directory is missing".to_string());
        }
        return Err("PUBLISH_FAILED: worker produced no Markdown output".to_string());
    };
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err("PUBLISH_FAILED: worker produced no Markdown output".to_string());
    }
    let mut chapters = Vec::with_capacity(files.len());
    let mut exported_sources = Vec::with_capacity(files.len());
    let single = files.len() == 1;
    for (index, (relative, source)) in files.into_iter().enumerate() {
        // Flatten into the book folder: one file → "{stem}.md"; multi → keep unique names.
        let file_name = if single {
            format!("{}.md", sanitize_podcast_folder_name(preferred_stem))
        } else {
            Path::new(&relative)
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("chapter-{}.md", index + 1))
        };
        let destination = incoming.join(&file_name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
        }
        fs::copy(&source, &destination).map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
        exported_sources.push((file_name.clone(), destination.clone()));
        let title = Path::new(&file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Podcast")
            .to_string();
        let word_count = fs::read_to_string(&source)
            .map(|content| {
                content
                    .chars()
                    .filter(|value| !value.is_whitespace())
                    .count() as u64
            })
            .unwrap_or(0);
        let chapter_id = format!(
            "podcast:{}",
            Sha256::digest(file_name.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        chapters.push(Chapter {
            id: chapter_id,
            path: file_name.replace('\\', "/"),
            title,
            date: None,
            vote_count: 0,
            word_count,
        });
    }
    Ok((chapters, exported_sources))
}

fn write_book_metadata(
    incoming: &Path,
    task_id: &str,
    book_id: &str,
    source_id: &str,
    title: &str,
    revision: u64,
    engine_version: &str,
    chapters: Vec<Chapter>,
) -> Result<(String, String), String> {
    let now = Utc::now().to_rfc3339();
    let manifest = Manifest {
        schema_version: 1,
        book_id: book_id.to_string(),
        title: title.to_string(),
        source: "podcast".to_string(),
        source_id: Some(source_id.to_string()),
        generated_at: now.clone(),
        updated_at: now.clone(),
        chapters,
    };
    atomic_file::write(
        &incoming.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("PUBLISH_FAILED: {error}"))?,
    )?;
    let manifest_sha256 = publish::hash_file(&incoming.join("manifest.json"))?;
    let provenance = json!({
        "schemaVersion": 1,
        "bookId": book_id,
        "sourceId": source_id,
        "sourceKind": "podcast",
        "createdByTaskId": task_id,
        "lastSuccessfulTaskId": task_id,
        "revision": revision,
        "manifestSha256": manifest_sha256,
        "engineVersion": engine_version,
        "updatedAt": now,
    });
    atomic_file::write(
        &incoming.join("provenance.json"),
        &serde_json::to_vec_pretty(&provenance)
            .map_err(|error| format!("PUBLISH_FAILED: {error}"))?,
    )?;
    let provenance_sha256 = publish::hash_file(&incoming.join("provenance.json"))?;
    Ok((manifest_sha256, provenance_sha256))
}

pub fn publish_task_result_at(
    control: &mut ControlDb,
    locations: &StorageLocations,
    task_id: &str,
) -> Result<PublishTransaction, String> {
    validate_task_id(task_id)?;
    let snapshot = control
        .task_snapshot(task_id)?
        .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
    let task_root = locations
        .data_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    let spec: Value = serde_json::from_slice(
        &fs::read(task_root.join("task.json"))
            .map_err(|error| format!("PUBLISH_FAILED: {error}"))?,
    )
    .map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
    let publish_spec = spec
        .get("publish")
        .ok_or_else(|| "PUBLISH_FAILED: publish metadata is missing".to_string())?;
    let book_id = required_string(publish_spec, "bookId")?;
    let source_id = required_string(publish_spec, "sourceId")?;
    let incoming_relative_path = required_string(publish_spec, "incomingRelativePath")?;
    let revision = publish_spec
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "PUBLISH_FAILED: publish revision is missing".to_string())?;
    if !safe_segment(&source_id) || !incoming_relative_path.starts_with(".incoming/") {
        return Err("PUBLISH_FAILED: publish path is unsafe".to_string());
    }
    let incoming = locations.library_root.join(&incoming_relative_path);
    let input_name = spec
        .get("input")
        .and_then(|value| value.get("relativePath"))
        .and_then(Value::as_str)
        .and_then(|value| Path::new(value).file_stem())
        .and_then(|value| value.to_str())
        .unwrap_or("Podcast")
        .to_string();
    // Human-readable shelf path: 播客/{文件名}/  (not Cache, not hash folders).
    let final_relative_path = podcast_library_relative_path(&input_name);
    let rollback_relative_path = format!(".revisions/{source_id}/{revision}");
    let existing = load_transaction(&locations.library_root, task_id).ok();
    if existing
        .as_ref()
        .is_some_and(|item| item.phase == PublishPhase::Committed)
    {
        release_podcast_cache_lease(locations, task_id, snapshot.cache_lease_bytes)?;
        return Ok(existing.expect("committed transaction must exist"));
    }
    // Clear incomplete journals so a re-publish (Retry) always starts from a clean Prepared state.
    // Leaving RolledBack/Prepared journals makes commit_transaction no-op and looks like a hang/crash.
    if existing.is_some() {
        let journal = locations
            .library_root
            .join(".transactions")
            .join(format!("{task_id}.json"));
        let _ = fs::remove_file(journal);
    }
    if incoming.exists() {
        fs::remove_dir_all(&incoming).map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
    }
    fs::create_dir_all(&incoming).map_err(|error| format!("PUBLISH_FAILED: {error}"))?;
    let output_root = locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id)
        .join("output");
    let engine_version = spec
        .get("compatibility")
        .and_then(|value| value.get("engineVersion"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let (chapters, markdown_sources) = match copy_outputs(&output_root, &incoming, &input_name) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&incoming);
            return Err(error);
        }
    };
    let (manifest_sha256, provenance_sha256) = match write_book_metadata(
        &incoming,
        task_id,
        &book_id,
        &source_id,
        &input_name,
        revision,
        engine_version,
        chapters,
    ) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&incoming);
            return Err(error);
        }
    };
    let transaction = PublishTransaction {
        schema_version: 1,
        transaction_id: task_id.to_string(),
        task_id: task_id.to_string(),
        book_id: book_id.clone(),
        incoming_relative_path,
        final_relative_path,
        rollback_relative_path,
        manifest_sha256,
        provenance_sha256,
        revision,
        phase: PublishPhase::Prepared,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    control.record_publish_transaction(
        &transaction.transaction_id,
        task_id,
        &book_id,
        "prepared",
        &format!(".transactions/{}.json", task_id),
    )?;
    let committed = commit_transaction(&locations.library_root, &transaction)?;
    if !matches!(committed.phase, PublishPhase::Committed) {
        // commit_transaction returns Ok(RolledBack) when validation fails mid-flight.
        // Surface that as a hard error so Retry does not mark the task successful.
        return Err(format!(
            "PUBLISH_FAILED: transaction ended in {:?}",
            committed.phase
        ));
    }
    control.record_publish_transaction(
        &committed.transaction_id,
        task_id,
        &book_id,
        "committed",
        &format!(".transactions/{}.json", task_id),
    )?;
    // Durable markdown-only copy on the user's Desktop shelf (survives cache cleanup).
    // Skip QA / test channels so unit tests do not touch the real Desktop.
    if locations.channel == "production" {
        if let Err(error) = export_markdown_to_desktop_shelf(&input_name, &markdown_sources) {
            eprintln!("Desktop podcast export warning: {error}");
        }
    }
    release_podcast_cache_lease(locations, task_id, snapshot.cache_lease_bytes)?;
    Ok(committed)
}

#[cfg(test)]
mod tests {
    use super::{publish_task_result_at, sanitize_podcast_folder_name};

    #[test]
    fn sanitizes_podcast_folder_names_for_windows() {
        assert_eq!(
            sanitize_podcast_folder_name(r#"[商业就是这样] 349.Vol.264 把世界杯作为方法"#),
            "[商业就是这样] 349.Vol.264 把世界杯作为方法"
        );
        assert_eq!(
            sanitize_podcast_folder_name(r#"a/b:c*d?"#),
            "a b c d"
        );
        assert_eq!(sanitize_podcast_folder_name("..."), "未命名播客");
    }
    use crate::cache::{acquire_podcast_cache_lease, read_podcast_recovery};
    use crate::control::ControlDb;
    use crate::storage::StorageLocations;
    use crate::tasks::{
        LifecycleState, ProgressMode, RequiredAction, TaskEvent, TaskKind, TaskOutcome,
        TaskProgress, TaskSnapshot,
    };
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn publishes_worker_markdown_and_releases_lease() {
        let root =
            std::env::temp_dir().join(format!("immersive-podcast-publish-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let locations = StorageLocations {
            channel: "test".to_string(),
            settings_path: root.join(r"Settings\settings.json"),
            data_root: root.join("Data"),
            cache_root: root.join("Cache"),
            logs_root: root.join("Logs"),
            runtime_state_root: root.join("RuntimeState"),
            backups_root: root.join("Backups"),
            library_root: root.join("Library"),
            runtime_root: PathBuf::from("runtime"),
        };
        let task_id = "task-1";
        let source_id = "a".repeat(64);
        let task_root = locations.data_root.join(r"Podcast\Tasks").join(task_id);
        fs::create_dir_all(&task_root).expect("task root must exist");
        // Prefer final output/, but also verify internal fallback works when output/ is empty.
        let internal = locations
            .cache_root
            .join(r"Podcast\Tasks")
            .join(task_id)
            .join(r"work\internal\markdown_raw");
        fs::create_dir_all(&internal).expect("internal root must exist");
        fs::write(internal.join("result.md"), "# Result\n\nPublished")
            .expect("internal markdown must write");
        fs::write(
            task_root.join("task.json"),
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": 1,
                "taskId": task_id,
                "input": {"relativePath": "input/source.mp3", "inputSha256": source_id, "bytes": 12},
                "compatibility": {"engineVersion": "engine-hash"},
                "publish": {"bookId": format!("podcast:{source_id}"), "sourceId": source_id, "revision": 1, "incomingRelativePath": ".incoming/task-1"}
            }))
            .expect("task spec must serialize"),
        )
        .expect("task spec must write");
        acquire_podcast_cache_lease(&locations, task_id, "queued", 12).expect("lease must persist");
        let now = "2026-07-12T00:00:00Z".to_string();
        let snapshot = TaskSnapshot {
            id: task_id.to_string(),
            kind: TaskKind::Podcast,
            revision: 1,
            last_sequence: 1,
            lifecycle_state: LifecycleState::Queued,
            outcome: TaskOutcome::None,
            required_action: RequiredAction::None,
            progress: TaskProgress {
                mode: ProgressMode::Indeterminate,
                percent: None,
                completed_units: None,
                total_units: None,
                label: None,
                unit: None,
                source_total_units: None,
                skipped_units: None,
            },
            error_code: None,
            error_message: None,
            retry_after_seconds: None,
            engine_stage: "queued".to_string(),
            engine_status: "waiting".to_string(),
            recoverable: true,
            can_pause: false,
            can_resume: false,
            can_retry: false,
            can_cancel: true,
            book_id: Some(format!("podcast:{source_id}")),
            source_id: Some(source_id.clone()),
            cache_lease_bytes: 12,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_heartbeat_at: None,
            checkpoint_at: None,
        };
        let event = TaskEvent {
            schema_version: 1,
            task_id: task_id.to_string(),
            sequence: 1,
            revision: 1,
            event_type: "queued".to_string(),
            snapshot,
            created_at: now,
        };
        let mut control = ControlDb::open(&locations.data_root.join(r"App\control.db"))
            .expect("control database must open");
        control
            .persist_task_event(&event)
            .expect("queued event must persist");
        let transaction = publish_task_result_at(&mut control, &locations, task_id)
            .expect("publish must succeed");
        assert_eq!(transaction.phase, crate::publish::PublishPhase::Committed);
        let repeated = publish_task_result_at(&mut control, &locations, task_id)
            .expect("repeated publish must be idempotent");
        assert_eq!(repeated.phase, crate::publish::PublishPhase::Committed);
        assert!(locations
            .library_root
            .join("播客")
            .join("source")
            .join("manifest.json")
            .is_file());
        assert!(locations
            .library_root
            .join("播客")
            .join("source")
            .join("source.md")
            .is_file());
        assert_eq!(
            read_podcast_recovery(&locations, task_id)
                .expect("recovery must load")
                .expect("recovery must exist")
                .lease_held,
            false
        );
        drop(control);
        fs::remove_dir_all(root).expect("fixture must remove");
    }
}
