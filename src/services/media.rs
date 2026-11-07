//! Event-driven access to Windows' active system media session.
//!
//! WinRT objects and potentially remote async calls live on one MTA worker. The
//! GUI sends commands without blocking; their actual results arrive in snapshots.
//! No playback engine, synthetic metadata, or idle refresh timer is used.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Foundation::TypedEventHandler,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    },
    Storage::Streams::DataReader,
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};

#[derive(Clone, Debug, PartialEq)]
pub struct MediaSnapshot {
    pub title: String,
    pub artist: String,
    pub source: String,
    pub playing: bool,
    pub available: bool,
    pub can_play_pause: bool,
    pub can_previous: bool,
    pub can_next: bool,
    pub can_seek: bool,
    pub position_seconds: f64,
    pub duration_seconds: f64,
    pub artwork: Option<Vec<u8>>,
    pub error: Option<String>,
}
impl Default for MediaSnapshot {
    fn default() -> Self {
        Self {
            title: "Nothing playing".into(),
            artist: "Start media in a supported app".into(),
            source: String::new(),
            playing: false,
            available: false,
            can_play_pause: false,
            can_previous: false,
            can_next: false,
            can_seek: false,
            position_seconds: 0.0,
            duration_seconds: 0.0,
            artwork: None,
            error: None,
        }
    }
}

enum Command {
    Refresh,
    PlayPause,
    Previous,
    Next,
    Seek(f64),
    Shutdown,
}
struct Shared {
    snapshot: Mutex<MediaSnapshot>,
    callback: Box<dyn Fn(MediaSnapshot) + Send + Sync>,
    stopping: AtomicBool,
    dirty: AtomicU8,
    initialized: AtomicBool,
    sender: mpsc::Sender<Command>,
}
impl Shared {
    fn publish(&self, snapshot: MediaSnapshot) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        let changed = {
            let mut previous = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            if *previous == snapshot {
                false
            } else {
                *previous = snapshot.clone();
                true
            }
        };
        let first = !self.initialized.swap(true, Ordering::AcqRel);
        if changed || first {
            (self.callback)(snapshot);
        }
    }
    fn notify(&self, flags: u8) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        if self.dirty.fetch_or(flags, Ordering::AcqRel) == 0 {
            let _ = self.sender.send(Command::Refresh);
        }
    }
}

pub struct MediaService {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl MediaService {
    /// Starts asynchronously. Initialization and remote failures are reported in
    /// `MediaSnapshot::error`; this call only fails if the worker cannot start.
    pub fn new(on_change: impl Fn(MediaSnapshot) + Send + Sync + 'static) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(MediaSnapshot::default()),
            callback: Box::new(on_change),
            stopping: AtomicBool::new(false),
            dirty: AtomicU8::new(0),
            initialized: AtomicBool::new(false),
            sender,
        });
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("still-media".into())
            .spawn(move || {
                if let Err(error) = worker(&state, receiver) {
                    let mut snapshot = MediaSnapshot::default();
                    snapshot.error = Some(error);
                    state.publish(snapshot);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn snapshot(&self) -> MediaSnapshot {
        self.shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    /// Queue an event-driven refresh; does not create a recurring timer.
    pub fn refresh(&self) {
        self.shared.notify(7);
    }
    fn command(&self, command: Command) -> Result<(), String> {
        if self.shared.stopping.load(Ordering::Acquire) {
            return Err("Media service stopped".into());
        }
        if !self.snapshot().available {
            return Err("No Windows media session is available".into());
        }
        self.shared
            .sender
            .send(command)
            .map_err(|_| "Media service is unavailable".into())
    }
    /// `Ok` means queued. The actual publisher's acknowledgement/error is emitted
    /// through `on_change`, rather than pretending the transport already changed.
    pub fn play_pause(&self) -> Result<(), String> {
        self.command(Command::PlayPause)
    }
    pub fn previous(&self) -> Result<(), String> {
        self.command(Command::Previous)
    }
    pub fn next(&self) -> Result<(), String> {
        self.command(Command::Next)
    }
    pub fn seek(&self, position_seconds: f64) -> Result<(), String> {
        if !position_seconds.is_finite() {
            return Err("Seek position must be finite".into());
        }
        self.command(Command::Seek(position_seconds.max(0.0)))
    }
}
impl Drop for MediaService {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        let _ = self.shared.sender.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// Wait only while a remote operation is outstanding. Completion wakes this channel;
// cancellation is checked on bounded waits so shutdown cannot hang behind a player.
// This is never entered while the service is idle.
macro_rules! wait {
    ($operation:expr,$shared:expr) => {{
        let operation = $operation.map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(1);
        operation
            .when(move |result| {
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            })
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        loop {
            if $shared.stopping.load(Ordering::Acquire)
                || started.elapsed() > Duration::from_secs(3)
            {
                let _ = operation.Cancel();
                break Err("Media operation cancelled or timed out".to_string());
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => {
                    result?;
                    break operation.GetResults().map_err(|e| e.to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break Err("Media operation completion channel closed".to_string()),
            }
        }
    }};
}
struct Apartment;
impl Apartment {
    fn new() -> Result<Self, String> {
        // SAFETY: this is a newly created worker with no pre-existing COM apartment.
        // Every WinRT interface is dropped before this guard calls RoUninitialize.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|e| e.to_string())?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() }
    }
}

struct SessionEvents {
    session: Session,
    metadata: i64,
    playback: i64,
    timeline: i64,
}
impl SessionEvents {
    fn new(session: Session, shared: &Arc<Shared>) -> Result<Self, String> {
        let state = shared.clone();
        let metadata = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                state.notify(3);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        let state = shared.clone();
        let playback = match session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            state.notify(1);
            Ok(())
        })) {
            Ok(token) => token,
            Err(e) => {
                let _ = session.RemoveMediaPropertiesChanged(metadata);
                return Err(e.to_string());
            }
        };
        let state = shared.clone();
        let timeline =
            match session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                state.notify(1);
                Ok(())
            })) {
                Ok(token) => token,
                Err(e) => {
                    let _ = session.RemoveMediaPropertiesChanged(metadata);
                    let _ = session.RemovePlaybackInfoChanged(playback);
                    return Err(e.to_string());
                }
            };
        Ok(Self {
            session,
            metadata,
            playback,
            timeline,
        })
    }
}
impl Drop for SessionEvents {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.metadata);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
        let _ = self.session.RemoveTimelinePropertiesChanged(self.timeline);
    }
}
struct ManagerEvents {
    manager: Manager,
    current: i64,
    sessions: i64,
}
impl ManagerEvents {
    fn new(manager: Manager, shared: &Arc<Shared>) -> Result<Self, String> {
        let state = shared.clone();
        let current = manager
            .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                state.notify(7);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        let state = shared.clone();
        let sessions = match manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            state.notify(7);
            Ok(())
        })) {
            Ok(t) => t,
            Err(e) => {
                let _ = manager.RemoveCurrentSessionChanged(current);
                return Err(e.to_string());
            }
        };
        Ok(Self {
            manager,
            current,
            sessions,
        })
    }
}
impl Drop for ManagerEvents {
    fn drop(&mut self) {
        let _ = self.manager.RemoveCurrentSessionChanged(self.current);
        let _ = self.manager.RemoveSessionsChanged(self.sessions);
    }
}

