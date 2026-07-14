use crate::control::{CommandClaim, ControlDb};
use crate::settings::AppSettings;
use crate::tasks::{
    LifecycleState, ProgressMode, RequiredAction, TaskErrorCode, TaskEvent, TaskKind, TaskOutcome,
    TaskProgress, TaskSnapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const TASK_EVENT_NAME: &str = "acquisition://task-event";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_EMIT_INTERVAL: Duration = Duration::from_secs(5);

static ACTIVE_POLLERS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn active_pollers() -> &'static Mutex<HashSet<String>> {
    ACTIVE_POLLERS.get_or_init(|| Mutex::new(HashSet::new()))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateZhihuTaskRequest {
    pub people_id: String,
    pub item_types: ZhihuItemTypes,
    pub top_n: Option<u32>,
    pub sort_by: ZhihuSortBy,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZhihuLoginStatus {
    pub logged_in: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ZhihuItemTypes {
    Answers,
    Articles,
    All,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ZhihuSortBy {
    Time,
    Vote,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateResponse {
    success: bool,
    task_id: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct RemoteTask {
    id: String,
    author_id: String,
    status: String,
    total_count: u64,
    success_count: u64,
    failed_count: u64,
}

fn validate_request(request: &CreateZhihuTaskRequest) -> Result<(), String> {
    if request.people_id.is_empty()
        || request.people_id.len() > 80
        || !request
            .people_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
    {
        return Err("INVALID_ZHIHU_PEOPLE_ID".to_string());
    }
    if let Some(top_n) = request.top_n {
        if !(1..=5_000).contains(&top_n) {
            return Err("INVALID_ZHIHU_TOP_N".to_string());
        }
    }
    Ok(())
}

fn initial_snapshot(task_id: &str, request: &CreateZhihuTaskRequest) -> TaskSnapshot {
    let now = chrono::Utc::now().to_rfc3339();
    TaskSnapshot {
        id: task_id.to_string(),
        kind: TaskKind::Zhihu,
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
            label: Some("等待开始".to_string()),
            unit: Some("篇".to_string()),
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
        book_id: Some(format!("zhihu:{}", request.people_id)),
        source_id: Some(request.people_id.clone()),
        cache_lease_bytes: 0,
        created_at: now.clone(),
        updated_at: now,
        last_heartbeat_at: None,
        checkpoint_at: None,
    }
}

fn create_event(snapshot: TaskSnapshot) -> TaskEvent {
    TaskEvent {
        schema_version: 1,
        task_id: snapshot.id.clone(),
        sequence: snapshot.last_sequence,
        revision: snapshot.revision,
        event_type: "queued".to_string(),
        created_at: snapshot.created_at.clone(),
        snapshot,
    }
}

pub fn create_task(
    settings: &AppSettings,
    request: &CreateZhihuTaskRequest,
) -> Result<TaskSnapshot, String> {
    validate_request(request)?;
    let response: CreateResponse = crate::tools::zhihu_post_json(
        settings,
        "/api/tasks",
        &serde_json::json!({
            "peopleId": request.people_id,
            "itemTypes": request.item_types,
            "topN": request.top_n,
            "sortBy": request.sort_by,
        }),
    )?;
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| "ZHIHU_TASK_CREATE_FAILED".to_string()));
    }
    let task_id = response
        .task_id
        .ok_or_else(|| "ZHIHU_TASK_ID_MISSING".to_string())?;
    crate::cache::validate_task_id(&task_id)?;
    let snapshot = initial_snapshot(&task_id, request);
    let event = create_event(snapshot.clone());
    ControlDb::open_current()?.persist_task_event(&event)?;
    Ok(snapshot)
}

pub fn login_status(settings: &AppSettings) -> Result<ZhihuLoginStatus, String> {
    let response: ApiResponse<ZhihuLoginStatus> =
        crate::tools::zhihu_get_json(settings, "/api/login-status")?;
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| "ZHIHU_LOGIN_STATUS_FAILED".to_string()));
    }
    response
        .data
        .ok_or_else(|| "ZHIHU_LOGIN_STATUS_MISSING".to_string())
}

pub fn start_login(settings: &AppSettings) -> Result<(), String> {
    let response: ApiResponse<serde_json::Value> =
        crate::tools::zhihu_post_json(settings, "/api/login/start", &serde_json::json!({}))?;
    if response.success {
        Ok(())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "ZHIHU_LOGIN_START_FAILED".to_string()))
    }
}

