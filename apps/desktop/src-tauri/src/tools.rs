use crate::settings::AppSettings;
use serde::Serialize;
#[cfg(not(windows))]
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

mod launcher;
#[cfg(windows)]
mod ready;
#[cfg(windows)]
mod tool_manager;

#[cfg(windows)]
use crate::job_object::JobObject;
use launcher::{command_for, require_runtime, tool_paths};
#[cfg(windows)]
use ready::wait_for_ready;
#[cfg(windows)]
use tool_manager::{EngineHealth, ManagedProcess, ProcessDescriptor, ToolManager};

#[cfg(windows)]
static TOOL_MANAGER: OnceLock<Mutex<ToolManager>> = OnceLock::new();
#[cfg(not(windows))]
static LAUNCHED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLaunch {
    pub tool: String,
    pub message: String,
    pub url: Option<String>,
}

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

fn tool_url(kind: ToolKind, port: Option<u16>) -> Option<String> {
    port.map(|port| match kind {
        ToolKind::Zhihu | ToolKind::Podcast => format!("http://127.0.0.1:{port}"),
    })
}

fn action_for(tool: &str) -> Result<ToolKind, String> {
    ToolKind::parse(tool)
}

pub fn status(tool: &str) -> Result<ToolStatus, String> {
    let kind = action_for(tool)?;
    let paths = tool_paths(&crate::settings::runtime_root()?, kind);
    let ready = require_runtime(&paths).is_ok();
    #[cfg(windows)]
    let running = TOOL_MANAGER
        .get_or_init(|| Mutex::new(ToolManager::default()))
        .lock()
        .map_err(|_| "Tool process state is unavailable".to_string())?
        .is_running(kind.key())?;
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

pub fn launch(tool: &str, settings: &AppSettings) -> Result<ToolLaunch, String> {
    let kind = action_for(tool)?;
    let runtime_root = crate::settings::runtime_root()?;
    let key = kind.key();
    let token = uuid::Uuid::new_v4().simple().to_string();
    #[cfg(windows)]
    let mut manager = TOOL_MANAGER
        .get_or_init(|| Mutex::new(ToolManager::default()))
        .lock()
        .map_err(|_| "Tool process state is unavailable".to_string())?;
    #[cfg(windows)]
    if let Some(snapshot) = manager.refresh(key)? {
        if snapshot.exit_status.is_none() {
            return Ok(ToolLaunch {
                tool: key.to_string(),
                message: "工具已在本次应用会话中启动。".to_string(),
                url: tool_url(kind, snapshot.port),
            });
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
        return Ok(ToolLaunch {
            tool: key.to_string(),
            message: "工具已在本次应用会话中启动。".to_string(),
            url: None,
        });
    }

    let mut command = command_for(&runtime_root, settings, kind, &token)?;
    #[cfg(windows)]
    let ready = {
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
        let descriptor = ProcessDescriptor {
            engine: key.to_string(),
            port: Some(ready.port),
            protocol_version: Some(ready.protocol_version),
            token,
            started_at: chrono::Utc::now().to_rfc3339(),
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
    Ok(ToolLaunch {
        tool: key.to_string(),
        message: match kind {
            ToolKind::Zhihu => "知乎归档控制台正在启动。",
            ToolKind::Podcast => "播客转写窗口正在启动。",
        }
        .to_string(),
        #[cfg(windows)]
        url: tool_url(kind, Some(ready.port)),
        #[cfg(not(windows))]
        url: None,
    })
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
