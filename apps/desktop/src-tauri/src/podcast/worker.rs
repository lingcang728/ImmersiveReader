use crate::control::ControlDb;
use crate::settings::AppSettings;
use crate::storage::StorageLocations;
use crate::tasks::{LifecycleState, TaskEvent, TaskKind};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use crate::job_object::JobObject;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle};
#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;

#[cfg(windows)]
type WorkerJob = JobObject;
#[cfg(not(windows))]
type WorkerJob = ();

/// P1-5: each worker loads a multi-GB Whisper model — run at most one at a
/// time. Additional tasks stay Queued and are pulled up when the slot frees.
const MAX_CONCURRENT_WORKERS: usize = 1;
/// P1-5: bound the pull-next loop so a permanently failing task cannot spin it.
const MAX_QUEUE_DRAIN_ATTEMPTS: usize = 16;
/// P1-2: once the worker has exited, drain its pipes at most this long. A
/// grandchild (e.g. ffmpeg spawned by python) that inherited the pipe write
/// end otherwise keeps EOF — and the pump loop — from ever arriving.
const POST_EXIT_DRAIN_GRACE: Duration = Duration::from_secs(2);

type ChildHandle = Arc<Mutex<Option<Child>>>;
struct WorkerEntry {
    // P3-26: no `Child` slot — `run_worker` owns the Child and the
    // non-Windows terminate path signals the pinned `pid` instead, so a
    // stored Option<Child> would be write-only.
    /// Process handle duplicated at spawn time (P1-3). While it stays open the
    /// OS cannot recycle the PID, so suspend/resume/terminate always land on
    /// our worker and never on an unrelated process that reused the PID.
    #[cfg(windows)]
    process: OwnedHandle,
    pid: u32,
    /// Pairing truth for P1-4: true while this process's threads are suspended
    /// by `pause_task`. Mutated only under the `workers()` mutex so a double
    /// pause can never stack a second suspend count.
    suspended: bool,
}

static ACTIVE_WORKERS: OnceLock<Mutex<HashMap<String, WorkerEntry>>> = OnceLock::new();

fn workers() -> &'static Mutex<HashMap<String, WorkerEntry>> {
    ACTIVE_WORKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Agreed accessor added to `control.rs` by a parallel change:
/// `pub fn task_lifecycle_state(&self, task_id: &str) -> Result<Option<String>, String>`
/// returning the serde snake_case `LifecycleState` name. This local trait
/// supplies the identical signature until that inherent method lands; inherent
/// methods shadow trait methods, so the real implementation then takes over
/// with no further edit needed here.
#[allow(dead_code)]
trait ControlDbLifecycleState {
    fn task_lifecycle_state(&self, task_id: &str) -> Result<Option<String>, String>;
}

#[allow(dead_code)]
impl ControlDbLifecycleState for ControlDb {
    fn task_lifecycle_state(&self, task_id: &str) -> Result<Option<String>, String> {
        Ok(self.task_snapshot(task_id)?.map(|snapshot| {
            serde_json::to_value(&snapshot.lifecycle_state)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default()
        }))
    }
}

/// DB lifecycle for `task_id` (None when the DB or row is unreadable/absent).
fn worker_lifecycle_state(task_id: &str) -> Option<String> {
    ControlDb::open_current()
        .ok()
        .and_then(|control| control.task_lifecycle_state(task_id).ok().flatten())
}

fn is_paused_state(state: &str) -> bool {
    state.eq_ignore_ascii_case("paused") || state.eq_ignore_ascii_case("pausing")
}

fn is_terminal_state(state: &str) -> bool {
    state.eq_ignore_ascii_case("terminal") || state.eq_ignore_ascii_case("stopping")
}

fn emit_task(app: &AppHandle, event: &TaskEvent) {
    let _ = app.emit(super::tasks::TASK_EVENT_NAME, event);
}

