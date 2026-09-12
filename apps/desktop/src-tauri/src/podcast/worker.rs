use crate::control::ControlDb;
use crate::settings::AppSettings;
use crate::storage::StorageLocations;
use crate::tasks::TaskEvent;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use crate::job_object::JobObject;

#[cfg(windows)]
type WorkerJob = JobObject;
#[cfg(not(windows))]
type WorkerJob = ();

type ChildHandle = Arc<Mutex<Option<Child>>>;
struct WorkerEntry {
    child: ChildHandle,
    pid: u32,
}

static ACTIVE_WORKERS: OnceLock<Mutex<HashMap<String, WorkerEntry>>> = OnceLock::new();

fn workers() -> &'static Mutex<HashMap<String, WorkerEntry>> {
    ACTIVE_WORKERS.get_or_init(|| Mutex::new(HashMap::new()))
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
        .env("IMMERSIVE_PODCAST_DATA_ROOT", data_root)
        .env("IMMERSIVE_PODCAST_CACHE_ROOT", cache_root)
        .env("IMMERSIVE_LIBRARY_ROOT", &settings.library_root)
        .env(
            "IMMERSIVE_PODCAST_MODEL_ROOT",
            locations.runtime_root.join("podcast/models"),
        )
        .env(
            "IMMERSIVE_PODCAST_PYTHON",
            locations.runtime_root.join("podcast/python/python.exe"),
        )
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

fn apply_worker_line(
    task_id: &str,
    app: &AppHandle,
    stream: &str,
    line: &str,
    last_error: &mut Option<String>,
) {
    if stream == "stderr" && !line.trim().is_empty() {
        *last_error = Some(line.to_string());
    }
    if let Ok(mut control) = ControlDb::open_current() {
        if let Ok(Some(event)) = control.record_worker_line(task_id, stream, line) {
            emit_task(app, &event);
        }
    }
}

fn run_worker(task_id: String, app: AppHandle, child_handle: ChildHandle, job: Option<WorkerJob>) {
    let (sender, receiver) = mpsc::channel();
    let mut child = match child_handle.lock() {
        Ok(mut slot) => match slot.take() {
            Some(child) => child,
            None => return,
        },
        Err(_) => return,
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
    let mut last_error = None;
    let mut exit_status: Option<Result<ExitStatus, std::io::Error>> = None;
    loop {
        match receiver.recv_timeout(Duration::from_millis(150)) {
            Ok((stream, line)) => {
                apply_worker_line(&task_id, &app, &stream, &line, &mut last_error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if exit_status.is_none() {
                    match child.try_wait() {
                        Ok(Some(status)) => exit_status = Some(Ok(status)),
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
        // Once the process has exited, keep draining until readers disconnect.
        if exit_status.is_some() {
            while let Ok((stream, line)) = receiver.try_recv() {
                apply_worker_line(&task_id, &app, &stream, &line, &mut last_error);
            }
        }
    }
    let status = match exit_status {
        Some(value) => value,
        None => child.wait(),
    };
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
                Ok(()) => (true, last_error),
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
        Ok(value) => (value.success(), last_error),
        Err(error) => (false, Some(error.to_string())),
    };
    if let Ok(mut control) = ControlDb::open_current() {
        if let Ok(Some(event)) =
            control.finish_worker_task(&task_id, success, status_message.as_deref())
        {
            emit_task(&app, &event);
        }
    }
    drop(job);
    if let Ok(mut active) = workers().lock() {
        active.remove(&task_id);
    }
}

pub fn start_task(task_id: String, app: AppHandle) -> Result<(), String> {
    crate::cache::validate_task_id(&task_id)?;
    {
        let active = workers()
            .lock()
            .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
        if active.contains_key(&task_id) {
            return Err("WORKER_ALREADY_RUNNING".to_string());
        }
    }
    let locations = StorageLocations::current()?;
    let settings = crate::settings::load_settings()?;
    let mut command = podcast_worker_command(&locations, &settings, &task_id)?;
    persist_starting(&app, &task_id)?;
    let (child, job) = match spawn_worker(&mut command) {
        Ok(value) => value,
        Err(error) => {
            let mut control = ControlDb::open_current()?;
            if let Ok(Some(event)) = control.finish_worker_task(&task_id, false, Some(&error)) {
                emit_task(&app, &event);
            }
            return Err(error);
        }
    };
    let pid = child.id();
    let child_handle = Arc::new(Mutex::new(Some(child)));
    {
        let mut active = workers()
            .lock()
            .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
        active.insert(
            task_id.clone(),
            WorkerEntry {
                child: Arc::clone(&child_handle),
                pid,
            },
        );
    }
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
        let _ = terminate_task(entry.pid, &entry.child);
    }
    Ok(())
}

fn terminate_task(pid: u32, _child: &ChildHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::job_object::terminate_process(pid)
    }
    #[cfg(not(windows))]
    {
        let mut slot = _child
            .lock()
            .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
        let process = slot
            .as_mut()
            .ok_or_else(|| "WORKER_NOT_RUNNING".to_string())?;
        process.kill().map_err(|error| error.to_string())
    }
}

pub fn pause_task(task_id: &str) -> Result<(), String> {
    let active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    let entry = active
        .get(task_id)
        .ok_or_else(|| "WORKER_NOT_RUNNING".to_string())?;
    #[cfg(windows)]
    {
        crate::job_object::suspend_process(entry.pid)
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
    terminate_task(entry.pid, &entry.child)
}

pub fn resume_task(task_id: &str) -> Result<(), String> {
    let active = workers()
        .lock()
        .map_err(|_| "WORKER_STATE_UNAVAILABLE".to_string())?;
    let entry = active
        .get(task_id)
        .ok_or_else(|| "WORKER_NOT_RUNNING".to_string())?;
    #[cfg(windows)]
    {
        crate::job_object::resume_process(entry.pid)
    }
    #[cfg(not(windows))]
    {
        let _ = entry;
        Err("WORKER_RESUME_UNSUPPORTED".to_string())
    }
}

#[allow(dead_code)]
fn parse_worker_json(line: &str) -> Option<Value> {
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests {
    use super::{parse_worker_json, read_stream};
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
}
