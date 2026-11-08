mod client;
mod usage;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::SystemTime,
};
use still_island_plugin_sdk::{
    serde_json::{Value, json},
    *,
};
use usage::Snapshot;

enum Event {
    Host(HostMessage),
    End(Result<(), String>),
    Fetched(u64, Result<Value, String>),
}

#[derive(Default)]
struct State {
    executable: PathBuf,
    snapshot: Option<Snapshot>,
    error: String,
    busy: bool,
    attempted: bool,
    preview_pending: bool,
    generation: u64,
    revision: u64,
}

fn executable(config: &Value) -> PathBuf {
    if let Some(path) = config["executable"]
        .as_str()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        return PathBuf::from(path);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let installed = PathBuf::from(local).join("Programs/OpenAI/Codex/bin/codex.exe");
        if installed.is_file() {
            return installed;
        }
    }
    PathBuf::from(if cfg!(windows) { "codex.exe" } else { "codex" })
}

impl State {
    fn publish(&mut self) -> Result<(), String> {
        self.revision += 1;
        // The host already supplies the integration name above the view.
        let mut nodes = Vec::new();
        if self.busy {
            nodes.push(text("status", "Reading Codex limits…"));
        } else if !self.error.is_empty() {
            nodes.push(text("status", &self.error));
        }
        if let Some(snapshot) = &self.snapshot {
            if !self.error.is_empty() {
                nodes.push(text("stale", "Refresh failed · last reported usage below"));
            }
            nodes.extend(snapshot.nodes(SystemTime::now()));
        } else if !self.busy && self.error.is_empty() {
            nodes.push(text(
                "status",
                "Open Usage or choose Refresh to read your signed-in Codex quota.",
            ));
        }
        nodes.push(Node {
            id: "actions".into(),
            kind: NodeKind::Row,
            spacing: 6.,
            children: vec![
                button("refresh", "Refresh", "refresh"),
                button("show", "Show in notch", "show"),
            ],
            ..Node::default()
        });
        send(&PluginMessage::View {
            compact: self.snapshot.as_ref().map(|snapshot| {
                let mut summary = snapshot.compact();
                if !self.error.is_empty() {
                    summary.body = format!("Last reported · {}", summary.body);
                }
                summary
            }),
            view_id: "main".into(),
            revision: self.revision,
            root: column(nodes),
        })
    }
}

fn run_plugin() -> Result<(), String> {
    let (sender, receiver) = mpsc::sync_channel(32);
    let input_sender = sender.clone();
    thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        loop {
            match read_message::<HostMessage>(&mut input) {
                Ok(Some(message)) => {
                    let shutdown = matches!(message, HostMessage::Shutdown);
                    if input_sender.send(Event::Host(message)).is_err() || shutdown {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = input_sender.send(Event::End(Ok(())));
                    break;
                }
                Err(error) => {
                    let _ = input_sender.send(Event::End(Err(error)));
                    break;
                }
            }
        }
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let mut worker = None;
    let mut state = State::default();
    let mut initialized = false;
    let outcome = (|| {
        while let Ok(event) = receiver.recv() {
            match event {
                Event::End(result) => return result,
                Event::Host(HostMessage::Shutdown) => break,
                Event::Host(HostMessage::Init {
                    protocol_version,
                    config,
                }) => {
                    if initialized || protocol_version != PROTOCOL_VERSION {
                        return Err("Unsupported plugin initialization".into());
                    }
                    initialized = true;
                    state.executable = executable(&config);
                    send(&PluginMessage::Ready {
                        protocol_version: PROTOCOL_VERSION,
                    })?;
                }
                _ if !initialized => {
                    return Err("Initialize the plugin before requesting usage".into());
                }
                Event::Host(HostMessage::Configure { config }) => {
                    let next = executable(&config);
                    if next != state.executable {
                        cancel.store(true, Ordering::Release);
                        state.generation = state.generation.wrapping_add(1);
                        state.executable = next;
                        state.snapshot = None;
                        state.error.clear();
                        state.attempted = false;
                        state.preview_pending = false;
                    }
                }
                Event::Host(HostMessage::ViewRequest { view_id }) => {
                    if view_id != "main" {
                        return Err("Unknown usage view".into());
                    }
                    if !state.attempted && !state.busy {
                        begin_refresh(&mut state, &cancel, &sender, &mut worker);
                    }
                }
                Event::Host(HostMessage::Invoke { action_id, .. }) => match action_id.as_str() {
                    "refresh" => {
                        let already = state.busy;
                        if !already {
                            begin_refresh(&mut state, &cancel, &sender, &mut worker);
                        }
                        result(
                            action_id,
                            if already {
                                "Refresh already running"
                            } else {
                                "Refresh started"
                            },
                            None,
                        )?;
                    }
                    "show" => {
                        if let Some(snapshot) = &state.snapshot {
                            send(&snapshot.preview())?;
                            result(action_id, "Showing reported usage", None)?;
                        } else {
                            state.preview_pending = true;
                            if !state.busy {
                                begin_refresh(&mut state, &cancel, &sender, &mut worker);
                            }
                            result(action_id, "Reading usage for notch preview", None)?;
                        }
                    }
                    _ => failure(action_id, "Unknown action")?,
                },
                Event::Fetched(generation, response) => {
                    if let Some(handle) = worker.take() {
                        let _ = handle.join();
                    }
                    state.busy = false;
                    if generation == state.generation {
                        match response.and_then(|data| Snapshot::parse(&data, SystemTime::now())) {
                            Ok(snapshot) => {
                                if state.preview_pending {
                                    send(&snapshot.preview())?;
                                }
                                state.error.clear();
                                state.snapshot = Some(snapshot);
                            }
                            Err(error) => state.error = error,
                        }
                        state.preview_pending = false;
                    }
                }
            }
            state.publish()?;
        }
        Ok(())
    })();
    cancel.store(true, Ordering::Release);
    // Closing the receiver also releases a worker blocked on a full event queue.
    drop(receiver);
    if let Some(handle) = worker.take() {
        let _ = handle.join();
    }
    outcome
}

fn begin_refresh(
    state: &mut State,
    cancel: &Arc<AtomicBool>,
    sender: &mpsc::SyncSender<Event>,
    worker: &mut Option<thread::JoinHandle<()>>,
) {
    state.busy = true;
    state.attempted = true;
    cancel.store(false, Ordering::Release);
    let path = state.executable.clone();
    let generation = state.generation;
    let cancel = cancel.clone();
    let sender = sender.clone();
    *worker = Some(thread::spawn(move || {
        let result = client::read_limits(&path, &cancel);
        let _ = sender.send(Event::Fetched(generation, result));
    }));
}

fn main() {
    let outcome = if std::env::args().any(|arg| arg == "--check") {
        client::read_limits(&executable(&Value::Null), &AtomicBool::new(false)).and_then(|data| {
            let snapshot = Snapshot::parse(&data, SystemTime::now())?;
            let windows: Vec<_> = snapshot.windows.iter().map(|w| json!({"used_percent":w.used,"window_minutes":w.minutes,"resets_at":w.reset})).collect();
            println!("{}", json!({"windows":windows}));
            Ok(())
        })
    } else {
        run_plugin()
    };
    if let Err(error) = outcome {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