fn podcast_worker_command(
    locations: &StorageLocations,
    settings: &AppSettings,
    task_id: &str,
) -> Result<Command, String> {
    let executable = locations.runtime_root.join("podcast/python/python.exe");
    let script = locations
        .runtime_root
        .join("podcast/app/scripts/transcribe_task.py");
    let task_spec = locations
        .data_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id)
        .join("task.json");
    let data_root = locations.data_root.join("Podcast");
    let cache_root = locations
        .cache_root
        .join("Podcast")
        .join("Tasks")
        .join(task_id);
    for path in [&executable, &script, &task_spec, &data_root, &cache_root] {
        if !path.exists() {
            return Err(format!("WORKER_RUNTIME_MISSING: {}", path.display()));
        }
    }
    let mut path_parts = vec![locations.runtime_root.join("podcast/ffmpeg")];
    if let Some(existing) = std::env::var_os("PATH") {
        path_parts.extend(std::env::split_paths(&existing));
    }
    let mut command = Command::new(executable);
    command
        .arg(script)
        .arg("--task-spec")
        .arg(task_spec)
        .current_dir(locations.runtime_root.join("podcast/app"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Vendored Python 3.12 predates PEP-686: on zh-CN Windows its piped
        // stdio defaults to cp936 while the worker emits ensure_ascii=False
        // Chinese JSON/log lines. Force UTF-8 stdio so host-side line
        // decoding always sees valid UTF-8 bytes.
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        // Models are vendored under runtime\podcast\models; faster-whisper must
        // fail fast on an unresolvable model reference instead of attempting a
        // ~1.6GB HF download from a machine that may be offline (P1-27).
        .env("HF_HUB_OFFLINE", "1")
        .env("IMMERSIVE_PODCAST_DATA_ROOT", data_root)
        .env("IMMERSIVE_PODCAST_CACHE_ROOT", cache_root)
        .env("IMMERSIVE_LIBRARY_ROOT", &settings.library_root)
        .env(
            "IMMERSIVE_PODCAST_MODEL_ROOT",
            locations.runtime_root.join("podcast/models"),
        )
        // P3-24: IMMERSIVE_PODCAST_PYTHON was injected but nothing in
        // tools/podcast-transcriber ever read it — removed.
        .env(
            "PATH",
            std::env::join_paths(path_parts).map_err(|error| error.to_string())?,
        );
    if let Some(api_key) =
        crate::secrets::deepseek_api_key(&crate::settings::AppChannel::current())?
    {
        command.env("DEEPSEEK_API_KEY", api_key);
    }
    Ok(command)
}

fn persist_starting(app: &AppHandle, task_id: &str) -> Result<(), String> {
    let mut control = ControlDb::open_current()?;
    if let Some(event) = control.mark_task_starting(task_id)? {
        emit_task(app, &event);
    }
    Ok(())
}

#[cfg(windows)]
fn spawn_worker(command: &mut Command) -> Result<(Child, Option<WorkerJob>), String> {
    let (child, job) = JobObject::spawn_suspended(command)?;
    Ok((child, Some(job)))
}

#[cfg(not(windows))]
fn spawn_worker(command: &mut Command) -> Result<(Child, Option<WorkerJob>), String> {
    Ok((command.spawn().map_err(|error| error.to_string())?, None))
}

/// P1-3: duplicate the child's process handle so the worker entry can pin the
/// exact process object. While any handle to a process is open the OS cannot
/// recycle its PID — a stored bare PID can otherwise name a stranger process.
#[cfg(windows)]
fn duplicate_process_handle(child: &Child) -> Result<OwnedHandle, String> {
    // SAFETY: `child` is alive here and its process handle is valid; the
    // borrow lasts only for the duration of the DuplicateHandle call inside
    // try_clone_to_owned (DUPLICATE_SAME_ACCESS — std's spawn handle carries
    // PROCESS_ALL_ACCESS, covering TERMINATE / QUERY_LIMITED_INFORMATION).
    let borrowed = unsafe { BorrowedHandle::borrow_raw(child.as_raw_handle()) };
    borrowed
        .try_clone_to_owned()
        .map_err(|error| format!("DuplicateHandle failed: {error}"))
}

fn read_stream<R: std::io::Read + Send + 'static>(
    stream: R,
    name: &'static str,
    sender: mpsc::Sender<(String, String)>,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) => break,
                Ok(_) => {
                    // Keep the BufRead::lines() framing contract: strip one
                    // trailing LF/CRLF so each send carries one complete
                    // NDJSON/log line.
                    if buffer.last() == Some(&b'\n') {
                        buffer.pop();
                        if buffer.last() == Some(&b'\r') {
                            buffer.pop();
                        }
                    }
                    // Lossy decode: stray non-UTF-8 bytes (e.g. cp936 output
                    // from a worker that ignored PYTHONUTF8) become U+FFFD
                    // instead of killing the pump and deadlocking the child
                    // on a full pipe.
                    let line = String::from_utf8_lossy(&buffer).into_owned();
                    let _ = sender.send((name.to_string(), line));
                }
                Err(error) => {
                    let _ = sender.send((name.to_string(), format!("stream read failed: {error}")));
                    break;
                }
            }
        }
    });
}

