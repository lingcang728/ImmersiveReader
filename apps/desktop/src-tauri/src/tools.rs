use crate::settings::AppSettings;
use serde::de::DeserializeOwned;
use serde::Serialize;
#[cfg(not(windows))]
use std::collections::HashSet;
#[cfg(windows)]
use std::sync::Condvar;
use std::sync::{Mutex, OnceLock};
#[cfg(windows)]
use std::sync::MutexGuard;
#[cfg(windows)]
use std::time::Duration;

mod launcher;
#[cfg(windows)]
mod ready;
#[cfg(windows)]
mod sidecar_http;
#[cfg(windows)]
mod tool_manager;

#[cfg(windows)]
use crate::job_object::JobObject;
use launcher::{command_for, require_runtime, tool_paths};
#[cfg(windows)]
use ready::wait_for_ready;
#[cfg(windows)]
use sidecar_http::SidecarHttpClient;
#[cfg(windows)]
use tool_manager::{EngineHealth, LaunchClaim, ManagedProcess, ProcessDescriptor, ToolManager};

#[cfg(windows)]
static TOOL_MANAGER: OnceLock<Mutex<ToolManager>> = OnceLock::new();
/// Fires when a claimed launch slot settles — either a `ManagedProcess` was
/// published or the attempt failed and released the slot. Threads that find an
/// engine already launching park on this condvar with `TOOL_MANAGER` released
/// instead of pinning the mutex for the whole spawn+health window.
#[cfg(windows)]
static LAUNCH_UPDATED: Condvar = Condvar::new();
#[cfg(windows)]
static ENGINE_RECOVERY_DONE: OnceLock<()> = OnceLock::new();
#[cfg(not(windows))]
static LAUNCHED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Locks the shared tool manager for a short in-memory read or write.
/// Process spawn, READY handshakes, HTTP health checks, and control.db I/O
/// must all run with this guard released.
#[cfg(windows)]
fn tool_manager() -> Result<MutexGuard<'static, ToolManager>, String> {
    TOOL_MANAGER
        .get_or_init(|| Mutex::new(ToolManager::default()))
        .lock()
        .map_err(|_| "Tool process state is unavailable".to_string())
}

/// Releases a `ToolManager::begin_launch` claim and wakes waiting launchers.
/// Dropped only after `launch_claimed_engine` has published (or failed and
/// torn down) its process, so waiters always observe a settled state.
#[cfg(windows)]
struct EngineLaunchGuard {
    engine: &'static str,
}

#[cfg(windows)]
impl EngineLaunchGuard {
    fn new(engine: &'static str) -> Self {
        Self { engine }
    }
}

