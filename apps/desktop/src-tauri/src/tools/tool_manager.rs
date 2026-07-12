use crate::job_object::JobObject;
use serde::Serialize;
use std::collections::HashMap;
use std::process::{Child, ExitStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EngineHealth {
    Ready,
    Exited,
}

pub(super) struct ProcessDescriptor {
    pub engine: String,
    pub port: Option<u16>,
    pub protocol_version: Option<u32>,
    pub token: String,
    pub started_at: String,
    pub health: EngineHealth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProcessExitStatus {
    pub code: Option<i32>,
    pub success: bool,
}

impl From<ExitStatus> for ProcessExitStatus {
    fn from(value: ExitStatus) -> Self {
        Self {
            code: value.code(),
            success: value.success(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProcessSnapshot {
    pub engine: String,
    pub pid: u32,
    pub port: Option<u16>,
    pub protocol_version: Option<u32>,
    pub started_at: String,
    pub health: EngineHealth,
    pub exit_status: Option<ProcessExitStatus>,
}

pub(super) struct ManagedProcess {
    child: Child,
    _job: JobObject,
    descriptor: ProcessDescriptor,
    exit_status: Option<ProcessExitStatus>,
}

impl ManagedProcess {
    pub(super) fn new(child: Child, job: JobObject, descriptor: ProcessDescriptor) -> Self {
        Self {
            child,
            _job: job,
            descriptor,
            exit_status: None,
        }
    }

    fn snapshot(&self) -> ProcessSnapshot {
        ProcessSnapshot {
            engine: self.descriptor.engine.clone(),
            pid: self.child.id(),
            port: self.descriptor.port,
            protocol_version: self.descriptor.protocol_version,
            started_at: self.descriptor.started_at.clone(),
            health: self.descriptor.health,
            exit_status: self.exit_status,
        }
    }

    fn refresh(&mut self) -> Result<(), String> {
        if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
            self.descriptor.health = EngineHealth::Exited;
            self.exit_status = Some(status.into());
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct ToolManager {
    processes: HashMap<String, ManagedProcess>,
}

impl ToolManager {
    pub(super) fn clear(&mut self) {
        self.processes.clear();
    }

    pub(super) fn insert(&mut self, process: ManagedProcess) -> Result<(), String> {
        if process.descriptor.token.is_empty() {
            return Err("ENGINE_TOKEN_REQUIRED".to_string());
        }
        if let Some(existing) = self.processes.get_mut(&process.descriptor.engine) {
            existing.refresh()?;
            if existing.exit_status.is_none() {
                return Err("ENGINE_ALREADY_RUNNING".to_string());
            }
        }
        self.processes
            .insert(process.descriptor.engine.clone(), process);
        Ok(())
    }

    pub(super) fn refresh(&mut self, engine: &str) -> Result<Option<ProcessSnapshot>, String> {
        let Some(process) = self.processes.get_mut(engine) else {
            return Ok(None);
        };
        process.refresh()?;
        Ok(Some(process.snapshot()))
    }

    pub(super) fn token(&self, engine: &str) -> Option<&str> {
        self.processes
            .get(engine)
            .map(|process| process.descriptor.token.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{EngineHealth, ManagedProcess, ProcessDescriptor, ToolManager};
    use crate::job_object::JobObject;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    #[test]
    fn tracks_owned_process_and_refreshes_exit_status() {
        let child = Command::new("cmd.exe")
            .args(["/C", "exit /B 7"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("test child must start");
        let pid = child.id();
        let job = JobObject::kill_on_close().expect("job object must be created");
        job.assign(&child).expect("child must join job");
        let descriptor = ProcessDescriptor {
            engine: "podcast".to_string(),
            port: Some(43_210),
            protocol_version: Some(1),
            token: "memory-only-secret".to_string(),
            started_at: "2026-07-12T06:30:00Z".to_string(),
            health: EngineHealth::Ready,
        };
        let process = ManagedProcess::new(child, job, descriptor);
        let mut manager = ToolManager::default();

        manager.insert(process).expect("process must be registered");
        let initial = manager
            .refresh("podcast")
            .expect("process status must refresh")
            .expect("process must exist");
        let deadline = Instant::now() + Duration::from_secs(5);
        let exited = loop {
            let snapshot = manager
                .refresh("podcast")
                .expect("process status must refresh")
                .expect("process must remain registered");
            if snapshot.exit_status.is_some() {
                break snapshot;
            }
            assert!(Instant::now() < deadline, "test child did not exit");
            std::thread::yield_now();
        };

        assert_eq!(initial.pid, pid);
        assert_eq!(initial.port, Some(43_210));
        assert_eq!(initial.protocol_version, Some(1));
        assert_eq!(exited.health, EngineHealth::Exited);
        assert_eq!(
            exited.exit_status.expect("exit status must exist").code,
            Some(7)
        );
        assert_eq!(
            manager
                .processes
                .get("podcast")
                .expect("process must exist")
                .descriptor
                .token,
            "memory-only-secret"
        );
        assert!(manager
            .refresh("podcast")
            .expect("running state must load")
            .expect("process must remain registered")
            .exit_status
            .is_some());
        let serialized = serde_json::to_string(&exited).expect("snapshot must serialize");
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("memory-only-secret"));

        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Mutex<ToolManager>>();
    }
}
