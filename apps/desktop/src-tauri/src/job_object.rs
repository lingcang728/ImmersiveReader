use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Child;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

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
}
