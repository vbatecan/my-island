//! Read-only app-server handshake. No conversations, inference, or auth files.
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use still_island_plugin_sdk::serde_json::{self, Value, json};

const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_LINE: usize = 64 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
type Message = Result<Value, String>;

pub fn read_limits(executable: &Path, cancel: &AtomicBool) -> Result<Value, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Codex usage request cancelled".into());
    }
    let deadline = Instant::now() + TIMEOUT;
    let mut command = Command::new(executable);
    command
        .args([
            "-c",
            "analytics.enabled=false",
            "-c",
            "otel.exporter=\"none\"",
            "-c",
            "otel.trace_exporter=\"none\"",
            "app-server",
            "--listen",
            "stdio://",
        ])
        .env("OTEL_SDK_DISABLED", "true")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Keep the host's Job Object inheritance; do not use BREAKAWAY_FROM_JOB.
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut session = Session::start(&mut command)?;
    session.send(&json!({
        "id": 1, "method": "initialize", "params": {
            "clientInfo": { "name": "still_island_usage", "title": "Still Island Usage", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": false }
        }
    }))?;
    session.response(1, deadline, cancel)?;
    session.send(&json!({ "method": "initialized" }))?;
    session.send(&json!({ "id": 2, "method": "account/rateLimits/read" }))?;
    session.response(2, deadline, cancel)
}

struct Session {
    child: Child,
    receiver: Option<Receiver<Message>>,
    reader: Option<JoinHandle<()>>,
    finished: Receiver<()>,
    stopping: Arc<AtomicBool>,
}

impl Session {
    fn start(command: &mut Command) -> Result<Self, String> {
        let child = command
            .spawn()
            .map_err(|_| "Cannot start the selected Codex executable".to_owned())?;
        let (sender, receiver) = mpsc::sync_channel(4);
        let (done, finished) = mpsc::sync_channel(1);
        let stopping = Arc::new(AtomicBool::new(false));
        // Establish cleanup ownership before any subsequent fallible operation.
        let mut session = Self {
            child,
            receiver: Some(receiver),
            reader: None,
            finished,
            stopping,
        };
        let stdout = session
            .child
            .stdout
            .take()
            .ok_or("Codex output pipe unavailable")?;
        let stopping = session.stopping.clone();
        session.reader = Some(
            std::thread::Builder::new()
                .name("codex-usage-read".into())
                .spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    let mut total = 0;
                    while !stopping.load(Ordering::Acquire) {
                        let message = match read_line(&mut reader, &mut total) {
                            Ok(Some(line)) if line.iter().all(u8::is_ascii_whitespace) => continue,
                            Ok(Some(line)) => serde_json::from_slice(&line)
                                .map_err(|_| "Codex returned invalid protocol data".to_owned()),
                            Ok(None) => break,
                            Err(error) => Err(error),
                        };
                        let failed = message.is_err();
                        if sender.send(message).is_err() || failed {
                            break;
                        }
                    }
                    let _ = done.send(());
                })
                .map_err(|_| "Cannot create Codex response reader")?,
        );
        Ok(session)
    }

    fn send(&mut self, value: &Value) -> Result<(), String> {
        let input = self
            .child
            .stdin
            .as_mut()
            .ok_or("Codex input pipe unavailable")?;
        // All requests are small constant schemas, well below the pipe capacity.
        serde_json::to_writer(&mut *input, value).map_err(|_| "Cannot send Codex request")?;
        input
            .write_all(b"\n")
            .and_then(|_| input.flush())
            .map_err(|_| "Cannot send Codex request".into())
    }

    fn response(
        &self,
        expected: u64,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let receiver = self
            .receiver
            .as_ref()
            .ok_or("Codex response pipe unavailable")?;
        receive_response(receiver, expected, deadline, cancel)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        drop(self.child.stdin.take());
        // This handle belongs exclusively to this request, never the user's UI.
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Unblock a reader waiting to send into a full bounded channel.
        drop(self.receiver.take());
        if let Some(reader) = self.reader.take() {
            #[cfg(windows)]
            if !reader.is_finished() {
                use std::os::windows::io::AsRawHandle;
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn CancelSynchronousIo(thread: *mut std::ffi::c_void) -> i32;
                }
                // SAFETY: JoinHandle owns a live thread handle until dropped.
                // Cancellation only interrupts that thread's pending pipe read.
                unsafe {
                    let _ = CancelSynchronousIo(reader.as_raw_handle());
                }
            }
            // An unexpected descendant may retain stdout. Cleanup must not hang
            // the plugin: cancel the read, then bound the completion wait. The
            // plugin host Job Object remains responsible for the process tree.
            let _ = self.finished.recv_timeout(Duration::from_millis(100));
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}

