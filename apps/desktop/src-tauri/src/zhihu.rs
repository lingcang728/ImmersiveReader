use crate::control::{CommandClaim, ControlDb};
use crate::settings::AppSettings;
use crate::tasks::{
    LifecycleState, ProgressMode, RequiredAction, TaskErrorCode, TaskEvent, TaskKind, TaskOutcome,
    TaskProgress, TaskSnapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const TASK_EVENT_NAME: &str = "acquisition://task-event";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_EMIT_INTERVAL: Duration = Duration::from_secs(5);
/// Hard wall-clock cap for one `reconcile_active_tasks` pass. The reconcile
/// runs on the acquisition-snapshot path and must never scale with the number
/// of local tasks (previously N x ~15s of serial sidecar waits per pass).
const RECONCILE_DEADLINE: Duration = Duration::from_secs(20);
/// Maximum number of concurrent sidecar fetches during a reconcile pass.
const RECONCILE_MAX_CONCURRENCY: usize = 4;

/// Set while a reconcile pass is in flight so overlapping invocations from
/// sibling blocking threads return early instead of doubling the fan-out.
static RECONCILE_RUNNING: AtomicBool = AtomicBool::new(false);

/// RAII latch for [`RECONCILE_RUNNING`]; released on every exit path.
struct ReconcilePassGuard;

impl Drop for ReconcilePassGuard {
    fn drop(&mut self) {
        RECONCILE_RUNNING.store(false, Ordering::SeqCst);
    }
}

fn try_begin_reconcile_pass() -> Option<ReconcilePassGuard> {
    RECONCILE_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| ReconcilePassGuard)
}

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
    #[serde(default)]
    index_status: Option<String>,
    #[serde(default)]
    source_reported_count: Option<u64>,
    #[serde(default)]
    discovered_count: Option<u64>,
    #[serde(default)]
    index_checkpoint_json: Option<String>,
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
        display_name: Some(request.people_id.clone()),
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

/// Blocking: performs a bounded (~15s) sidecar HTTP request plus SQLite writes,
/// and may launch the sidecar on first use. Must run on a blocking context
/// (`tauri::async_runtime::spawn_blocking` or a dedicated thread) — never
/// directly on the Tauri IPC event-loop thread.
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

/// Blocking: one bounded (~15s) sidecar HTTP request, plus a possible sidecar
/// launch on first use. Call only from a blocking context — never directly on
/// the Tauri IPC event-loop thread.
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

/// Blocking: one bounded (~15s) sidecar HTTP request, plus a possible sidecar
/// launch on first use. Call only from a blocking context — never directly on
/// the Tauri IPC event-loop thread.
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

/// Blocking: SQLite reads/writes plus one bounded (~15s) sidecar HTTP request
/// (and a possible sidecar launch on first use). Call only from a blocking
/// context — never directly on the Tauri IPC event-loop thread.
pub fn start_task(
    task_id: &str,
    expected_revision: u64,
    settings: &AppSettings,
    app: &AppHandle,
) -> Result<TaskSnapshot, String> {
    crate::cache::validate_task_id(task_id)?;
    let mut control = ControlDb::open_current()?;
    control.validate_task_control(task_id, TaskKind::Zhihu, expected_revision)?;
    if let Some(event) = control.mark_task_starting(task_id)? {
        app.emit(TASK_EVENT_NAME, event)
            .map_err(|error| error.to_string())?;
    }
    let response: ApiResponse<serde_json::Value> =
        match crate::tools::zhihu_post_json(
            settings,
            &format!("/api/tasks/{task_id}/start"),
            &serde_json::json!({}),
        ) {
            Ok(response) => response,
            Err(error) => {
                if let Some(event) = control.rollback_starting_task(task_id)? {
                    let _ = app.emit(TASK_EVENT_NAME, event);
                }
                return Err(error);
            }
        };
    if !response.success {
        if let Some(event) = control.rollback_starting_task(task_id)? {
            let _ = app.emit(TASK_EVENT_NAME, event);
        }
        return Err(response
            .error
            .unwrap_or_else(|| "ZHIHU_TASK_START_FAILED".to_string()));
    }
    let snapshot = control
        .task_snapshot(task_id)?
        .ok_or_else(|| "TASK_NOT_FOUND".to_string())?;
    ensure_poller(task_id.to_string(), settings.clone(), app.clone());
    Ok(snapshot)
}

