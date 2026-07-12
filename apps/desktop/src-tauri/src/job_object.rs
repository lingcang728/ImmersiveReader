use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
};

fn resume_primary_thread(process_id: u32) -> Result<(), String> {
    // SAFETY: [Category 8 - FFI boundary] TH32CS_SNAPTHREAD ignores the process
    // argument and returns an owned read-only system thread snapshot handle.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "CreateToolhelp32Snapshot failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: [Category 12 - invalid free] the successful snapshot handle is
    // transferred exactly once to OwnedHandle for deterministic cleanup.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = THREADENTRY32 {
        dwSize: u32::try_from(std::mem::size_of::<THREADENTRY32>())
            .map_err(|error| error.to_string())?,
        ..THREADENTRY32::default()
    };
    // SAFETY: [Category 8 - FFI boundary] snapshot is live and entry points to
    // writable initialized storage whose dwSize matches THREADENTRY32.
    let mut found = unsafe {
        Thread32First(
            snapshot.as_raw_handle() as HANDLE,
            std::ptr::from_mut(&mut entry),
        )
    };
    while found != 0 {
        if entry.th32OwnerProcessID == process_id {
            return resume_thread(entry.th32ThreadID);
        }
        // SAFETY: [Category 8 - FFI boundary] the same live snapshot and valid
        // THREADENTRY32 storage are reused for the next enumeration result.
        found = unsafe {
            Thread32Next(
                snapshot.as_raw_handle() as HANDLE,
                std::ptr::from_mut(&mut entry),
            )
        };
    }
    Err("PRIMARY_THREAD_NOT_FOUND".to_string())
}

fn resume_thread(thread_id: u32) -> Result<(), String> {
    // SAFETY: [Category 8 - FFI boundary] thread_id came from a live ToolHelp
    // entry and only THREAD_SUSPEND_RESUME access is requested without inheritance.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
    if thread.is_null() {
        return Err(format!(
            "OpenThread failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: [Category 12 - invalid free] the non-null OpenThread handle is
    // transferred exactly once to OwnedHandle for deterministic cleanup.
    let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
    // SAFETY: [Category 8 - FFI boundary] OwnedHandle keeps the thread handle
    // live and THREAD_SUSPEND_RESUME access authorizes ResumeThread.
    let previous_count = unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) };
    if previous_count == u32::MAX {
        return Err(format!(
            "ResumeThread failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    if previous_count != 1 {
        return Err(format!("UNEXPECTED_SUSPEND_COUNT:{previous_count}"));
    }
    Ok(())
}

pub struct JobObject {
    handle: OwnedHandle,
}

impl JobObject {
    pub fn kill_on_close() -> Result<Self, String> {
        // SAFETY: [Category 8 - FFI boundary] both optional pointers are null,
        // requesting default security and an unnamed Job Object from Windows.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(format!(
                "CreateJobObjectW failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: [Category 12 - invalid free] CreateJobObjectW returned one
        // non-null owned handle, transferred exactly once into OwnedHandle.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: [Category 8 - FFI boundary] OwnedHandle keeps the Job handle
        // live, and information is initialized with its exact byte count.
        let configured = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle() as HANDLE,
                JobObjectExtendedLimitInformation,
                (&information as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            let error = std::io::Error::last_os_error();
            return Err(format!("SetInformationJobObject failed: {error}"));
        }
        Ok(Self { handle })
    }

    pub fn assign(&self, child: &Child) -> Result<(), String> {
        let process = child.as_raw_handle() as HANDLE;
        // SAFETY: [Category 8 - FFI boundary] OwnedHandle and Child keep both
        // handles live for the duration of AssignProcessToJobObject.
        if unsafe { AssignProcessToJobObject(self.handle.as_raw_handle() as HANDLE, process) } == 0
        {
            return Err(format!(
                "AssignProcessToJobObject failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub fn spawn_suspended(command: &mut Command) -> Result<(Child, Self), String> {
        command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
        let job = Self::kill_on_close()?;
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        if let Err(error) = job.assign(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        if let Err(error) = resume_primary_thread(child.id()) {
            drop(job);
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, job))
    }
}

#[cfg(test)]
mod tests {
    use super::JobObject;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    #[test]
    fn closing_job_terminates_assigned_process() {
        let mut child = Command::new("cmd.exe")
            .args(["/C", "ping -t 127.0.0.1 >NUL"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("test child must start");
        let job = JobObject::kill_on_close().expect("job object must be created");
        job.assign(&child).expect("child must join job");

        drop(job);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait().expect("child status must load").is_some() {
                child.wait().expect("terminated child must be reaped");
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        panic!("closing the Job Object did not terminate the assigned process");
    }

    #[test]
    fn suspended_spawn_assigns_job_before_resuming_child() {
        let mut command = Command::new("cmd.exe");
        command.args(["/C", "exit /B 0"]);

        let (mut child, job) = JobObject::spawn_suspended(&mut command)
            .expect("suspended child must join job and resume");
        let status = child.wait().expect("resumed child must exit");
        drop(job);

        assert!(status.success());
    }
}