pub fn start_task(
    task_id: &str,
    expected_revision: u64,
    settings: &AppSettings,
    app: &AppHandle,
) -> Result<TaskSnapshot, String> {
    crate::cache::validate_task_id(task_id)?;
    let response: ApiResponse<serde_json::Value> = crate::tools::zhihu_post_json(
        settings,
        &format!("/api/tasks/{task_id}/start"),
        &serde_json::json!({}),
    )?;
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| "ZHIHU_TASK_START_FAILED".to_string()));
    }
    let mut control = ControlDb::open_current()?;
    let current = control
        .task_snapshot(task_id)?
        .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
    if current.kind != TaskKind::Zhihu || current.revision != expected_revision {
        return Err(if current.revision != expected_revision {
            "REVISION_CONFLICT".to_string()
        } else {
            "TASK_KIND_CONFLICT".to_string()
        });
    }
    let event = control
        .mark_task_starting(task_id)?
        .ok_or_else(|| "TASK_NOT_QUEUED".to_string())?;
    let snapshot = event.snapshot.clone();
    app.emit(TASK_EVENT_NAME, event)
        .map_err(|error| error.to_string())?;
    ensure_poller(task_id.to_string(), settings.clone(), app.clone());
    Ok(snapshot)
}

pub fn control_task(
    task_id: &str,
    action: &str,
    expected_revision: u64,
    request_id: &str,
    settings: &AppSettings,
    app: &AppHandle,
) -> Result<TaskSnapshot, String> {
    crate::cache::validate_task_id(task_id)?;
    if request_id.trim().is_empty() {
        return Err("INVALID_REQUEST_ID".to_string());
    }
    if !matches!(action, "pause" | "resume" | "cancel") {
        return Err("INVALID_TASK_CONTROL".to_string());
    }
    let input_hash = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&serde_json::json!({
                "taskId": task_id,
                "action": action,
                "expectedRevision": expected_revision,
            }))
            .map_err(|error| error.to_string())?,
        )
    );
    let mut control = ControlDb::open_current()?;
    match control.claim_command(request_id, "control_zhihu_task", &input_hash)? {
        CommandClaim::Existing(record) => {
            if let Some(error) = record.error_code {
                return Err(error);
            }
            serde_json::from_str(
                record
                    .result_json
                    .as_deref()
                    .ok_or_else(|| "COMMAND_RESULT_MISSING".to_string())?,
            )
            .map_err(|error| error.to_string())
        }
        CommandClaim::New => {
            let result = (|| {
                let path = match action {
                    "pause" => format!("/api/tasks/{task_id}/pause"),
                    "resume" => format!("/api/tasks/{task_id}/start"),
                    "cancel" => format!("/api/tasks/{task_id}/cancel"),
                    _ => unreachable!(),
                };
                let response: ApiResponse<serde_json::Value> =
                    crate::tools::zhihu_post_json(settings, &path, &serde_json::json!({}))?;
                if !response.success {
                    return Err(response
                        .error
                        .unwrap_or_else(|| "ZHIHU_TASK_CONTROL_FAILED".to_string()));
                }
                let event = control.control_task(task_id, action, expected_revision)?;
                app.emit(TASK_EVENT_NAME, event.clone())
                    .map_err(|error| error.to_string())?;
                Ok(event.snapshot)
            })();
            match result {
                Ok(snapshot) => {
                    let json =
                        serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
                    control.complete_command(
                        request_id,
                        &json,
                        None,
                        i64::try_from(snapshot.revision).ok(),
                    )?;
                    Ok(snapshot)
                }
                Err(error) => {
                    control.complete_command(request_id, "{}", Some(&error), None)?;
                    Err(error)
                }
            }
        }
    }
}

