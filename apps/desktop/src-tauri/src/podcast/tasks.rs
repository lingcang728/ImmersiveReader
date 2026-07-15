use super::task_contract::{write_task_contract, TaskContractRequest};
use super::task_request::{replay, request_hash, validate_budget, StoredPreview};
use super::{
    copy_verified_input_with_progress, AddPodcastFilesRequest, DuplicatePolicy, PodcastAddResult,
    PodcastBudgetApproval, PodcastPreviewStore,
};
use crate::cache::set_podcast_recovery_compatibility;
use crate::control::{CommandClaim, ControlDb};
use crate::storage::StorageLocations;
use crate::tasks::{
    LifecycleState, ProgressMode, RequiredAction, TaskEvent, TaskKind, TaskOutcome, TaskProgress,
    TaskSnapshot,
};
use std::path::Path;
use uuid::Uuid;

const COMMAND_NAME: &str = "add_podcast_files";
pub const TASK_EVENT_NAME: &str = "acquisition://task-event";

struct FileSnapshotParams<'a> {
    task_id: String,
    file: &'a super::PodcastFilePreview,
    cache_bytes: u64,
    revision: u64,
    last_sequence: u64,
    stage: &'a str,
    status: &'a str,
    label: &'a str,
    percent: Option<f64>,
    completed_units: Option<u64>,
    total_units: Option<u64>,
    created_at: Option<String>,
}

fn snapshot_for_file(params: FileSnapshotParams<'_>) -> TaskSnapshot {
    let FileSnapshotParams {
        task_id,
        file,
        cache_bytes,
        revision,
        last_sequence,
        stage,
        status,
        label,
        percent,
        completed_units,
        total_units,
        created_at,
    } = params;
    let now = chrono::Utc::now().to_rfc3339();
    TaskSnapshot {
        id: task_id,
        kind: TaskKind::Podcast,
        revision,
        last_sequence,
        lifecycle_state: LifecycleState::Queued,
        outcome: TaskOutcome::None,
        required_action: RequiredAction::None,
        progress: TaskProgress {
            mode: if percent.is_some() || total_units.is_some() {
                ProgressMode::Determinate
            } else {
                ProgressMode::Indeterminate
            },
            // input_copy is only the first ~8% of the whole pipeline.
            percent: percent.map(|value| {
                if stage == "input_copy" {
                    (value.clamp(0.0, 100.0) / 100.0) * 8.0
                } else {
                    value
                }
            }),
            completed_units,
            total_units,
            label: Some(label.to_string()),
            unit: Some("字节".to_string()),
            source_total_units: Some(file.bytes),
            skipped_units: None,
        },
        error_code: None,
        error_message: None,
        retry_after_seconds: None,
        engine_stage: stage.to_string(),
        engine_status: status.to_string(),
        recoverable: true,
        can_pause: false,
        can_resume: false,
        can_retry: false,
        can_cancel: true,
        book_id: Some(file.book_id.clone()),
        source_id: Some(file.source_id.clone()),
        display_name: Some(
            std::path::Path::new(&file.file_name)
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .unwrap_or(&file.file_name)
                .to_string(),
        ),
        cache_lease_bytes: cache_bytes,
        created_at: created_at.unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
        last_heartbeat_at: Some(now.clone()),
        checkpoint_at: Some(now),
    }
}

pub(crate) fn queued_event(
    task_id: String,
    file: &super::PodcastFilePreview,
    cache_bytes: u64,
) -> TaskEvent {
    let snapshot = snapshot_for_file(FileSnapshotParams {
        task_id,
        file,
        cache_bytes,
        revision: 1,
        last_sequence: 1,
        stage: "queued",
        status: "waiting",
        label: "等待转写",
        percent: None,
        completed_units: None,
        total_units: None,
        created_at: None,
    });
    TaskEvent {
        schema_version: 1,
        task_id: snapshot.id.clone(),
        sequence: 1,
        revision: 1,
        event_type: "queued".to_string(),
        created_at: snapshot.created_at.clone(),
        snapshot,
    }
}

