use crate::control::ControlDb;
use crate::settings::AppSettings;
use crate::storage::StorageLocations;
use crate::tasks::TaskEvent;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
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
        .env("IMMERSIVE_PODCAST_DATA_ROOT", data_root)
        .env("IMMERSIVE_PODCAST_CACHE_ROOT", cache_root)
        .env("IMMERSIVE_LIBRARY_ROOT", &settings.library_root)
        .env(
            "IMMERSIVE_PODCAST_MODEL_ROOT",
            locations.runtime_root.join("podcast/models"),
        )
        .env(
            "IMMERSIVE_PODCAST_PYTHON",
            &locations.runtime_root.join("podcast/python/python.exe"),
        )
        .env(
            "PATH",
            std::env::join_paths(path_parts).map_err(|error| error.to_string())?,
        );
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
        for line in BufReader::new(stream).lines() {
            match line {
                Ok(value) => {
                    let _ = sender.send((name.to_string(), value));
                }
                Err(error) => {
                    let _ = sender.send((name.to_string(), format!("stream read failed: {error}")));
                    break;
                }
            }
        }
    });
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
    let status = child.wait();
    drop(sender);
    let mut last_error = None;
    for (stream, line) in receiver {
        if stream == "stderr" && !line.trim().is_empty() {
            last_error = Some(line.clone());
        }
        if let Ok(mut control) = ControlDb::open_current() {
            if let Ok(Some(event)) = control.record_worker_line(&task_id, &stream, &line) {
                emit_task(&app, &event);
            }
        }
    }
    let (success, status_message) = match status {
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
        return crate::job_object::terminate_process(pid);
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
    use super::parse_worker_json;

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
