use crate::settings::AppSettings;
use serde::de::DeserializeOwned;
use serde::Serialize;
#[cfg(not(windows))]
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
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
use tool_manager::{EngineHealth, ManagedProcess, ProcessDescriptor, ToolManager};

#[cfg(windows)]
static TOOL_MANAGER: OnceLock<Mutex<ToolManager>> = OnceLock::new();
#[cfg(windows)]
static ENGINE_RECOVERY_DONE: OnceLock<()> = OnceLock::new();
#[cfg(not(windows))]
static LAUNCHED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub tool: String,
    pub state: String,
    pub version: String,
    pub message: String,
}

#[derive(Clone, Copy)]
enum ToolKind {
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
    let mut manager = TOOL_MANAGER
        .get_or_init(|| Mutex::new(ToolManager::default()))
        .lock()
        .map_err(|_| "Tool process state is unavailable".to_string())?;
    manager.clear();
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
    recover_stale_engine_instances()?;
    #[cfg(windows)]
    let running = {
        let mut manager = TOOL_MANAGER
            .get_or_init(|| Mutex::new(ToolManager::default()))
            .lock()
            .map_err(|_| "Tool process state is unavailable".to_string())?;
        let snapshot = manager.refresh(kind.key())?;
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

fn launch(tool: &str, settings: &AppSettings) -> Result<(), String> {
    let kind = action_for(tool)?;
    let runtime_root = crate::settings::runtime_root()?;
    let key = kind.key();
    let token = uuid::Uuid::new_v4().simple().to_string();
    #[cfg(windows)]
    recover_stale_engine_instances()?;
    #[cfg(windows)]
    let mut manager = TOOL_MANAGER
        .get_or_init(|| Mutex::new(ToolManager::default()))
        .lock()
        .map_err(|_| "Tool process state is unavailable".to_string())?;
    #[cfg(windows)]
    if let Some(snapshot) = manager.refresh(key)? {
        if snapshot.exit_status.is_none() {
            return Ok(());
        }
    }
    #[cfg(not(windows))]
    let launches = LAUNCHED.get_or_init(|| Mutex::new(HashSet::new()));
    #[cfg(not(windows))]
    let mut guard = launches
        .lock()
        .map_err(|_| "Tool launch state is unavailable".to_string())?;
    #[cfg(not(windows))]
    if guard.contains(key) {
        return Ok(());
    }

    let mut command = command_for(&runtime_root, settings, kind, &token)?;
    #[cfg(windows)]
    let _ready = {
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
            token: token.clone(),
            started_at,
            health: EngineHealth::Ready,
        };
        manager.insert(ManagedProcess::new(child, job, descriptor))?;
        ready
    };
    #[cfg(not(windows))]
    {
        command.spawn().map_err(|error| error.to_string())?;
        guard.insert(key.to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn zhihu_client(settings: &AppSettings) -> Result<SidecarHttpClient, String> {
    launch("zhihu", settings)?;
    let mut manager = TOOL_MANAGER
        .get_or_init(|| Mutex::new(ToolManager::default()))
        .lock()
        .map_err(|_| "Tool process state is unavailable".to_string())?;
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
    SidecarHttpClient::new(&format!("http://127.0.0.1:{port}"), &token)
}

/// Ensure the Zhihu sidecar is launched and responsive (best-effort for snapshot reconcile).
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