fn worker(shared: &Arc<Shared>, receiver: mpsc::Receiver<Command>) -> Result<(), String> {
    let _apartment = Apartment::new()?;
    let manager = wait!(Manager::RequestAsync(), shared)?;
    let _manager_events = ManagerEvents::new(manager.clone(), shared)?;
    let mut session: Option<SessionEvents> = None;
    shared.notify(7);
    while let Ok(command) = receiver.recv() {
        if shared.stopping.load(Ordering::Acquire) || matches!(command, Command::Shutdown) {
            break;
        }
        let result = (|| -> Result<(), String> {
            if matches!(command, Command::Refresh) {
                let flags = shared.dirty.swap(0, Ordering::AcqRel);
                let mut changed = false;
                if flags & 4 != 0 {
                    let current = manager.GetCurrentSession().ok();
                    if current.as_ref() != session.as_ref().map(|s| &s.session) {
                        session = None; // revoke the previous publisher's event tokens first
                        if let Some(current) = current {
                            session = Some(SessionEvents::new(current, shared)?);
                        }
                        changed = true;
                    }
                }
                if let Some(session) = session.as_ref() {
                    let snapshot = read_snapshot(
                        &session.session,
                        shared,
                        flags & 2 != 0 || changed,
                        changed,
                    )?;
                    shared.publish(snapshot);
                } else {
                    shared.publish(MediaSnapshot::default());
                }
            } else if let Some(session) = session.as_ref() {
                execute(&session.session, command, shared)?;
                let snapshot = read_snapshot(&session.session, shared, false, false)?;
                shared.publish(snapshot);
            } else {
                return Err("No Windows media session is available".into());
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut snapshot = shared
                .snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            snapshot.error = Some(error);
            shared.publish(snapshot);
        }
    }
    // Reverse declaration order revokes subscriptions and drops WinRT objects
    // before Apartment is uninitialized on its owning worker thread.
    Ok(())
}

fn execute(session: &Session, command: Command, shared: &Shared) -> Result<(), String> {
    let info = session.GetPlaybackInfo().map_err(|e| e.to_string())?;
    let controls = info.Controls().map_err(|e| e.to_string())?;
    let accepted = match command {
        Command::PlayPause => {
            if controls.IsPlayPauseToggleEnabled().unwrap_or(false) {
                wait!(session.TryTogglePlayPauseAsync(), shared)?
            } else if info.PlaybackStatus().map_err(|e| e.to_string())? == PlaybackStatus::Playing
                && controls.IsPauseEnabled().unwrap_or(false)
            {
                wait!(session.TryPauseAsync(), shared)?
            } else if controls.IsPlayEnabled().unwrap_or(false) {
                wait!(session.TryPlayAsync(), shared)?
            } else {
                return Err("The current player does not support play/pause".into());
            }
        }
        Command::Previous => {
            if !controls.IsPreviousEnabled().unwrap_or(false) {
                return Err("The current player does not support previous".into());
            }
            wait!(session.TrySkipPreviousAsync(), shared)?
        }
        Command::Next => {
            if !controls.IsNextEnabled().unwrap_or(false) {
                return Err("The current player does not support next".into());
            }
            wait!(session.TrySkipNextAsync(), shared)?
        }
        Command::Seek(seconds) => {
            if !controls.IsPlaybackPositionEnabled().unwrap_or(false) {
                return Err("The current player does not support seeking".into());
            }
            let timeline = session.GetTimelineProperties().map_err(|e| e.to_string())?;
            let start = timeline.StartTime().map_err(|e| e.to_string())?.Duration;
            let minimum = timeline.MinSeekTime().map_err(|e| e.to_string())?.Duration;
            let maximum = timeline
                .MaxSeekTime()
                .map_err(|e| e.to_string())?
                .Duration
                .max(minimum);
            let ticks = (seconds * 10_000_000.0).clamp(0.0, i64::MAX as f64) as i64;
            let target = start.saturating_add(ticks).clamp(minimum, maximum);
            wait!(session.TryChangePlaybackPositionAsync(target), shared)?
        }
        _ => return Ok(()),
    };
    if accepted {
        Ok(())
    } else {
        Err("The player rejected the media command".into())
    }
}

fn read_snapshot(
    session: &Session,
    shared: &Shared,
    metadata: bool,
    new_session: bool,
) -> Result<MediaSnapshot, String> {
    let mut snapshot = if new_session {
        MediaSnapshot::default()
    } else {
        shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    };
    snapshot.available = true;
    snapshot.error = None;
    snapshot.source = session
        .SourceAppUserModelId()
        .map_err(|e| e.to_string())?
        .to_string();
    let info = session.GetPlaybackInfo().map_err(|e| e.to_string())?;
    let controls = info.Controls().map_err(|e| e.to_string())?;
    let status = info.PlaybackStatus().map_err(|e| e.to_string())?;
    if status == PlaybackStatus::Closed {
        return Ok(MediaSnapshot::default());
    }
    snapshot.playing = status == PlaybackStatus::Playing;
    snapshot.can_play_pause = controls.IsPlayPauseToggleEnabled().unwrap_or(false)
        || if snapshot.playing {
            controls.IsPauseEnabled().unwrap_or(false)
        } else {
            controls.IsPlayEnabled().unwrap_or(false)
        };
    snapshot.can_previous = controls.IsPreviousEnabled().unwrap_or(false);
    snapshot.can_next = controls.IsNextEnabled().unwrap_or(false);
    snapshot.can_seek = controls.IsPlaybackPositionEnabled().unwrap_or(false);
    let timeline = session.GetTimelineProperties().map_err(|e| e.to_string())?;
    let start = timeline.StartTime().map_err(|e| e.to_string())?.Duration;
    let end = timeline.EndTime().map_err(|e| e.to_string())?.Duration;
    snapshot.duration_seconds = (end.saturating_sub(start).max(0) as f64) / 10_000_000.0;
    snapshot.position_seconds = ((timeline
        .Position()
        .map_err(|e| e.to_string())?
        .Duration
        .saturating_sub(start)
        .max(0) as f64)
        / 10_000_000.0)
        .min(snapshot.duration_seconds);
    if metadata {
        let properties = wait!(session.TryGetMediaPropertiesAsync(), shared)?;
        snapshot.title = properties.Title().map_err(|e| e.to_string())?.to_string();
        snapshot.artist = properties.Artist().map_err(|e| e.to_string())?.to_string();
        if snapshot.title.is_empty() {
            snapshot.title = "Untitled media".into();
        }
        snapshot.artwork = read_artwork(&properties, shared).ok().flatten();
    }
    Ok(snapshot)
}
fn read_artwork(
    properties: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
    shared: &Shared,
) -> Result<Option<Vec<u8>>, String> {
    let Ok(reference) = properties.Thumbnail() else {
        return Ok(None);
    };
    let stream = wait!(reference.OpenReadAsync(), shared)?;
    let result = (|| -> Result<Option<Vec<u8>>, String> {
        let size = stream.Size().map_err(|e| e.to_string())?;
        // Keep session-provided encoded artwork bounded; decode/resize on the UI
        // side if desired. No artwork is written to disk or transmitted anywhere.
        if size == 0 || size > 1024 * 1024 {
            return Ok(None);
        }
        let reader = DataReader::CreateDataReader(&stream).map_err(|e| e.to_string())?;
        let loaded = wait!(reader.LoadAsync(size as u32), shared)?;
        let mut bytes = vec![0; loaded as usize];
        reader.ReadBytes(&mut bytes).map_err(|e| e.to_string())?;
        let _ = reader.Close();
        Ok(Some(bytes))
    })();
    let _ = stream.Close();
    result
}
