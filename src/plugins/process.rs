//! Per-running-plugin blocking pipe readers and a sleeping supervisor. Nothing
//! waits for plugin I/O on the Slint thread; queues and deadlines are bounded.
use super::*;
use std::{
    io::{BufReader, Read},
    os::windows::process::CommandExt,
    process::{Command as OsCommand, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};
use still_island_plugin_protocol::{
    PROTOCOL_VERSION, PluginMessage, read_message, validate_compact_summary, validate_layout,
    write_message,
};
pub(super) enum Command {
    Send(HostMessage),
    Visible(bool),
    Stop,
    Received(PluginMessage),
    Exited(String),
}
pub(super) struct ProcessControl {
    job: Mutex<Option<native::Job>>,
    cancelled: AtomicBool,
}
impl ProcessControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn terminate(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(job) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            job.terminate();
        }
    }
}
pub(super) fn start(inner: &Arc<Inner>, id: &str) -> Result<(), String> {
    let config = inner.full_configuration(id)?;
    let (manifest, directory, generation, receiver, control, sender) = {
        let mut entries = inner.entries.lock().unwrap_or_else(|e| e.into_inner());
        let e = entries.get_mut(id).ok_or("Unknown plugin")?;
        if !e.prefs.enabled {
            return Err("Enable this trusted plugin before running it".into());
        }
        if e.running.is_some() {
            return Ok(());
        }
        if e.error.is_some() {
            return Err("Plugin unavailable. Choose Restart to try again".into());
        }
        let (sender, receiver) = mpsc::sync_channel(128);
        let control = Arc::new(ProcessControl {
            job: Mutex::new(None),
            cancelled: AtomicBool::new(false),
        });
        e.generation = e.generation.wrapping_add(1);
        let generation = e.generation;
        e.running = Some(Running {
            sender: sender.clone(),
            job: control.clone(),
        });
        e.status = "Starting".into();
        (
            e.manifest.clone(),
            e.directory.clone(),
            generation,
            receiver,
            control,
            sender,
        )
    };
    let inner = inner.clone();
    let id = id.to_owned();
    let failure_inner = inner.clone();
    let failure_id = id.clone();
    thread::Builder::new()
        .name(format!("plugin-{id}"))
        .spawn(move || {
            let result = supervise(
                &inner, &manifest, &directory, config, receiver, sender, &control, generation,
            );
            control.terminate();
            let current = {
                let mut entries = inner.entries.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(e) = entries.get_mut(&id) {
                    if e.generation == generation {
                        e.running = None;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
            if current {
                match result {
                    Ok(()) => inner.status(&id, "Stopped", None),
                    Err(error) => inner.status(&id, "Unavailable", Some(error)),
                }
            }
        })
        .map_err(|e| {
            failure_inner.status(
                &failure_id,
                "Unavailable",
                Some("Could not start plugin supervisor".into()),
            );
            e.to_string()
        })?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn supervise(
    inner: &Arc<Inner>,
    manifest: &Manifest,
    directory: &Path,
    config: Value,
    receiver: Receiver<Command>,
    sender: SyncSender<Command>,
    control: &Arc<ProcessControl>,
    generation: u64,
) -> Result<(), String> {
    package::directory_inside(&inner.root, directory)?;
    let root = directory
        .canonicalize()
        .map_err(|_| "Plugin directory is missing")?;
    if !root.starts_with(&inner.root) {
        return Err("Plugin directory escapes its installation".into());
    }
    let live = package::read_manifest(&root.join("manifest.json"))?;
    if serde_json::to_value(&live).ok() != serde_json::to_value(manifest).ok() {
        return Err("Plugin manifest changed; import the package again".into());
    }
    let exe = package::contained_file(&root, &manifest.entrypoint)?;
    for asset in &manifest.assets {
        package::contained_file(&root, asset)?;
    }
    if control.cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut child = OsCommand::new(exe)
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| "Could not start plugin executable")?;
    let job = match native::Job::attach(&child) {
        Ok(job) => job,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not attach plugin process cleanup job".into());
        }
    };
    *control.job.lock().unwrap_or_else(|e| e.into_inner()) = Some(job);
    if control.cancelled.load(Ordering::Acquire) {
        control.terminate();
        let _ = child.wait();
        return Ok(());
    }
    let input = child.stdin.take().ok_or("Plugin stdin unavailable")?;
    let output = child.stdout.take().ok_or("Plugin stdout unavailable")?;
    let mut errors = child.stderr.take().ok_or("Plugin stderr unavailable")?;
    let reader_sender = sender.clone();
    let stdout = thread::Builder::new()
        .name("plugin-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let next = match read_message::<PluginMessage>(&mut reader) {
                    Ok(Some(message)) => Command::Received(message),
                    Ok(None) => Command::Exited("Plugin process exited".into()),
                    Err(e) => Command::Exited(e),
                };
                let exited = matches!(next, Command::Exited(_));
                if reader_sender.send(next).is_err() || exited {
                    break;
                }
            }
        })
        .map_err(|_| "Could not start plugin output reader")?;
    // Keep only complete bounded lines: secrets split across pipe chunks must
    // be redacted before any truncation or retention.
    let diagnostic_host = Arc::downgrade(inner);
    let diagnostic_id = manifest.id.clone();
    let stderr = thread::Builder::new()
        .name("plugin-errors".into())
        .spawn(move || {
            let mut buffer = [0u8; 1024];
            let mut line = Vec::new();
            let mut oversized = false;
            while let Ok(count) = errors.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                for byte in &buffer[..count] {
                    if *byte == b'\n' {
                        if let Some(host) = diagnostic_host.upgrade() {
                            host.log(
                                &diagnostic_id,
                                if oversized {
                                    "Diagnostic line exceeded limit"
                                } else {
                                    std::str::from_utf8(&line).unwrap_or("Invalid diagnostic text")
                                },
                            );
                        }
                        line.clear();
                        oversized = false;
                    } else if !oversized {
                        if line.len() < 65536 {
                            line.push(*byte);
                        } else {
                            line.clear();
                            oversized = true;
                        }
                    }
                }
            }
        })
        .map_err(|_| "Could not start plugin diagnostic reader")?;
    let (writer_sender, writer_receiver) = mpsc::sync_channel::<HostMessage>(32);
    let writer_events = sender.clone();
    let writer = thread::Builder::new()
        .name("plugin-input".into())
        .spawn(move || {
            let mut input = input;
            while let Ok(message) = writer_receiver.recv() {
                if let Err(error) = write_message(&mut input, &message) {
                    let _ = writer_events.send(Command::Exited(error));
                    break;
                }
            }
        })
        .map_err(|_| "Could not start plugin input writer")?;
    let result = (|| {
        writer_sender
            .try_send(HostMessage::Init {
                protocol_version: PROTOCOL_VERSION,
                config,
            })
            .map_err(|_| "Plugin input queue is full")?;
        let mut ready = false;
        let startup = Instant::now() + Duration::from_secs(5);
        let mut queued = Vec::new();
        let mut pending = HashMap::<String, Instant>::new();
        let mut revisions = HashMap::new();
        let mut visible = false;
        let mut last_view = Instant::now() - Duration::from_secs(2);
        let mut latest_view = None;
        let mut activity_shown: Option<Instant> = None;
        let mut error_shown: Option<Instant> = None;
        let mut rate_window = Instant::now();
        let mut message_count = 0u32;
        loop {
            if control.cancelled.load(Ordering::Acquire) {
                graceful_shutdown(&writer_sender, &mut child);
                return Ok(());
            }
            let valid = inner
                .entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&manifest.id)
                .is_some_and(|e| e.generation == generation);
            if !valid {
                graceful_shutdown(&writer_sender, &mut child);
                return Ok(());
            }
            let now = Instant::now();
            if !ready && now >= startup {
                return Err("Plugin initialization timed out".into());
            }
            if pending.values().any(|end| now >= *end) {
                return Err("Plugin action timed out".into());
            }
            let cadence = Duration::from_millis(if visible { 100 } else { 1000 });
            if latest_view.is_some() && now.duration_since(last_view) >= cadence {
                inner.emit(&manifest.id, latest_view.take().unwrap());
                last_view = now;
            }
            let mut deadline = if !ready { Some(startup) } else { None };
            for end in pending.values() {
                deadline = Some(deadline.map_or(*end, |old| old.min(*end)));
            }
            if latest_view.is_some() {
                let end = last_view + cadence;
                deadline = Some(deadline.map_or(end, |old| old.min(end)));
            }
            let command = if let Some(end) = deadline {
                match receiver.recv_timeout(end.saturating_duration_since(Instant::now())) {
                    Ok(c) => c,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => return Ok(()),
                }
            } else {
                receiver.recv().map_err(|_| "Plugin supervisor stopped")?
            };
            match command {
                Command::Stop => {
                    graceful_shutdown(&writer_sender, &mut child);
                    return Ok(());
                }
                Command::Exited(error) => return Err(error),
                Command::Visible(value) => visible = value,
                Command::Send(message) => {
                    if !ready {
                        if queued.len() >= 32 {
                            return Err("Plugin initialization queue overflow".into());
                        }
                        queued.push(message);
                        continue;
                    }
                    if let HostMessage::Invoke { action_id, .. } = &message {
                        pending.insert(action_id.clone(), Instant::now() + Duration::from_secs(10));
                    }
                    if let HostMessage::ViewRequest { view_id } = &message {
                        pending.insert(
                            format!("view:{view_id}"),
                            Instant::now() + Duration::from_secs(10),
                        );
                    }
                    writer_sender
                        .try_send(message)
                        .map_err(|_| "Plugin input queue is full")?;
                }
                Command::Received(message) => {
                    if rate_window.elapsed() >= Duration::from_secs(1) {
                        rate_window = Instant::now();
                        message_count = 0;
                    }
                    message_count += 1;
                    if message_count > 100 {
                        return Err("Plugin message rate exceeded the host limit".into());
                    }
                    match &message {
                        PluginMessage::Ready { protocol_version } => {
                            if ready || *protocol_version != PROTOCOL_VERSION {
                                return Err("Incompatible or repeated plugin handshake".into());
                            }
                            ready = true;
                            inner.status(&manifest.id, "Running", None);
                            for message in queued.drain(..) {
                                if let HostMessage::ViewRequest { view_id } = &message {
                                    pending.insert(
                                        format!("view:{view_id}"),
                                        Instant::now() + Duration::from_secs(10),
                                    );
                                }
                                if let HostMessage::Invoke { action_id, .. } = &message {
                                    pending.insert(
                                        action_id.clone(),
                                        Instant::now() + Duration::from_secs(10),
                                    );
                                }
                                writer_sender
                                    .try_send(message)
                                    .map_err(|_| "Plugin input queue is full")?;
                            }
                        }
                        _ if !ready => {
                            return Err("Plugin sent data before completing initialization".into());
                        }
                        PluginMessage::View {
                            view_id,
                            revision,
                            root,
                            compact,
                        } => {
                            if !manifest.views.iter().any(|v| &v.id == view_id) {
                                return Err("Plugin sent an undeclared view".into());
                            }
                            validate_layout(root, manifest)?;
                            if let Some(summary) = compact {
                                validate_compact_summary(summary)?;
                            }
                            pending.remove(&format!("view:{view_id}"));
                            if revisions.get(view_id).is_some_and(|r| revision <= r) {
                                continue;
                            }
                            revisions.insert(view_id.clone(), *revision);
                            latest_view = Some(redacted_value(inner, &manifest.id, &message)?);
                        }
                        PluginMessage::Activity {
                            id, title, body, ..
                        } => {
                            if !still_island_plugin_protocol::valid_id(id)
                                || title.len() > 200
                                || body.len() > 1000
                            {
                                return Err("Invalid activity request".into());
                            }
                            if activity_shown
                                .is_some_and(|last| last.elapsed() < Duration::from_secs(30))
                            {
                                continue;
                            }
                            activity_shown = Some(Instant::now());
                            inner
                                .emit(&manifest.id, redacted_value(inner, &manifest.id, &message)?);
                        }
                        PluginMessage::ActionResult {
                            action_id,
                            message: result_text,
                            ..
                        } => {
                            if result_text.len() > 4096 {
                                return Err("Plugin action result exceeds size limit".into());
                            }
                            // Only a pending explicit host invocation can produce a
                            // URI launch response. Unsolicited results are ignored.
                            if pending.remove(action_id).is_some() {
                                inner.emit(
                                    &manifest.id,
                                    redacted_value(inner, &manifest.id, &message)?,
                                );
                            }
                        }
                        PluginMessage::Error { message } => {
                            // A view can fail legitimately (for example a bad API
                            // token). Keep the process configurable; action calls
                            // still require their explicit ActionResult response.
                            pending.retain(|id, _| !id.starts_with("view:"));
                            if error_shown
                                .is_some_and(|last| last.elapsed() < Duration::from_secs(1))
                            {
                                continue;
                            }
                            error_shown = Some(Instant::now());
                            inner.status(&manifest.id, "Running", Some(message.clone()))
                        }
                    }
                }
            }
        }
    })();
    control.terminate();
    drop(writer_sender);
    let _ = child.wait();
    drop(receiver);
    drop(sender);
    let _ = writer.join();
    let _ = stdout.join();
    let _ = stderr.join();
    result
}
fn graceful_shutdown(sender: &SyncSender<HostMessage>, child: &mut std::process::Child) {
    // The GUI only sets cancellation and queues Stop. This bounded grace period
    // runs on the supervisor, while the independent writer can remain blocked.
    let _ = sender.try_send(HostMessage::Shutdown);
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        if child.try_wait().is_ok_and(|status| status.is_some()) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // The supervisor's unconditional job termination immediately follows this
    // helper, including when the plugin never reads its shutdown message.
}
fn redacted_value(inner: &Inner, id: &str, message: &PluginMessage) -> Result<Value, String> {
    let mut value = serde_json::to_value(message).map_err(|e| e.to_string())?;
    let secrets = inner.secrets(id)?;
    fn visit(secrets: &HashMap<String, String>, value: &mut Value) {
        match value {
            Value::String(s) => {
                for secret in secrets.values().filter(|s| !s.is_empty()) {
                    *s = s.replace(secret, "[redacted]");
                }
                *s = s.chars().take(4096).collect();
            }
            Value::Array(a) => {
                for v in a {
                    visit(secrets, v)
                }
            }
            Value::Object(o) => {
                for v in o.values_mut() {
                    visit(secrets, v)
                }
            }
            _ => {}
        }
    }
    visit(&secrets, &mut value);
    Ok(value)
}