/// P1-28: the worker's structured `{"type":"fatal",...}` NDJSON line is the
/// authoritative failure reason (contract channel: stdout; stderr accepted too
/// for robustness). Returns the line when it is a fatal record.
fn worker_fatal_line(line: &str) -> Option<String> {
    let value = parse_worker_json(line)?;
    (value.get("type").and_then(Value::as_str) == Some("fatal")).then(|| line.to_string())
}

/// P2-11/P2-12: run `op` against the worker's held control.db connection
/// (`held`, opened once per worker instead of per output line). On a write
/// failure the possibly-poisoned connection is dropped and the op is retried
/// once on a fresh open — a lost event leaves the task stuck Running, so
/// silent `if let Ok` swallowing is not acceptable. Every failure is logged
/// to stderr; the final error is returned for the caller to escalate.
fn worker_db_call<T>(
    task_id: &str,
    held: &mut Option<ControlDb>,
    op: impl Fn(&mut ControlDb) -> Result<T, String>,
) -> Result<T, String> {
    let mut last_error = String::from("CONTROL_DB_UNAVAILABLE");
    for attempt in 0..2 {
        if held.is_none() {
            match ControlDb::open_current() {
                Ok(db) => *held = Some(db),
                Err(error) => {
                    eprintln!("podcast worker {task_id}: control.db open failed: {error}");
                    last_error = error;
                    continue;
                }
            }
        }
        match op(held.as_mut().expect("held connection was just opened")) {
            Ok(value) => return Ok(value),
            Err(error) => {
                eprintln!(
                    "podcast worker {task_id}: control.db write attempt {} failed: {error}",
                    attempt + 1
                );
                last_error = error;
                *held = None; // reopen on the next attempt
            }
        }
    }
    Err(last_error)
}

/// P2-11: `finish_worker_task` is the terminal event — losing it strands the
/// task in Running until the stale-worker watchdog reaps it. Retry once on a
/// fresh connection, and if persistence still fails say so loudly.
fn finish_task_logged(
    held: &mut Option<ControlDb>,
    task_id: &str,
    app: &AppHandle,
    success: bool,
    message: Option<&str>,
) {
    match worker_db_call(task_id, held, |control| {
        control.finish_worker_task(task_id, success, message)
    }) {
        Ok(Some(event)) => emit_task(app, &event),
        Ok(None) => {}
        Err(error) => eprintln!(
            "podcast worker {task_id}: terminal task event could not be persisted after retry: {error}; \
             task stays non-terminal until the stale-worker watchdog marks it interrupted"
        ),
    }
}

fn apply_worker_line(
    task_id: &str,
    db: &mut Option<ControlDb>,
    app: &AppHandle,
    stream: &str,
    line: &str,
    stderr_tail: &mut Option<String>,
    fatal_error: &mut Option<String>,
) {
    if stream == "stderr" && !line.trim().is_empty() {
        *stderr_tail = Some(line.to_string());
    }
    if let Some(fatal) = worker_fatal_line(line) {
        *fatal_error = Some(fatal);
    }
    // P2-11: a dropped write here leaves the task Running forever — retry on
    // a fresh connection once and log the loss instead of `if let Ok`.
    match worker_db_call(task_id, db, |control| {
        control.record_worker_line(task_id, stream, line)
    }) {
        Ok(Some(event)) => emit_task(app, &event),
        Ok(None) => {}
        Err(error) => {
            eprintln!("podcast worker {task_id}: worker line event lost after retry: {error}")
        }
    }
}