/// Blocking: SQLite reads/writes plus one bounded (~15s) sidecar HTTP request
/// (and a possible sidecar launch on first use). Call only from a blocking
/// context — never directly on the Tauri IPC event-loop thread.
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
                control.validate_task_control(task_id, TaskKind::Zhihu, expected_revision)?;
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
        "success" | "partial_success" | "failed" | "cancelled"
    );
    let lifecycle_state = match remote.status.as_str() {
        "running" => LifecycleState::Running,
        "paused" => LifecycleState::Paused,
        "success" | "partial_success" | "failed" | "cancelled" => LifecycleState::Terminal,
        _ => LifecycleState::Queued,
    };
    let outcome = match remote.status.as_str() {
        "success" => TaskOutcome::Success,
        "partial_success" => TaskOutcome::PartialSuccess,
        "failed" => TaskOutcome::Failed,
        "cancelled" => TaskOutcome::Cancelled,
        _ => TaskOutcome::None,
    };
    let completed = remote.success_count.saturating_add(remote.failed_count);
    let discovered = remote.discovered_count.unwrap_or(remote.total_count);
    let source_total = remote.source_reported_count;
    let determinate = remote.total_count > 0 || discovered > 0;
    let total_for_percent = if remote.total_count > 0 {
        remote.total_count
    } else {
        discovered
    };
    let percent = determinate.then(|| {
        if total_for_percent == 0 {
            0.0
        } else {
            (completed as f64 / total_for_percent as f64 * 100.0).clamp(0.0, 100.0)
        }
    });
    let index_complete = remote
        .index_status
        .as_deref()
        .map(|s| s == "complete")
        .unwrap_or_else(|| {
            remote.total_count > 0
                && remote.success_count.saturating_add(remote.failed_count) >= remote.total_count
        });
    let stage = if index_complete { "content" } else { "index" };
    let label = if terminal {
        Some(format!(
            "完成 {} / {}（失败 {}）",
            remote.success_count, remote.total_count, remote.failed_count
        ))
    } else if !index_complete {
        Some(format!(
            "索引中 · 已发现 {}{}",
            discovered,
            source_total
                .map(|t| format!(" / API {t}"))
                .unwrap_or_default()
        ))
    } else {
        Some(format!(
            "{} · 已归档 {} / {}（失败 {}）",
            if remote.status == "paused" {
                "已暂停"
            } else {
                "抓取正文"
            },
            remote.success_count,
            remote.total_count,
            remote.failed_count
        ))
    };
    let task_id = remote.id;
    let author_id = remote.author_id;
    let now = chrono::Utc::now().to_rfc3339();
    let checkpoint_at = remote
        .index_checkpoint_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.get("lastHeartbeatAt").and_then(|v| v.as_i64()))
        .map(|ms| {
            chrono::DateTime::from_timestamp_millis(ms)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| now.clone())
        });
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
            completed_units: Some(if index_complete {
                completed
            } else {
                discovered
            }),
            total_units: Some(if remote.total_count > 0 {
                remote.total_count
            } else {
                discovered
            }),
            label,
            unit: Some("篇".to_string()),
            source_total_units: source_total,
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
        engine_stage: stage.to_string(),
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
        source_id: (!author_id.is_empty()).then(|| author_id.clone()),
        display_name: (!author_id.is_empty()).then_some(author_id),
        cache_lease_bytes: 0,
        created_at: now.clone(),
        updated_at: now.clone(),
        last_heartbeat_at: Some(now.clone()),
        checkpoint_at: checkpoint_at.or(Some(now)),
    }
}

