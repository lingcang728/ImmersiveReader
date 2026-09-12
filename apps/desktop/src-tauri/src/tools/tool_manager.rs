use crate::job_object::JobObject;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EngineHealth {
    Ready,
    /// The process is alive but its `/health` endpoint keeps failing — the
    /// engine is hung (suspended or deadlocked) and becomes a restart
    /// candidate once the consecutive-failure threshold is reached.
    Unresponsive,
    Exited,
}

/// Consecutive `/health` probe failures after which a live engine is treated
/// as hung and becomes eligible for a forced Job-Object restart.
pub(super) const HEALTH_FAILURE_THRESHOLD: u32 = 3;
/// The first health-triggered restart is immediate; each subsequent one must
/// wait out an exponentially growing window (`BASE * 2^attempts`, capped).
const RESTART_BACKOFF_BASE: Duration = Duration::from_secs(5);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Largest shift applied to the backoff exponent; beyond it the cap applies.
const RESTART_BACKOFF_MAX_SHIFT: u32 = 6;

/// Outcome of recording one runtime `/health` probe against a managed engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProbeOutcome {
    /// The probe succeeded; the consecutive-failure counter was reset.
    Healthy,
    /// The probe failed while the process stayed alive; carries the
    /// consecutive failure count.
    Failed(u32),
    /// Nothing to record: the engine has no live managed process, or the
    /// probed process exited / was replaced while the probe was in flight.
    Gone,
}

/// Outcome of claiming the launch slot for an engine. A held claim marks the
/// engine as "starting" so the manager mutex can be released while the caller
/// performs the slow spawn/READY/health/DB pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LaunchClaim {
    /// The caller owns this engine's launch attempt until `finish_launch`.
    Acquired,
    /// Another thread is already spawning or health-checking this engine.
    AlreadyStarting,
    /// A live managed process is already registered for this engine.
    AlreadyRunning,
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
    /// Consecutive `/health` probe failures observed while this process
    /// stayed alive. Resets on the first healthy probe.
    health_failures: u32,
}

impl ManagedProcess {
    pub(super) fn new(child: Child, job: JobObject, descriptor: ProcessDescriptor) -> Self {
        Self {
            child,
            _job: job,
            descriptor,
            exit_status: None,
            health_failures: 0,
        }
    }

    /// Terminates the child and reaps it, returning the exit status when
    /// observable. The kill-on-close Job Object is closed first so even a
    /// suspended process is terminated by the job teardown and the `wait`
    /// below cannot hang.
    pub(super) fn kill_and_reap(mut self) -> Option<ProcessExitStatus> {
        drop(self._job);
        let _ = self.child.kill();
        self.child.wait().ok().map(Into::into)
    }