fn run_worker(task_id: String, app: AppHandle, child_handle: ChildHandle, job: Option<WorkerJob>) {
    // P2-12: one control.db connection serves every line of worker output —
    // opening per line re-ran the whole bootstrap each time. `None` degrades
    // to a lazy open inside worker_db_call on first use.
    let mut control_db: Option<ControlDb> = ControlDb::open_current()
        .map_err(|error| {
            eprintln!("podcast worker {task_id}: control.db open failed at start: {error}");
            error
        })
        .ok();
    let (sender, receiver) = mpsc::channel();
    let mut child = match child_handle.lock() {
        Ok(mut slot) => match slot.take() {
            Some(child) => child,
            None => {
                release_worker_entry(&task_id);
                return;
            }
        },
        Err(_) => {
            release_worker_entry(&task_id);
            return;
        }
    };
    if let Some(stdout) = child.stdout.take() {
        read_stream(stdout, "stdout", sender.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        read_stream(stderr, "stderr", sender.clone());
    }
    // Drop the local sender so the channel closes when both stream readers finish.
    drop(sender);

    // Consume stdout/stderr live while the worker runs — never wait for exit first.
    let mut stderr_tail = None;
    let mut fatal_error = None;
    let mut exit_status: Option<Result<ExitStatus, std::io::Error>> = None;
    // P1-2: a grandchild (e.g. ffmpeg spawned by python) inherits the pipe
    // write end, so reader threads may never see EOF and the channel may never
    // disconnect. Once the worker's exit is observed, drain queued lines for a
    // bounded grace only — dropping the Job Object below then kills the
    // lingering grandchildren (child processes of a job member join the job
    // automatically because no BREAKAWAY flag is allowed on it).
    let mut drain_deadline: Option<Instant> = None;
    loop {
        match receiver.recv_timeout(Duration::from_millis(150)) {
            Ok((stream, line)) => {
                apply_worker_line(
                    &task_id,
                    &mut control_db,
                    &app,
                    &stream,
                    &line,
                    &mut stderr_tail,
                    &mut fatal_error,
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if exit_status.is_none() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            exit_status = Some(Ok(status));
                            drain_deadline = Some(Instant::now() + POST_EXIT_DRAIN_GRACE);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            exit_status = Some(Err(error));
                            break;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if exit_status.is_none() {
                    exit_status = Some(child.wait());
                }
                break;
            }
        }
        // Once the process has exited, keep draining until readers disconnect —
        // but never longer than the post-exit grace window.
        if exit_status.is_some() {
            while let Ok((stream, line)) = receiver.try_recv() {
                apply_worker_line(
                    &task_id,
                    &mut control_db,
                    &app,
                    &stream,
                    &line,
                    &mut stderr_tail,
                    &mut fatal_error,
                );
            }
            if drain_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
        }
    }
    let status = match exit_status {
        Some(value) => value,
        None => child.wait(),
    };
    // P1-28: the structured fatal NDJSON line is the real failure reason; only
    // fall back to the stderr tail when no fatal record was seen.
    let resolved_error = fatal_error.or(stderr_tail);
    let (success, status_message) = match status {
        Ok(value) if value.success() => {
            // Must honor settings.libraryRoot — bare StorageLocations::current() points at the
            // default Documents library and leaves books invisible to open_task_result.
            match StorageLocations::current_with_library_settings().and_then(|locations| {
                let mut control = ControlDb::open_current()?;
                let transaction =
                    super::publish_task_result_at(&mut control, &locations, &task_id)?;
                // Refuse "success" unless the shelf book is actually findable.
                crate::library::open_book(&locations.library_root, &transaction.book_id)
                    .map_err(|error| {
                        format!("PUBLISH_FAILED: published book is not readable ({error})")
                    })?;
                Ok(())
            }) {
                Ok(()) => (true, resolved_error),
                Err(error) => (
                    false,
                    Some(
                        serde_json::json!({
                            "type": "fatal",
                            "errorCode": "PUBLISH_FAILED",
                            "message": error,
                        })
                        .to_string(),
                    ),
                ),
            }
        }
        Ok(value) => (value.success(), resolved_error),
        Err(error) => (false, Some(error.to_string())),
    };
    finish_task_logged(
        &mut control_db,
        &task_id,
        &app,
        success,
        status_message.as_deref(),
    );
    drop(job);
    release_worker_entry(&task_id);
    // P1-5: the concurrency slot is free — promote the next queued podcast
    // task so a batch drains one worker at a time.
    start_next_queued_worker(&app);
}

fn release_worker_entry(task_id: &str) {
    if let Ok(mut active) = workers().lock() {
        active.remove(task_id);
    }
}

/// P1-1/P1-15: task ids that currently have a live in-process worker —
/// startup recovery (`recover_interrupted_tasks`, called from the lib.rs
/// setup sweep) uses this set to tell a dead-Running row apart from a task
/// a live worker is still driving. No frontend command exposes it.
pub fn active_podcast_task_ids() -> Result<Vec<String>, String> {
    let active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    Ok(active.keys().cloned().collect())
}

/// P1-5: with the concurrency slot free, promote the oldest Queued podcast
/// task. Composed over the existing `task_snapshots` control-db API (a
/// dedicated `next_queued_podcast_task` accessor could replace it later —
/// flagged for the control.rs owner).
fn start_next_queued_worker(app: &AppHandle) {
    let mut tried: HashSet<String> = HashSet::new();
    loop {
        if tried.len() >= MAX_QUEUE_DRAIN_ATTEMPTS {
            break;
        }
        let next = ControlDb::open_current()
            .ok()
            .and_then(|control| control.task_snapshots(Some(TaskKind::Podcast)).ok())
            .and_then(|snapshots| {
                snapshots
                    .into_iter()
                    .filter(|snapshot| {
                        snapshot.lifecycle_state == LifecycleState::Queued
                            && !tried.contains(&snapshot.id)
                    })
                    .min_by(|a, b| a.created_at.cmp(&b.created_at))
                    .map(|snapshot| snapshot.id)
            });
        let Some(task_id) = next else {
            break;
        };
        tried.insert(task_id.clone());
        match start_task(task_id, app.clone()) {
            // Either the slot is now consumed, or a concurrent start won it and
            // this task stayed Queued — its runner will pull it up later.
            Ok(()) => break,
            // Start failed (task terminated or spawn error recorded) — try the
            // next queued task instead of stalling the whole queue.
            Err(_) => continue,
        }
    }
}

pub fn start_task(task_id: String, app: AppHandle) -> Result<(), String> {
    crate::cache::validate_task_id(&task_id)?;
    // P1-5: hold the workers lock across admission + spawn so the single-slot
    // cap is atomic — two concurrent starts can never both see a free slot.
    let mut active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    if active.contains_key(&task_id) {
        return Err("WORKER_ALREADY_RUNNING".to_string());
    }
    if active.len() >= MAX_CONCURRENT_WORKERS {
        // Leave the task Queued; run_worker pulls the next queued task when
        // this slot frees, so the batch drains one worker at a time.
        return Ok(());
    }
    let locations = StorageLocations::current()?;
    let settings = crate::settings::load_settings()?;
    let mut command = podcast_worker_command(&locations, &settings, &task_id)?;
    persist_starting(&app, &task_id)?;
    #[allow(unused_mut)] // `mut` only used by the Windows handle-dup failure path.
    let (mut child, job) = match spawn_worker(&mut command) {
        Ok(value) => value,
        Err(error) => {
            // P2-11: persist the terminal failure with retry + logging — a
            // lost event here strands the task in Starting forever.
            let mut db = None;
            finish_task_logged(&mut db, &task_id, &app, false, Some(&error));
            return Err(error);
        }
    };
    let pid = child.id();
    #[cfg(windows)]
    let process = match duplicate_process_handle(&child) {
        Ok(handle) => handle,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let mut db = None;
            finish_task_logged(&mut db, &task_id, &app, false, Some(&error));
            return Err(error);
        }
    };
    let child_handle = Arc::new(Mutex::new(Some(child)));
    active.insert(
        task_id.clone(),
        WorkerEntry {
            #[cfg(windows)]
            process,
            pid,
            suspended: false,
        },
    );
    drop(active);
    thread::spawn({
        let task_id = task_id.clone();
        let app = app.clone();
        move || run_worker(task_id, app, child_handle, job)
    });
    Ok(())
}

pub fn stop_all() -> Result<(), String> {
    let active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    for entry in active.values() {
        let _ = terminate_task(entry);
    }
    Ok(())
}

fn terminate_task(entry: &WorkerEntry) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = entry.pid;
        // The pinned process handle identifies our worker even if its PID was
        // somehow recycled — never a bare OpenProcess(pid) (P1-3).
        // SAFETY: `entry.process` is an owned clone of the live worker handle.
        unsafe { crate::job_object::terminate_process(entry.process.as_raw_handle() as HANDLE) }
    }
    #[cfg(not(windows))]
    {
        // P3-26: `run_worker` takes the Child out of the shared slot at
        // startup, so the old `slot.as_mut().kill()` always hit `None` and
        // reported WORKER_NOT_RUNNING for a live worker. Only the pinned pid
        // remains — signal it through kill(1). The pid cannot be recycled
        // while this entry is registered: `run_worker` only releases the
        // entry after wait() reaps the child, so the (possibly zombie)
        // process still owns it until then.
        let status = Command::new("kill")
            .arg("-KILL")
            .arg(entry.pid.to_string())
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "WORKER_KILL_FAILED: kill -KILL {} exited with {status}",
                entry.pid
            ))
        }
    }
}