fn remote_snapshot(remote: RemoteTask) -> TaskSnapshot {
    let terminal = matches!(
        remote.status.as_str(),
        "success" | "partial_success" | "failed"
    );
    let lifecycle_state = match remote.status.as_str() {
        "running" => LifecycleState::Running,
        "paused" => LifecycleState::Paused,
        "success" | "partial_success" | "failed" => LifecycleState::Terminal,
        _ => LifecycleState::Queued,
    };
    let outcome = match remote.status.as_str() {
        "success" => TaskOutcome::Success,
        "partial_success" => TaskOutcome::PartialSuccess,
        "failed" => TaskOutcome::Failed,
        _ => TaskOutcome::None,
    };
    let completed = remote.success_count.saturating_add(remote.failed_count);
    let determinate = remote.total_count > 0;
    let percent = determinate
        .then(|| (completed as f64 / remote.total_count as f64 * 100.0).clamp(0.0, 100.0));
    let label = if terminal {
        Some(format!(
            "完成 {} / {}（失败 {}）",
            remote.success_count, remote.total_count, remote.failed_count
        ))
    } else {
        Some(format!(
            "{} · {} / {}",
            if remote.status == "paused" {
                "已暂停"
            } else {
                "抓取中"
            },
            completed,
            remote.total_count
        ))
    };
    let index_complete = remote.index_status_is_complete();
    let task_id = remote.id;
    let author_id = remote.author_id;
    let now = chrono::Utc::now().to_rfc3339();
    TaskSnapshot {
        id: task_id,
        kind: TaskKind::Zhihu,
        revision: 1,
        last_sequence: 1,
        lifecycle_state,
        outcome,
        required_action: RequiredAction::None,
        progress: TaskProgress {
            mode: if determinate {
                ProgressMode::Determinate
            } else {
                ProgressMode::Indeterminate
            },
            percent,
            completed_units: Some(completed),
            total_units: Some(remote.total_count),
            label,
            unit: Some("篇".to_string()),
            source_total_units: None,
            skipped_units: None,
        },
        error_code: if remote.status == "failed" {
            Some(TaskErrorCode::Unknown)
        } else {
            None
        },
        error_message: if remote.status == "failed" {
            Some("知乎 sidecar 报告任务失败。".to_string())
        } else {
            None
        },
        retry_after_seconds: None,
        engine_stage: if index_complete {
            "content".to_string()
        } else {
            "index".to_string()
        },
        engine_status: remote.status.clone(),
        recoverable: !matches!(remote.status.as_str(), "success"),
        can_pause: remote.status == "running",
        can_resume: remote.status == "paused",
        can_retry: terminal && remote.status != "success",
        can_cancel: !terminal,
        book_id: if author_id.is_empty() {
            None
        } else {
            Some(format!("zhihu:{}", author_id))
        },
        source_id: (!author_id.is_empty()).then_some(author_id),
        cache_lease_bytes: 0,
        created_at: now.clone(),
        updated_at: now.clone(),
        last_heartbeat_at: Some(now.clone()),
        checkpoint_at: Some(now),
    }
}

impl RemoteTask {
    fn index_status_is_complete(&self) -> bool {
        self.total_count > 0
            && self.success_count.saturating_add(self.failed_count) >= self.total_count
    }
}

fn fetch_remote_task(settings: &AppSettings, task_id: &str) -> Result<RemoteTask, String> {
    let response: ApiResponse<RemoteTask> =
        crate::tools::zhihu_get_json(settings, &format!("/api/tasks/{task_id}"))?;
    if !response.success {
        return Err(response
            .error
            .unwrap_or_else(|| "ZHIHU_TASK_FETCH_FAILED".to_string()));
    }
    response
        .data
        .ok_or_else(|| "ZHIHU_TASK_MISSING".to_string())
}

/// Apply sidecar task state into the control DB. Sidecar success overrides a
/// false local "interrupted/crashed" terminal mirror.
fn apply_remote_task(
    remote: RemoteTask,
    app: Option<&AppHandle>,
) -> Result<Option<TaskEvent>, String> {
    let terminal = matches!(
        remote.status.as_str(),
        "success" | "partial_success" | "failed"
    );
    let next = remote_snapshot(remote);
    let mut control = ControlDb::open_current()?;
    let event = control.record_external_snapshot(
        next,
        if terminal {
            "engine_completed"
        } else {
            "engine_progress"
        },
    )?;
    if let (Some(app), Some(event)) = (app, event.as_ref()) {
        let _ = app.emit(TASK_EVENT_NAME, event.clone());
    }
    Ok(event)
}

/// Ensure a durable per-task supervisor is running (deduped by task id).
/// Runs until the task reaches a real terminal state — no 10-minute cap.
pub fn ensure_poller(task_id: String, settings: AppSettings, app: AppHandle) {
    {
        let Ok(mut guard) = active_pollers().lock() else {
            return;
        };
        if !guard.insert(task_id.clone()) {
            return;
        }
    }
    thread::spawn(move || {
        let mut last_heartbeat_emit = Instant::now()
            .checked_sub(HEARTBEAT_EMIT_INTERVAL)
            .unwrap_or_else(Instant::now);
        loop {
            let local_terminal = ControlDb::open_current()
                .ok()
                .and_then(|control| control.task_snapshot(&task_id).ok().flatten())
                .is_some_and(|snapshot| {
                    snapshot.lifecycle_state == LifecycleState::Terminal
                        && !matches!(snapshot.outcome, TaskOutcome::Interrupted)
                });
            if local_terminal {
                break;
            }

            match fetch_remote_task(&settings, &task_id) {
                Ok(remote) => {
                    let remote_terminal = matches!(
                        remote.status.as_str(),
                        "success" | "partial_success" | "failed"
                    );
                    let progress_changed = apply_remote_task(remote, Some(&app)).ok().flatten();
                    if progress_changed.is_none()
                        && !remote_terminal
                        && last_heartbeat_emit.elapsed() >= HEARTBEAT_EMIT_INTERVAL
                    {
                        if let Ok(mut control) = ControlDb::open_current() {
                            if let Ok(Some(mut snapshot)) = control.task_snapshot(&task_id) {
                                let now = chrono::Utc::now().to_rfc3339();
                                snapshot.last_heartbeat_at = Some(now.clone());
                                if let Ok(Some(event)) =
                                    control.record_external_snapshot(snapshot, "engine_heartbeat")
                                {
                                    let _ = app.emit(TASK_EVENT_NAME, event);
                                    last_heartbeat_emit = Instant::now();
                                }
                            }
                        }
                    } else if progress_changed.is_some() {
                        last_heartbeat_emit = Instant::now();
                    }
                    if remote_terminal {
                        break;
                    }
                }
                Err(_) => {
                    // Keep supervising; transient sidecar/network blips should not stop the loop.
                }
            }
            thread::sleep(POLL_INTERVAL);
        }
        if let Ok(mut guard) = active_pollers().lock() {
            guard.remove(&task_id);
        }
    });
}