    /// Records one `/health` probe outcome; returns the running count of
    /// consecutive failures.
    fn mark_health(&mut self, healthy: bool) -> u32 {
        if healthy {
            self.health_failures = 0;
            self.descriptor.health = EngineHealth::Ready;
        } else {
            self.health_failures = self.health_failures.saturating_add(1);
            self.descriptor.health = EngineHealth::Unresponsive;
        }
        self.health_failures
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

/// Per-engine debounce state for health-triggered restarts. Kept across
/// process instances so a repeatedly hung engine cannot respawn in a tight
/// loop; the window is measured from `last_restart_at`, so restarts that are
/// far apart are never delayed.
struct RestartBackoff {
    last_restart_at: Instant,
    attempts: u32,
}

#[derive(Default)]
pub(super) struct ToolManager {
    processes: HashMap<String, ManagedProcess>,
    /// Engines whose launch slot is claimed but whose process is not yet
    /// published. Entries appear in `processes` only after spawn, the READY
    /// handshake, the HTTP health check, and the engine_instances write all
    /// succeed; until then the claim itself deduplicates concurrent launchers.
    starting: HashSet<String>,
    /// Health-triggered restart bookkeeping keyed by engine. Entries persist
    /// across process replacements; a healthy probe does not clear them, so
    /// a freeze-relaunch-freeze loop keeps escalating its backoff.
    restarts: HashMap<String, RestartBackoff>,
}

impl ToolManager {
    pub(super) fn clear(&mut self) {
        self.processes.clear();
    }

    /// Claims the launch slot for `engine`, or reports why no launch is
    /// needed. An exited process never blocks a new claim: its snapshot stays
    /// queryable until the replacement publishes.
    pub(super) fn begin_launch(&mut self, engine: &str) -> Result<LaunchClaim, String> {
        if self.starting.contains(engine) {
            return Ok(LaunchClaim::AlreadyStarting);
        }
        if let Some(existing) = self.processes.get_mut(engine) {
            existing.refresh()?;
            if existing.exit_status.is_none() {
                return Ok(LaunchClaim::AlreadyRunning);
            }
        }
        self.starting.insert(engine.to_string());
        Ok(LaunchClaim::Acquired)
    }

    /// Releases a launch slot previously taken by `begin_launch`. Idempotent.
    pub(super) fn finish_launch(&mut self, engine: &str) {
        self.starting.remove(engine);
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

    /// Records one runtime `/health` probe outcome against the managed
    /// process identified by `pid`. A stale outcome — the registered process
    /// exited or was replaced while the probe was in flight — is reported as
    /// `Gone` and never counted.
    pub(super) fn record_health_probe(
        &mut self,
        engine: &str,
        pid: u32,
        healthy: bool,
    ) -> Result<ProbeOutcome, String> {
        let Some(process) = self.processes.get_mut(engine) else {
            return Ok(ProbeOutcome::Gone);
        };
        process.refresh()?;
        if process.exit_status.is_some() || process.child.id() != pid {
            return Ok(ProbeOutcome::Gone);
        }
        let failures = process.mark_health(healthy);
        Ok(if healthy {
            ProbeOutcome::Healthy
        } else {
            ProbeOutcome::Failed(failures)
        })
    }

    /// Backoff gate for a health-triggered restart. The first forced restart
    /// is permitted immediately; each subsequent one must wait out an
    /// exponentially growing window (`5s * 2^attempts`, capped at 300s)
    /// measured from the previously recorded restart. Permitted restarts are
    /// recorded here so concurrent callers share one debounce clock.
    pub(super) fn restart_permitted(&mut self, engine: &str) -> bool {
        self.restart_permitted_at(engine, Instant::now())
    }

    fn restart_permitted_at(&mut self, engine: &str, now: Instant) -> bool {
        if let Some(state) = self.restarts.get(engine) {
            let backoff = RESTART_BACKOFF_BASE
                .saturating_mul(1u32 << state.attempts.min(RESTART_BACKOFF_MAX_SHIFT))
                .min(RESTART_BACKOFF_MAX);
            if now.duration_since(state.last_restart_at) < backoff {
                return false;
            }
        }
        let state = self
            .restarts
            .entry(engine.to_string())
            .or_insert(RestartBackoff {
                last_restart_at: now,
                attempts: 0,
            });
        state.attempts = state.attempts.saturating_add(1);
        state.last_restart_at = now;
        true
    }

    /// Removes the managed process `pid` so the caller can destroy it with
    /// the manager lock released — dropping it closes its kill-on-close Job
    /// Object. Returns `None` when the registered process has already been
    /// replaced, so a stale kill decision can never remove a fresh engine.
    pub(super) fn take(&mut self, engine: &str, pid: u32) -> Option<ManagedProcess> {
        let current = self
            .processes
            .get(engine)
            .is_some_and(|process| process.child.id() == pid);
        if current {
            self.processes.remove(engine)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EngineHealth, LaunchClaim, ManagedProcess, ProbeOutcome, ProcessDescriptor, ToolManager,
    };
    use crate::job_object::JobObject;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn test_descriptor(engine: &str) -> ProcessDescriptor {
        ProcessDescriptor {
            engine: engine.to_string(),
            port: Some(43_210),
            protocol_version: Some(1),
            token: "memory-only-secret".to_string(),
            started_at: "2026-07-12T06:30:00Z".to_string(),
            health: EngineHealth::Ready,
        }
    }

    #[test]
    fn launch_claim_dedupes_and_releases() {
        let mut manager = ToolManager::default();

        assert_eq!(
            manager.begin_launch("zhihu").expect("claim must load"),
            LaunchClaim::Acquired
        );
        assert_eq!(
            manager.begin_launch("zhihu").expect("claim must load"),
            LaunchClaim::AlreadyStarting
        );
        manager.finish_launch("zhihu");
        assert_eq!(
            manager.begin_launch("zhihu").expect("claim must load"),
            LaunchClaim::Acquired
        );
        manager.finish_launch("zhihu");
        // Other engines are unaffected.
        assert_eq!(
            manager.begin_launch("podcast").expect("claim must load"),
            LaunchClaim::Acquired
        );
        manager.finish_launch("podcast");
    }

    #[test]
    fn launch_claim_reports_running_then_keeps_exited_queryable() {
        let child = Command::new("cmd.exe")
            .args(["/C", "ping -n 30 127.0.0.1 >NUL"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("test child must start");
        let job = JobObject::kill_on_close().expect("job object must be created");
        job.assign(&child).expect("child must join job");
        let mut manager = ToolManager::default();
        manager
            .insert(ManagedProcess::new(child, job, test_descriptor("zhihu")))
            .expect("process must be registered");

        assert_eq!(
            manager.begin_launch("zhihu").expect("claim must load"),
            LaunchClaim::AlreadyRunning
        );

        let mut process = manager.processes.remove("zhihu").expect("process exists");
        process.child.kill().expect("child must be killable");
        let _ = process.child.wait();
        manager
            .processes
            .insert("zhihu".to_string(), process);

        assert_eq!(
            manager.begin_launch("zhihu").expect("claim must load"),
            LaunchClaim::Acquired
        );
        // The exited snapshot stays queryable until the replacement publishes.
        let snapshot = manager
            .refresh("zhihu")
            .expect("process status must refresh")
            .expect("exited process must remain registered");
        assert_eq!(snapshot.health, EngineHealth::Exited);
        assert!(snapshot.exit_status.is_some());
        manager.finish_launch("zhihu");
    }

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

    #[test]
    fn health_probes_count_consecutive_failures_and_take_kills() {
        let child = Command::new("cmd.exe")
            .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("test child must start");
        let pid = child.id();
        let job = JobObject::kill_on_close().expect("job object must be created");
        job.assign(&child).expect("child must join job");
        let mut manager = ToolManager::default();
        manager
            .insert(ManagedProcess::new(child, job, test_descriptor("zhihu")))
            .expect("process must be registered");

        // A probe for a different (stale) pid is never counted.
        assert_eq!(
            manager
                .record_health_probe("zhihu", pid + 1, false)
                .expect("probe outcome must load"),
            ProbeOutcome::Gone
        );
        assert_eq!(
            manager
                .record_health_probe("zhihu", pid, false)
                .expect("probe outcome must load"),
            ProbeOutcome::Failed(1)
        );
        assert_eq!(
            manager
                .record_health_probe("zhihu", pid, false)
                .expect("probe outcome must load"),
            ProbeOutcome::Failed(2)
        );
        assert_eq!(
            manager
                .refresh("zhihu")
                .expect("process status must refresh")
                .expect("process must exist")
                .health,
            EngineHealth::Unresponsive
        );
        // A healthy probe resets the consecutive-failure counter.
        assert_eq!(
            manager
                .record_health_probe("zhihu", pid, true)
                .expect("probe outcome must load"),
            ProbeOutcome::Healthy
        );
        assert_eq!(
            manager
                .record_health_probe("zhihu", pid, false)
                .expect("probe outcome must load"),
            ProbeOutcome::Failed(1)
        );

        // take() only releases the matching process instance, then
        // kill_and_reap terminates it even though it never asked to exit.
        assert!(manager.take("zhihu", pid + 1).is_none());
        let process = manager
            .take("zhihu", pid)
            .expect("live process must be taken");
        assert!(manager
            .refresh("zhihu")
            .expect("refresh must load")
            .is_none());
        assert!(process.kill_and_reap().is_some());
    }

    #[test]
    fn health_probe_reports_exited_processes_as_gone() {
        let child = Command::new("cmd.exe")
            .args(["/C", "exit /B 0"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("test child must start");
        let pid = child.id();
        let job = JobObject::kill_on_close().expect("job object must be created");
        job.assign(&child).expect("child must join job");
        let mut manager = ToolManager::default();
        manager
            .insert(ManagedProcess::new(child, job, test_descriptor("zhihu")))
            .expect("process must be registered");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = manager
                .refresh("zhihu")
                .expect("process status must refresh")
                .expect("process must remain registered");
            if snapshot.exit_status.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "test child did not exit");
            std::thread::yield_now();
        }
        assert_eq!(
            manager
                .record_health_probe("zhihu", pid, false)
                .expect("probe outcome must load"),
            ProbeOutcome::Gone
        );
    }

    #[test]
    fn restart_backoff_permits_first_restart_then_defers() {
        let mut manager = ToolManager::default();

        // The first forced restart is immediate.
        assert!(manager.restart_permitted("zhihu"));
        // attempts=1 -> a second restart must wait out the 10s window.
        assert!(!manager.restart_permitted("zhihu"));

        let state = manager
            .restarts
            .get_mut("zhihu")
            .expect("restart state must exist");
        state.last_restart_at = Instant::now() - Duration::from_secs(11);
        assert!(manager.restart_permitted("zhihu"));

        // attempts=2 -> the window grows to 20s.
        let state = manager
            .restarts
            .get_mut("zhihu")
            .expect("restart state must exist");
        state.last_restart_at = Instant::now() - Duration::from_secs(11);
        assert!(!manager.restart_permitted("zhihu"));

        let state = manager
            .restarts
            .get_mut("zhihu")
            .expect("restart state must exist");
        state.last_restart_at = Instant::now() - Duration::from_secs(21);
        assert!(manager.restart_permitted("zhihu"));

        // Engines keep independent debounce clocks.
        assert!(manager.restart_permitted("podcast"));
    }
}
