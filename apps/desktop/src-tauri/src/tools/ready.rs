use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

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
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("ENGINE_READY_TIMEOUT".to_string());
        }
        let line = match receiver.recv_timeout(remaining) {
            Ok(Ok(line)) => line,
            Ok(Err(_)) => return Err("ENGINE_READY_STDOUT".to_string()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err("ENGINE_READY_TIMEOUT".to_string())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("ENGINE_READY_EOF".to_string())
            }
        };
        match parse_ready_line(&line, expected_engine, expected_pid) {
            Ok(ready) => return Ok(ready),
            // Noise lines (lossy-decoded GBK warnings, log output) are skipped
            // so the handshake keeps waiting for READY within the same timeout.
            Err(error) if error == "ENGINE_READY_INVALID_JSON" => continue,
            Err(error) => return Err(error),
        }
    }
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
        // Read raw byte lines: sidecar warnings/logs may arrive in a non-UTF-8
        // code page (e.g. GBK on zh-CN Windows), so decode each line lossily
        // instead of aborting on the first invalid byte.
        let mut stdout = BufReader::new(stdout);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match stdout.read_until(b'\n', &mut buffer) {
                Ok(0) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buffer).into_owned();
                    // Once READY is consumed the receiver is dropped; keep
                    // draining stdout so the child never blocks on a full pipe.
                    let _ = sender.send(Ok(line));
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                    break;
                }
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
    fn skips_decoded_noise_lines_while_waiting_for_ready() {
        let (sender, receiver) = mpsc::sync_channel::<Result<String, String>>(4);
        sender
            .send(Ok("warning: sidecar log noise".to_string()))
            .expect("noise line must send");
        sender
            .send(Ok(
                r#"{"engine":"podcast","protocolVersion":1,"pid":4242,"port":43210}"#
                    .to_string(),
            ))
            .expect("READY line must send");
        let ready = receive_ready(&receiver, "podcast", 4242, Duration::from_secs(1))
            .expect("READY after noise lines must be accepted");
        assert_eq!(ready.port, 43210);
    }

    #[test]
    fn tolerates_non_utf8_noise_before_ready() {
        // Emit raw GBK bytes ("中文\n") on stdout before the READY JSON line.
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "$s=[Console]::OpenStandardOutput(); $b=[byte[]](0xD6,0xD0,0xCE,0xC4,0x0A); $s.Write($b,0,$b.Length); $s.Flush(); Write-Output ('{\"engine\":\"podcast\",\"protocolVersion\":1,\"pid\":' + $PID + ',\"port\":43210}')",
            ])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("noisy PowerShell child must start");
        let expected_pid = child.id();
        let (ready, reader) = wait_for_ready(&mut child, "podcast", Duration::from_secs(5))
            .expect("READY after non-UTF-8 noise must be accepted");

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