fn receive_response(
    receiver: &Receiver<Message>,
    expected: u64,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Codex usage request cancelled".into());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Codex usage request timed out".into());
        }
        let message = match receiver.recv_timeout(remaining.min(Duration::from_millis(75))) {
            Ok(message) => message?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Codex exited before replying".into());
            }
        };
        if let Some(result) = response_result(message, expected) {
            return result;
        }
    }
}

fn response_result(message: Value, expected: u64) -> Option<Message> {
    if message.get("id").and_then(Value::as_u64) != Some(expected) {
        return None;
    }
    if message.get("error").is_some() {
        // Never echo remote error text: it may include account or token details.
        return Some(Err(
            "Codex could not read account limits; check its sign-in and account support".into(),
        ));
    }
    Some(
        message
            .get("result")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| "Codex returned an unsupported response".into()),
    )
}

fn read_line(reader: &mut impl BufRead, total: &mut usize) -> Result<Option<Vec<u8>>, String> {
    let mut line = Vec::new();
    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|_| "Cannot read Codex response")?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err("Codex returned an incomplete protocol line".into())
            };
        }
        let used = buffer
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(buffer.len(), |i| i + 1);
        if line.len().saturating_add(used) > MAX_LINE || total.saturating_add(used) > MAX_OUTPUT {
            return Err("Codex response exceeded the output budget".into());
        }
        let complete = buffer[used - 1] == b'\n';
        line.extend_from_slice(&buffer[..used]);
        *total += used;
        reader.consume(used);
        if complete {
            return Ok(Some(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn ignores_notifications_and_other_request_ids() {
        assert!(
            response_result(
                json!({"method":"account/updated","params":{"token":"secret"}}),
                2
            )
            .is_none()
        );
        assert!(response_result(json!({"id":1,"result":{}}), 2).is_none());
        assert_eq!(
            response_result(json!({"id":2,"result":{"rateLimits":{}}}), 2)
                .unwrap()
                .unwrap(),
            json!({"rateLimits":{}})
        );
    }

    #[test]
    fn malformed_or_error_responses_never_echo_remote_content() {
        let error = response_result(json!({"id":2,"error":{"message":"secret-token"}}), 2)
            .unwrap()
            .unwrap_err();
        assert!(!error.contains("secret"));
        assert!(
            response_result(json!({"id":2,"result":null}), 2)
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn cancellation_and_deadline_stop_waiting_without_a_server() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let cancel = AtomicBool::new(true);
        assert!(
            receive_response(&receiver, 2, Instant::now() + TIMEOUT, &cancel)
                .unwrap_err()
                .contains("cancelled")
        );
        cancel.store(false, Ordering::Release);
        assert!(
            receive_response(&receiver, 2, Instant::now(), &cancel)
                .unwrap_err()
                .contains("timed out")
        );
    }

    #[test]
    fn bounds_each_line_and_aggregate_output() {
        let mut total = 0;
        assert!(read_line(&mut Cursor::new(vec![b'x'; MAX_LINE + 1]), &mut total).is_err());
        let mut total = MAX_OUTPUT - 1;
        assert!(read_line(&mut Cursor::new(b"{}\n"), &mut total).is_err());
        assert!(read_line(&mut Cursor::new(b"{}"), &mut 0).is_err());
        assert_eq!(
            read_line(&mut Cursor::new(b"{}\n"), &mut 0)
                .unwrap()
                .unwrap(),
            b"{}\n"
        );
    }
}