fn create_tasks(
    stored: &StoredPreview,
    control: &mut ControlDb,
    locations: &StorageLocations,
    duplicate_policy: DuplicatePolicy,
    budget_approval: Option<&PodcastBudgetApproval>,
    broadcast: &mut impl FnMut(&TaskEvent),
) -> Result<PodcastAddResult, String> {
    let mut tasks = Vec::new();
    let mut existing_books = Vec::new();
    for file in &stored.preview.files {
        if duplicate_policy == DuplicatePolicy::ReuseExisting {
            if let Some(book_id) = &file.duplicate_book_id {
                existing_books.push(book_id.clone());
                continue;
            }
        }
        let task_id = Uuid::new_v4().simple().to_string();
        // Visible snapshot first so UI shows copy progress within 1s.
        let preparing = snapshot_for_file(FileSnapshotParams {
            task_id: task_id.clone(),
            file,
            cache_bytes: file.bytes,
            revision: 1,
            last_sequence: 1,
            stage: "input_copy",
            status: "copying",
            label: "正在复制输入",
            percent: Some(0.0),
            completed_units: Some(0),
            total_units: Some(file.bytes),
            created_at: None,
        });
        let preparing_event = TaskEvent {
            schema_version: 1,
            task_id: preparing.id.clone(),
            sequence: 1,
            revision: 1,
            event_type: "input_copy_started".to_string(),
            created_at: preparing.created_at.clone(),
            snapshot: preparing.clone(),
        };
        control.persist_task_event(&preparing_event)?;
        broadcast(&preparing_event);

        let mut last_revision = 1_u64;
        let mut last_sequence = 1_u64;
        let created_at = preparing.created_at.clone();
        let input = {
            let mut progress_cb = |copied: u64, total: u64| {
                let percent = if total == 0 {
                    100.0
                } else {
                    (copied as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
                };
                last_revision = last_revision.saturating_add(1);
                last_sequence = last_sequence.saturating_add(1);
                let label = format!("复制输入 {copied} / {total} 字节");
                let snapshot = snapshot_for_file(FileSnapshotParams {
                    task_id: task_id.clone(),
                    file,
                    cache_bytes: total,
                    revision: last_revision,
                    last_sequence,
                    stage: "input_copy",
                    status: "copying",
                    label: &label,
                    percent: Some(percent),
                    completed_units: Some(copied),
                    total_units: Some(total),
                    created_at: Some(created_at.clone()),
                });
                let event = TaskEvent {
                    schema_version: 1,
                    task_id: snapshot.id.clone(),
                    sequence: snapshot.last_sequence,
                    revision: snapshot.revision,
                    event_type: "input_copy_progress".to_string(),
                    created_at: snapshot.updated_at.clone(),
                    snapshot,
                };
                if control.persist_task_event(&event).is_ok() {
                    broadcast(&event);
                }
            };
            copy_verified_input_with_progress(
                Path::new(&file.path),
                locations,
                &task_id,
                &file.input_sha256,
                file.bytes,
                Some(&mut progress_cb),
            )
        };
        let input = match input {
            Ok(value) => value,
            Err(error) => {
                last_revision = last_revision.saturating_add(1);
                last_sequence = last_sequence.saturating_add(1);
                let mut failed = snapshot_for_file(FileSnapshotParams {
                    task_id: task_id.clone(),
                    file,
                    cache_bytes: file.bytes,
                    revision: last_revision,
                    last_sequence,
                    stage: "input_copy_failed",
                    status: "exited",
                    label: "输入复制失败",
                    percent: None,
                    completed_units: None,
                    total_units: Some(file.bytes),
                    created_at: Some(created_at.clone()),
                });
                failed.lifecycle_state = LifecycleState::Terminal;
                failed.outcome = TaskOutcome::Failed;
                failed.error_code = Some(crate::tasks::TaskErrorCode::InputCopyFailed);
                failed.error_message = Some(error);
                failed.recoverable = false;
                failed.can_cancel = false;
                let event = TaskEvent {
                    schema_version: 1,
                    task_id: failed.id.clone(),
                    sequence: failed.last_sequence,
                    revision: failed.revision,
                    event_type: "input_copy_failed".to_string(),
                    created_at: failed.updated_at.clone(),
                    snapshot: failed,
                };
                let _ = control.persist_task_event(&event);
                broadcast(&event);
                // Continue remaining files independently.
                continue;
            }
        };

        let book_revision = u64::from(file.duplicate_book_id.is_some()) + 1;
        let contract = TaskContractRequest {
            task_id: &task_id,
            file,
            input: &input,
            options: &stored.options,
            duplicate_policy,
            budget: &stored.preview.budget,
            budget_approval,
            revision: book_revision,
        };
        let compatibility = write_task_contract(locations, &contract)?;
        set_podcast_recovery_compatibility(locations, &task_id, compatibility)?;
        last_revision = last_revision.saturating_add(1);
        last_sequence = last_sequence.saturating_add(1);
        let ready = snapshot_for_file(FileSnapshotParams {
            task_id,
            file,
            cache_bytes: input.bytes,
            revision: last_revision,
            last_sequence,
            stage: "queued",
            status: "waiting",
            label: "输入就绪，等待转写",
            percent: Some(100.0),
            completed_units: Some(input.bytes),
            total_units: Some(input.bytes),
            created_at: Some(created_at),
        });
        let event = TaskEvent {
            schema_version: 1,
            task_id: ready.id.clone(),
            sequence: ready.last_sequence,
            revision: ready.revision,
            event_type: "queued".to_string(),
            created_at: ready.updated_at.clone(),
            snapshot: ready.clone(),
        };
        control.persist_task_event(&event)?;
        broadcast(&event);
        tasks.push(event.snapshot);
    }
    Ok(PodcastAddResult {
        tasks,
        existing_books,
    })
}

pub fn add_podcast_files_at(
    store: &PodcastPreviewStore,
    control: &mut ControlDb,
    locations: &StorageLocations,
    request: &AddPodcastFilesRequest<'_>,
    mut broadcast: impl FnMut(&TaskEvent),
) -> Result<PodcastAddResult, String> {
    if request.request_id.trim().is_empty() {
        return Err("INVALID_ARGUMENT".to_string());
    }
    let input_hash = request_hash(request)?;
    match control.claim_command(request.request_id, COMMAND_NAME, &input_hash)? {
        CommandClaim::Existing(record) => return replay(record),
        CommandClaim::New => {}
    }
    let result = (|| {
        let stored = store.get(request.preview_id)?;
        validate_budget(&stored, request.budget_approval)?;
        create_tasks(
            &stored,
            control,
            locations,
            request.duplicate_policy,
            request.budget_approval,
            &mut broadcast,
        )
    })();
    match result {
        Ok(value) => {
            let json = serde_json::to_string(&value).map_err(|error| error.to_string())?;
            let revision = value.tasks.iter().map(|task| task.revision).max();
            control.complete_command(
                request.request_id,
                &json,
                None,
                revision.and_then(|value| i64::try_from(value).ok()),
            )?;
            store.remove(request.preview_id);
            Ok(value)
        }
        Err(error_code) => {
            control.complete_command(request.request_id, "{}", Some(&error_code), None)?;
            Err(error_code)
        }
    }
}

#[cfg(test)]
mod tests;