pub fn pause_task(task_id: &str) -> Result<(), String> {
    let mut active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    let entry = active
        .get_mut(task_id)
        .ok_or_else(|| "WORKER_NOT_RUNNING".to_string())?;
    // P1-4: suspend must be idempotent. Two guards:
    //  1. `entry.suspended` — the pairing flag; survives the DB being flipped
    //     Paused->Running when buffered worker lines are replayed through
    //     record_worker_line.
    //  2. DB lifecycle — if the host already recorded Paused, never stack a
    //     second suspend count on the process.
    let lifecycle = worker_lifecycle_state(task_id);
    if entry.suspended
        || lifecycle.as_deref().is_some_and(is_paused_state)
        || lifecycle.as_deref().is_some_and(is_terminal_state)
    {
        return Ok(());
    }
    #[cfg(windows)]
    {
        // P3-27: tree suspend — a bare suspend_process only froze python.exe
        // while ffmpeg grandchildren kept running under a "Paused" UI.
        crate::job_object::suspend_process_tree(
            entry.process.as_raw_handle() as HANDLE,
            entry.pid,
        )?;
        entry.suspended = true;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        Err("WORKER_PAUSE_UNSUPPORTED".to_string())
    }
}

pub fn cancel_task(task_id: &str) -> Result<(), String> {
    let active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    let entry = active
        .get(task_id)
        .ok_or_else(|| "WORKER_NOT_RUNNING".to_string())?;
    terminate_task(entry)
}