#[cfg(windows)]
impl Drop for EngineLaunchGuard {
    fn drop(&mut self) {
        if let Ok(mut manager) = tool_manager() {
            manager.finish_launch(self.engine);
        }
        LAUNCH_UPDATED.notify_all();
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub tool: String,
    pub state: String,
    pub version: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolKind {
    Zhihu,
    Podcast,
}

impl ToolKind {
    fn parse(tool: &str) -> Result<Self, String> {
        match tool {
            "zhihu" => Ok(Self::Zhihu),
            "podcast" => Ok(Self::Podcast),
            _ => Err("Only the configured Zhihu and Podcast tools may be launched".to_string()),
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Zhihu => "zhihu",
            Self::Podcast => "podcast",
        }
    }
}

fn action_for(tool: &str) -> Result<ToolKind, String> {
    ToolKind::parse(tool)
}

#[cfg(windows)]
pub(crate) fn recover_stale_engine_instances() -> Result<(), String> {
    if ENGINE_RECOVERY_DONE.get().is_some() {
        return Ok(());
    }
    let mut control = crate::control::ControlDb::open_current()?;
    control.recover_stale_engine_instances()?;
    let _ = ENGINE_RECOVERY_DONE.set(());
    Ok(())
}

#[cfg(windows)]
pub(crate) fn stop_all() -> Result<(), String> {
    tool_manager()?.clear();
    crate::podcast::stop_workers()?;
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn stop_all() -> Result<(), String> {
    if let Some(launches) = LAUNCHED.get() {
        launches
            .lock()
            .map_err(|_| "Tool launch state is unavailable".to_string())?
            .clear();
    }
    crate::podcast::stop_workers()?;
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn recover_stale_engine_instances() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn persist_engine_exit(
    kind: ToolKind,
    snapshot: &tool_manager::ProcessSnapshot,
) -> Result<(), String> {
    let Some(exit_status) = snapshot.exit_status else {
        return Ok(());
    };
    let mut control = crate::control::ControlDb::open_current()?;
    control.mark_engine_crashed(kind.key(), snapshot.pid, exit_status.code)?;
    Ok(())
}

pub fn status(tool: &str) -> Result<ToolStatus, String> {
    let kind = action_for(tool)?;
    let paths = tool_paths(&crate::settings::runtime_root()?, kind);
    let ready = require_runtime(&paths).is_ok();
    #[cfg(windows)]
    let running = {
        recover_stale_engine_instances()?;
        // Short lock: refresh only runs try_wait on the managed child.
        let snapshot = {
            let mut manager = tool_manager()?;
            manager.refresh(kind.key())?
        };
        // The control.db write runs with TOOL_MANAGER released.
        if let Some(snapshot) = &snapshot {
            persist_engine_exit(kind, snapshot)?;
        }
        snapshot.is_some_and(|snapshot| snapshot.exit_status.is_none())
    };
    #[cfg(not(windows))]
    let running = LAUNCHED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| "Tool launch state is unavailable".to_string())?
        .contains(tool);
    Ok(ToolStatus {
        tool: tool.to_string(),
        state: if running {
            "running"
        } else if ready {
            "ready"
        } else {
            "error"
        }
        .to_string(),
        version: "1.0.0".to_string(),
        message: if ready {
            "受管运行时已就绪。"
        } else {
            "受管运行时缺失，请重新准备运行时。"
        }
        .to_string(),
    })
}

/// Spawns the sidecar, waits for the READY handshake, verifies HTTP health,
/// records the instance in control.db, and finally publishes the
/// `ManagedProcess` — re-acquiring `TOOL_MANAGER` only for that last insert.
/// Callers must already hold the `begin_launch` claim for `kind`.
#[cfg(windows)]
fn launch_claimed_engine(
    kind: ToolKind,
    runtime_root: &std::path::Path,
    settings: &AppSettings,
    token: String,
) -> Result<(), String> {
    let key = kind.key();
    let mut command = command_for(runtime_root, settings, kind, &token)?;
    let (mut child, job) = JobObject::spawn_suspended(&mut command)?;
    let ready = match wait_for_ready(&mut child, key, Duration::from_secs(15)) {
        Ok((ready, _reader)) => ready,
        Err(error) => {
            drop(job);
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let origin = format!("http://127.0.0.1:{}", ready.port);
    let health = SidecarHttpClient::new(&origin, &token).and_then(|client| {
        tauri::async_runtime::block_on(async {
            client.health().await?;
            let _: serde_json::Value = client.get_json("/api/status").await?;
            Ok::<(), String>(())
        })
    });
    if let Err(error) = health {
        drop(job);
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let started_at = chrono::Utc::now().to_rfc3339();
    let control = match crate::control::ControlDb::open_current() {
        Ok(control) => control,
        Err(error) => {
            drop(job);
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if let Err(error) = control.record_engine_instance(
        key,
        child.id(),
        Some(ready.port),
        Some(ready.protocol_version),
        &started_at,
    ) {
        drop(job);
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let descriptor = ProcessDescriptor {
        engine: key.to_string(),
        port: Some(ready.port),
        protocol_version: Some(ready.protocol_version),
        token,
        started_at,
        health: EngineHealth::Ready,
    };
    // Re-acquire the manager only to publish the process. If the insert fails
    // the ManagedProcess is dropped here, which closes the kill-on-close Job
    // Object and terminates the spawned child.
    tool_manager()?.insert(ManagedProcess::new(child, job, descriptor))
}

fn launch(tool: &str, settings: &AppSettings) -> Result<(), String> {
    let kind = action_for(tool)?;
    let runtime_root = crate::settings::runtime_root()?;
    let key = kind.key();
    let token = uuid::Uuid::new_v4().simple().to_string();
    #[cfg(windows)]
    {
        recover_stale_engine_instances()?;
        // Phase 1 (short lock): dedupe against in-flight launches and claim the
        // launch slot. When another thread is mid-launch the caller parks on
        // LAUNCH_UPDATED — the mutex is released while parked — then re-checks;
        // if that attempt failed this caller becomes the next launcher, matching
        // the old serialized-mutex semantics.
        let mut manager = tool_manager()?;
        loop {
            match manager.begin_launch(key)? {
                LaunchClaim::Acquired => break,
                LaunchClaim::AlreadyRunning => return Ok(()),
                LaunchClaim::AlreadyStarting => {
                    let (guard, _timeout) = LAUNCH_UPDATED
                        .wait_timeout(manager, Duration::from_secs(60))
                        .map_err(|_| "Tool process state is unavailable".to_string())?;
                    manager = guard;
                }
            }
        }
        drop(manager);
        // Phase 2 (unlocked): spawn + READY + HTTP health + engine_instances
        // write all run with TOOL_MANAGER released. The guard releases the
        // claim and notifies waiters on every exit path, including `?`.
        let _launch_guard = EngineLaunchGuard::new(key);
        launch_claimed_engine(kind, &runtime_root, settings, token)?;
    }
    #[cfg(not(windows))]
    {
        let launches = LAUNCHED.get_or_init(|| Mutex::new(HashSet::new()));
        let mut guard = launches
            .lock()
            .map_err(|_| "Tool launch state is unavailable".to_string())?;
        if !guard.contains(key) {
            let mut command = command_for(&runtime_root, settings, kind, &token)?;
            command.spawn().map_err(|error| error.to_string())?;
            guard.insert(key.to_string());
        }
    }
    Ok(())
}

/// Requests that `kind` be spawned, health-checked, and registered on a
/// dedicated background thread. Non-blocking: `TOOL_MANAGER` is held only for
/// the dedupe/claim check; if the engine already has a live managed process or
/// a launch is already in flight the request coalesces into a no-op. No
/// `block_on` runs on the caller's thread.
pub fn request_engine_warmup(kind: ToolKind) {
    #[cfg(windows)]
    {
        let key = kind.key();
        let claimed = tool_manager()
            .and_then(|mut manager| manager.begin_launch(key))
            .map(|claim| matches!(claim, LaunchClaim::Acquired))
            .unwrap_or(false);
        if !claimed {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name(format!("{key}-engine-warmup"))
            .spawn(move || {
                let _launch_guard = EngineLaunchGuard::new(key);
                let result = recover_stale_engine_instances()
                    .and_then(|()| crate::settings::load_settings())
                    .and_then(|settings| {
                        let runtime_root = crate::settings::runtime_root()?;
                        let token = uuid::Uuid::new_v4().simple().to_string();
                        launch_claimed_engine(kind, &runtime_root, &settings, token)
                    });
                if let Err(error) = result {
                    eprintln!("[tools] {key} engine warmup failed: {error}");
                }
            });
        if spawned.is_err() {
            // The warmup thread never started: release the claim and wake
            // waiters so a later request can retry.
            if let Ok(mut manager) = tool_manager() {
                manager.finish_launch(key);
            }
            LAUNCH_UPDATED.notify_all();
        }
    }
    #[cfg(not(windows))]
    let _ = kind;
}

#[cfg(windows)]
fn zhihu_client(settings: &AppSettings) -> Result<SidecarHttpClient, String> {
    launch("zhihu", settings)?;
    let (port, token) = {
        let mut manager = tool_manager()?;
        let snapshot = manager
            .refresh("zhihu")?
            .ok_or_else(|| "ENGINE_NOT_RUNNING".to_string())?;
        let port = snapshot
            .port
            .ok_or_else(|| "ENGINE_PORT_MISSING".to_string())?;
        let token = manager
            .token("zhihu")
            .ok_or_else(|| "ENGINE_TOKEN_MISSING".to_string())?
            .to_string();
        (port, token)
    };
    SidecarHttpClient::new(&format!("http://127.0.0.1:{port}"), &token)
}

/// Ensure the Zhihu sidecar is launched and responsive (synchronous variant;
/// snapshot-style callers should prefer [`request_engine_warmup`]).
#[allow(dead_code)]
pub(crate) fn ensure_zhihu_ready(settings: &AppSettings) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _client = zhihu_client(settings)?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = settings;
        Err("ZHIHU_ENGINE_UNSUPPORTED".to_string())
    }
}

pub(crate) fn zhihu_get_json<O: DeserializeOwned>(
    settings: &AppSettings,
    path: &str,
) -> Result<O, String> {
    #[cfg(windows)]
    {
        let client = zhihu_client(settings)?;
        tauri::async_runtime::block_on(client.get_json(path))
    }
    #[cfg(not(windows))]
    {
        let _ = (settings, path);
        Err("ZHIHU_ENGINE_UNSUPPORTED".to_string())
    }
}

pub(crate) fn zhihu_post_json<I: Serialize, O: DeserializeOwned>(
    settings: &AppSettings,
    path: &str,
    body: &I,
) -> Result<O, String> {
    #[cfg(windows)]
    {
        let client = zhihu_client(settings)?;
        tauri::async_runtime::block_on(client.post_json(path, body))
    }
    #[cfg(not(windows))]
    {
        let _ = (settings, path, body);
        Err("ZHIHU_ENGINE_UNSUPPORTED".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::action_for;

    #[test]
    fn rejects_arbitrary_commands() {
        assert!(action_for("cmd /c calc").is_err());
        assert!(action_for("zhihu").is_ok());
        assert!(action_for("podcast").is_ok());
    }
}