/// Blocking single-task fetch: one bounded (~15s) sidecar HTTP request via
/// `tools::zhihu_get_json` (which may also launch the sidecar on first use).
/// Only call from blocking threads — poller threads and reconcile workers, or
/// a `spawn_blocking` context — never on the Tauri IPC event-loop thread.
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
/// Blocking (SQLite + event emit only — no sidecar I/O); keep it on the
/// calling thread so DB writes stay serialized. Runs on poller threads and on
/// the reconcile caller thread; never on the IPC event-loop thread.
fn apply_remote_task(
    remote: RemoteTask,
    app: Option<&AppHandle>,
) -> Result<Option<TaskEvent>, String> {
    let terminal = matches!(
        remote.status.as_str(),
        "success" | "partial_success" | "failed" | "cancelled"
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
/// Non-blocking itself: all polling (`fetch_remote_task`, bounded ~15s per
/// request) happens on the spawned supervisor thread.
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
                        "success" | "partial_success" | "failed" | "cancelled"
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

/// Reconcile non-terminal (and falsely interrupted) Zhihu tasks with the
/// sidecar. Starts durable supervisors for still-active remote tasks.
///
/// # Blocking contract
///
/// This function performs blocking SQLite work and bounded sidecar HTTP
/// fan-out: per-task fetches run on a pool of at most
/// [`RECONCILE_MAX_CONCURRENCY`] detached worker threads (each request keeps
/// the sidecar client's own ~15s timeout) while this thread applies results
/// serially until every task has answered or [`RECONCILE_DEADLINE`] elapses.
/// It therefore never scales with the number of local tasks (previously a
/// serial N x ~15s per pass on the snapshot path).
///
/// It MUST NOT be invoked directly on the Tauri IPC event-loop thread —
/// command-path callers must dispatch it via
/// `tauri::async_runtime::spawn_blocking` (or another dedicated blocking
/// thread). It is safe to invoke from several blocking threads at once:
/// overlapping passes return `Ok(0)` immediately instead of doubling the
/// sidecar fan-out, and the in-flight pass's writes land in the same control
/// DB the caller reads afterwards.
pub fn reconcile_active_tasks(
    settings: &AppSettings,
    app: Option<&AppHandle>,
) -> Result<u32, String> {
    let Some(_pass) = try_begin_reconcile_pass() else {
        // A sibling blocking thread is already reconciling; its writes land
        // in the same control DB, so skipping this pass is safe.
        return Ok(0);
    };
    let control = ControlDb::open_current()?;
    let tasks = control.task_snapshots(Some(TaskKind::Zhihu))?;
    let mut pending = VecDeque::new();
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
        if needs_reconcile {
            pending.push_back(snapshot.id);
        }
    }
    let total = pending.len();
    if total == 0 {
        return Ok(0);
    }

    // Bounded fan-out: workers pull task ids and fetch remote state while this
    // thread applies results as they arrive. Workers are deliberately detached
    // — joining them would defeat the deadline; they unwind on their own once
    // the result receiver is dropped (each in-flight request is itself bounded
    // by the sidecar client's ~15s timeout), and failed sends just end them.
    let work = Arc::new(Mutex::new(pending));
    let (results_tx, results_rx) = mpsc::channel::<(String, Result<RemoteTask, String>)>();
    for _ in 0..total.min(RECONCILE_MAX_CONCURRENCY) {
        let work = Arc::clone(&work);
        let results_tx = results_tx.clone();
        let settings = settings.clone();
        thread::spawn(move || loop {
            let task_id = {
                let Ok(mut queue) = work.lock() else {
                    return;
                };
                match queue.pop_front() {
                    Some(task_id) => task_id,
                    None => return,
                }
            };
            let result = fetch_remote_task(&settings, &task_id);
            if results_tx.send((task_id, result)).is_err() {
                return;
            }
        });
    }
    drop(results_tx);

    let deadline = Instant::now() + RECONCILE_DEADLINE;
    let mut updated = 0u32;
    let mut received = 0usize;
    while received < total {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // Outstanding fetches are abandoned; workers unwind once their
            // bounded request ends and the send fails. Unreconciled tasks are
            // picked up by the next snapshot pass.
            break;
        }
        match results_rx.recv_timeout(remaining) {
            Ok((task_id, Ok(remote))) => {
                received += 1;
                let remote_active = matches!(remote.status.as_str(), "running" | "paused");
                if apply_remote_task(remote, app)?.is_some() {
                    updated = updated.saturating_add(1);
                }
                if remote_active {
                    if let Some(app) = app {
                        ensure_poller(task_id, settings.clone(), app.clone());
                    }
                    // Snapshot path without AppHandle still updates the DB;
                    // the poller starts on next start/resume.
                }
            }
            Ok((_task_id, Err(_))) => {
                received += 1;
                // Leave local state; avoid marking crashed solely because the
                // sidecar was briefly down.
            }
            Err(_) => break, // deadline elapsed or all workers exited early
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
            index_status: Some("complete".to_string()),
            source_reported_count: Some(6),
            discovered_count: Some(5),
            index_checkpoint_json: None,
        });
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Terminal);
        assert_eq!(snapshot.outcome, TaskOutcome::PartialSuccess);
        assert_eq!(snapshot.progress.percent, Some(100.0));
        assert!(snapshot.can_retry);
        assert_eq!(snapshot.book_id.as_deref(), Some("zhihu:author_1"));
        assert!(snapshot.last_heartbeat_at.is_some());
        assert_eq!(snapshot.progress.unit.as_deref(), Some("篇"));
        assert_eq!(snapshot.progress.source_total_units, Some(6));
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
            index_status: Some("complete".to_string()),
            source_reported_count: Some(150),
            discovered_count: Some(122),
            index_checkpoint_json: None,
        });
        assert_eq!(snapshot.lifecycle_state, LifecycleState::Running);
        assert!(snapshot.can_pause);
        assert!(!snapshot.can_resume);
        assert_eq!(snapshot.progress.completed_units, Some(90));
        assert_eq!(snapshot.progress.total_units, Some(122));
        assert_eq!(snapshot.progress.source_total_units, Some(150));
        assert_eq!(snapshot.engine_stage, "content");
        assert!(snapshot.last_heartbeat_at.is_some());
    }

    #[test]
    fn maps_index_phase_using_discovered_and_api_totals() {
        let snapshot = remote_snapshot(RemoteTask {
            id: "task_author_3_1".to_string(),
            author_id: "author_3".to_string(),
            status: "running".to_string(),
            total_count: 0,
            success_count: 0,
            failed_count: 0,
            index_status: Some("running".to_string()),
            source_reported_count: Some(200),
            discovered_count: Some(90),
            index_checkpoint_json: Some(
                r#"{"isEnd":false,"discovered":90,"totals":200,"lastHeartbeatAt":0}"#.to_string(),
            ),
        });
        assert_eq!(snapshot.engine_stage, "index");
        assert_eq!(snapshot.progress.completed_units, Some(90));
        assert_eq!(snapshot.progress.source_total_units, Some(200));
        assert!(snapshot
            .progress
            .label
            .as_deref()
            .unwrap_or_default()
            .contains("已发现 90"));
    }
}