pub fn resume_task(task_id: &str) -> Result<(), String> {
    let mut active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    let entry = active
        .get_mut(task_id)
        .ok_or_else(|| "WORKER_NOT_RUNNING".to_string())?;
    // P1-4: `entry.suspended` is the pairing truth — only resume when WE still
    // hold a live suspend, so ResumeThread can never underflow a stranger's
    // count. When the flag is set but the DB was flipped back to Running by
    // buffered worker lines, resuming is still the repair: leaving the process
    // frozen while the UI shows Running is exactly the deadlock being fixed.
    let lifecycle = worker_lifecycle_state(task_id);
    if lifecycle.as_deref().is_some_and(is_terminal_state) || !entry.suspended {
        return Ok(());
    }
    #[cfg(windows)]
    {
        crate::job_object::resume_process_tree(
            entry.process.as_raw_handle() as HANDLE,
            entry.pid,
        )?;
        entry.suspended = false;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        Err("WORKER_RESUME_UNSUPPORTED".to_string())
    }
}

fn parse_worker_json(line: &str) -> Option<Value> {
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests {
    use super::{parse_worker_json, read_stream, worker_fatal_line};
    use std::io::Cursor;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn read_stream_survives_non_utf8_lines_and_keeps_line_framing() {
        // cp936 bytes for 「中文」 followed by a valid UTF-8 NDJSON line and a
        // final line without a trailing newline.
        let bytes = b"\xd6\xd0\xce\xc4 log\r\n{\"type\":\"progress\",\"percent\":1}\ntail"
            .to_vec();
        let (sender, receiver) = mpsc::channel();
        read_stream(Cursor::new(bytes), "stdout", sender);

        let first = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("first line must arrive");
        let second = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("second line must arrive");
        let third = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("tail line must arrive");
        assert_eq!(first.0, "stdout");
        assert!(first.1.contains('\u{fffd}'));
        assert!(first.1.ends_with("log"));
        assert_eq!(second.1, "{\"type\":\"progress\",\"percent\":1}");
        assert_eq!(third.1, "tail");
        assert!(receiver.recv_timeout(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn worker_json_lines_are_optional_and_safe() {
        assert_eq!(
            parse_worker_json(r#"{"type":"progress","percent":42}"#)
                .and_then(|value| value.get("percent").and_then(|value| value.as_u64())),
            Some(42)
        );
        assert!(parse_worker_json("plain worker log").is_none());
    }

    #[test]
    fn fatal_ndjson_line_is_detected_and_ignored_when_absent() {
        // P1-28: only a structured type=fatal record beats the stderr tail.
        assert!(worker_fatal_line("ctranslate2 warning noise").is_none());
        assert!(worker_fatal_line(r#"{"type":"progress","percent":1}"#).is_none());
        assert!(worker_fatal_line(r#"{"type":"completed"}"#).is_none());
        let fatal = r#"{"type":"fatal","errorCode":"MODEL_INCOMPATIBLE","message":"模型不兼容"}"#;
        assert_eq!(worker_fatal_line(fatal).as_deref(), Some(fatal));
        // A fatal record on stderr counts too — the tail-line fallback would
        // otherwise miss a fatal followed by more stderr noise.
        assert!(worker_fatal_line("{not json").is_none());
    }
}
