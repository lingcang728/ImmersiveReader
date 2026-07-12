use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReadyMessage {
    pub engine: String,
    pub protocol_version: u32,
    pub pid: u32,
    pub port: u16,
}

pub(super) fn parse_ready_line(
    line: &str,
    expected_engine: &str,
    expected_pid: u32,
) -> Result<ReadyMessage, String> {
    let ready: ReadyMessage =
        serde_json::from_str(line.trim()).map_err(|_| "ENGINE_READY_INVALID_JSON".to_string())?;
    if ready.engine != expected_engine {
        return Err("ENGINE_READY_ENGINE_MISMATCH".to_string());
    }
    if ready.protocol_version != PROTOCOL_VERSION {
        return Err("ENGINE_READY_PROTOCOL_MISMATCH".to_string());
    }
    if ready.pid != expected_pid {
        return Err("ENGINE_READY_PID_MISMATCH".to_string());
    }
    if ready.port == 0 {
        return Err("ENGINE_READY_PORT_INVALID".to_string());
    }
    Ok(ready)
}

fn receive_ready(
    receiver: &Receiver<Result<String, String>>,
    expected_engine: &str,
    expected_pid: u32,
    timeout: Duration,
) -> Result<ReadyMessage, String> {
    let line = match receiver.recv_timeout(timeout) {
        Ok(Ok(line)) => line,
        Ok(Err(_)) => return Err("ENGINE_READY_STDOUT".to_string()),
        Err(mpsc::RecvTimeoutError::Timeout) => return Err("ENGINE_READY_TIMEOUT".to_string()),
        Err(mpsc::RecvTimeoutError::Disconnected) => return Err("ENGINE_READY_EOF".to_string()),
    };
    parse_ready_line(&line, expected_engine, expected_pid)
}

pub(super) fn wait_for_ready(
    child: &mut Child,
    expected_engine: &str,
    timeout: Duration,
) -> Result<(ReadyMessage, JoinHandle<()>), String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ENGINE_READY_STDOUT_MISSING".to_string())?;
    let expected_pid = child.id();
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        let first = match lines.next() {
            Some(Ok(line)) => Ok(line),
            Some(Err(error)) => Err(error.to_string()),
            None => Err("ENGINE_READY_EOF".to_string()),
        };
        let _ = sender.send(first);
        for line in lines {
            if line.is_err() {
                break;
            }
        }
    });
    let ready = receive_ready(&receiver, expected_engine, expected_pid, timeout)?;
    Ok((ready, reader))
}

#[cfg(test)]
mod tests {
    use super::{receive_ready, wait_for_ready, ReadyMessage};
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn validates_ready_engine_protocol_pid_and_dynamic_port() {
        let ready = super::parse_ready_line(
            r#"{"engine":"podcast","protocolVersion":1,"pid":4242,"port":43210}"#,
            "podcast",
            4242,
        )
        .expect("valid READY JSON must be accepted");

        assert_eq!(
            ready,
            ReadyMessage {
                engine: "podcast".to_string(),
                protocol_version: 1,
                pid: 4242,
                port: 43210,
            }
        );
    }

    #[test]
    fn rejects_mismatched_identity_and_zero_port() {
        for (line, expected_error) in [
            (
                r#"{"engine":"zhihu","protocolVersion":1,"pid":4242,"port":43210}"#,
                "ENGINE_READY_ENGINE_MISMATCH",
            ),
            (
                r#"{"engine":"podcast","protocolVersion":2,"pid":4242,"port":43210}"#,
                "ENGINE_READY_PROTOCOL_MISMATCH",
            ),
            (
                r#"{"engine":"podcast","protocolVersion":1,"pid":99,"port":43210}"#,
                "ENGINE_READY_PID_MISMATCH",
            ),
            (
                r#"{"engine":"podcast","protocolVersion":1,"pid":4242,"port":0}"#,
                "ENGINE_READY_PORT_INVALID",
            ),
        ] {
            assert_eq!(
                super::parse_ready_line(line, "podcast", 4242).unwrap_err(),
                expected_error
            );
        }
    }

    #[test]
    fn times_out_without_a_ready_line() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let error =
            receive_ready(&receiver, "podcast", 4242, Duration::from_millis(5)).unwrap_err();
        assert_eq!(error, "ENGINE_READY_TIMEOUT");
    }

    #[test]
    fn reads_ready_from_a_live_child_stdout() {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "Write-Output ('{\"engine\":\"podcast\",\"protocolVersion\":1,\"pid\":' + $PID + ',\"port\":43210}')",
            ])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("PowerShell child must start");
        let expected_pid = child.id();
        let (ready, reader) = wait_for_ready(&mut child, "podcast", Duration::from_secs(5))
            .expect("live child READY line must be accepted");

        assert_eq!(ready.pid, expected_pid);
        assert_eq!(ready.port, 43210);
        child.wait().expect("child must be reaped");
        reader.join().expect("stdout reader must finish");
    }

    #[test]
    fn times_out_and_allows_caller_to_reap_a_silent_child() {
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 2"])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("silent PowerShell child must start");
        let error = wait_for_ready(&mut child, "podcast", Duration::from_millis(20))
            .expect_err("silent child must hit the handshake timeout");
        assert_eq!(error, "ENGINE_READY_TIMEOUT");
        child.kill().expect("silent child must be terminated");
        child.wait().expect("silent child must be reaped");
    }
}