/// Reconcile non-terminal (and falsely interrupted) Zhihu tasks with the sidecar.
/// Starts durable supervisors for still-active remote tasks.
pub fn reconcile_active_tasks(
    settings: &AppSettings,
    app: Option<&AppHandle>,
) -> Result<u32, String> {
    let control = ControlDb::open_current()?;
    let tasks = control.task_snapshots(Some(TaskKind::Zhihu))?;
    let mut updated = 0u32;
    for snapshot in tasks {
        let needs_reconcile = matches!(
            snapshot.lifecycle_state,
            LifecycleState::Starting
                | LifecycleState::Running
                | LifecycleState::Pausing
                | LifecycleState::Paused
                | LifecycleState::Stopping
        ) || (snapshot.lifecycle_state == LifecycleState::Terminal
            && matches!(snapshot.outcome, TaskOutcome::Interrupted));
        if !needs_reconcile {
            continue;
        }
        match fetch_remote_task(settings, &snapshot.id) {
            Ok(remote) => {
                let remote_active = matches!(remote.status.as_str(), "running" | "paused");
                if apply_remote_task(remote, app)?.is_some() {
                    updated = updated.saturating_add(1);
                }
                if remote_active {
                    if let Some(app) = app {
                        ensure_poller(snapshot.id.clone(), settings.clone(), app.clone());
                    } else {
                        // Snapshot path without AppHandle still updates DB; poller starts on next start/resume.
                    }
                }
            }
            Err(_) => {
                // Leave local state; avoid marking crashed solely because the sidecar was briefly down.
            }
        }
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::{remote_snapshot, CreateZhihuTaskRequest, RemoteTask, ZhihuItemTypes, ZhihuSortBy};
    use crate::tasks::{LifecycleState, TaskOutcome};

    #[test]
    fn validates_create_request_bounds() {
        let mut request = CreateZhihuTaskRequest {
            people_id: "author_1".to_string(),
            item_types: ZhihuItemTypes::All,
            top_n: Some(5),
            sort_by: ZhihuSortBy::Time,
        };
        assert!(super::validate_request(&request).is_ok());
        request.top_n = Some(5_001);
        assert!(super::validate_request(&request).is_err());
    }

    #[test]
    fn maps_remote_terminal_progress_to_shared_snapshot() {
        let snapshot = remote_snapshot(RemoteTask {
            id: "task_author_1_1".to_string(),
            author_id: "author_1".to_string(),
            status: "partial_success".to_string(),
            total_count: 5,
            success_count: 4,
            failed_count: 1,
        });
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Terminal);
        assert_eq!(snapshot.outcome, TaskOutcome::PartialSuccess);
        assert_eq!(snapshot.progress.percent, Some(100.0));
        assert!(snapshot.can_retry);
        assert_eq!(snapshot.book_id.as_deref(), Some("zhihu:author_1"));
        assert!(snapshot.last_heartbeat_at.is_some());
        assert_eq!(snapshot.progress.unit.as_deref(), Some("篇"));
    }

    #[test]
    fn maps_running_remote_progress_with_heartbeat() {
        let snapshot = remote_snapshot(RemoteTask {
            id: "task_author_2_1".to_string(),
            author_id: "author_2".to_string(),
            status: "running".to_string(),
            total_count: 122,
            success_count: 90,
            failed_count: 0,
        });
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Running);
        assert!(snapshot.can_pause);
        assert!(!snapshot.can_resume);
        assert_eq!(snapshot.progress.completed_units, Some(90));
        assert_eq!(snapshot.progress.total_units, Some(122));
        assert!(snapshot.last_heartbeat_at.is_some());
    }
}
