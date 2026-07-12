use super::task_contract::{write_task_contract, TaskContractRequest};
use super::task_request::{replay, request_hash, validate_budget, StoredPreview};
use super::{
    copy_verified_input, AddPodcastFilesRequest, DuplicatePolicy, PodcastAddResult,
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

pub(crate) fn queued_event(
    task_id: String,
    file: &super::PodcastFilePreview,
    cache_bytes: u64,
) -> TaskEvent {
    let now = chrono::Utc::now().to_rfc3339();
    let snapshot = TaskSnapshot {
        id: task_id,
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
            label: Some("等待转写".to_string()),
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
        book_id: Some(file.book_id.clone()),
        source_id: Some(file.source_id.clone()),
        cache_lease_bytes: cache_bytes,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    TaskEvent {
        schema_version: 1,
        task_id: snapshot.id.clone(),
        sequence: 1,
        revision: 1,
        event_type: "queued".to_string(),
        snapshot,
        created_at: now,
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
        let input = copy_verified_input(
            Path::new(&file.path),
            locations,
            &task_id,
            &file.input_sha256,
            file.bytes,
        )?;
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
        let event = queued_event(task_id, file, input.bytes);
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
