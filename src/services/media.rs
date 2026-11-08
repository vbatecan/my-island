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
pub struct MediaPlayer {
    /// Opaque identity for one live session, never an application-wide identifier.
    pub id: String,
    pub name: String,
    pub playing: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MediaSnapshot {
    pub players: Vec<MediaPlayer>,
    /// Empty selects Automatic; otherwise one of `players`' opaque session IDs.
    pub selected_player: String,
    pub title: String,
    pub artist: String,
    pub album: String,
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
            players: Vec::new(),
            selected_player: String::new(),
            title: "Nothing playing".into(),
            artist: "Start media in a supported app".into(),
            album: String::new(),
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
    Select(String),
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
                    let snapshot = MediaSnapshot {
                        error: Some(error),
                        ..MediaSnapshot::default()
                    };
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
        self.with_snapshot(MediaSnapshot::clone)
    }
    /// Read selected metadata without cloning artwork or the entire player list.
    /// Keep the reader brief and do not re-enter the service while holding the lock.
    pub fn with_snapshot<R>(&self, read: impl FnOnce(&MediaSnapshot) -> R) -> R {
        let snapshot = self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        read(&snapshot)
    }
    /// Queue an event-driven refresh; does not create a recurring timer.
    pub fn refresh(&self) {
        self.shared.notify(7);
    }
    /// Select a live session, or pass an empty ID to follow Automatic selection.
    /// The worker revalidates the ID; vanished sessions fall back to Automatic.
    pub fn select_player(&self, id: String) -> Result<(), String> {
        if self.shared.stopping.load(Ordering::Acquire) {
            return Err("Media service stopped".into());
        }
        self.shared
            .sender
            .send(Command::Select(id))
            .map_err(|_| "Media service is unavailable".into())
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
            state.notify(5);
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

struct PlayerSession {
    session: Session,
    source: String,
    id: String,
    playback: i64,
}
impl PlayerSession {
    fn new(session: Session, id: String, shared: &Arc<Shared>) -> Result<Self, String> {
        let state = shared.clone();
        let playback = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                // A background player can start without becoming Windows' current
                // session. Re-arbitrate on every player's playback changes.
                state.notify(5);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            source: session
                .SourceAppUserModelId()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            session,
            id,
            playback,
        })
    }
}
impl Drop for PlayerSession {
    fn drop(&mut self) {
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
    }
}

fn choose_player(
    players: &[MediaPlayer],
    selected: &mut String,
    current: Option<usize>,
) -> Option<usize> {
    if !selected.is_empty() {
        if let Some(index) = players.iter().position(|p| &p.id == selected) {
            return Some(index);
        }
        selected.clear();
    }
    let current = current.filter(|&i| i < players.len());
    current
        .filter(|&i| players[i].playing)
        .or_else(|| players.iter().position(|p| p.playing))
        .or(current)
        .or_else(|| (!players.is_empty()).then_some(0))
}

fn player_name(source: &str) -> String {
    let basename = source.rsplit(['\\', '/']).next().unwrap_or(source);
    let lower = basename.to_ascii_lowercase();
    if lower.contains("spotify") {
        "Spotify".into()
    } else if lower == "msedge.exe" {
        "Microsoft Edge".into()
    } else if lower == "chrome.exe" {
        "Google Chrome".into()
    } else if lower == "firefox.exe" {
        "Firefox".into()
    } else {
        basename.trim_end_matches(".exe").to_owned()
    }
}

fn worker(shared: &Arc<Shared>, receiver: mpsc::Receiver<Command>) -> Result<(), String> {
    let _apartment = Apartment::new()?;
    let manager = wait!(Manager::RequestAsync(), shared)?;
    let _manager_events = ManagerEvents::new(manager.clone(), shared)?;
    let mut session: Option<SessionEvents> = None;
    let mut catalog: Vec<PlayerSession> = Vec::new();
    let mut next_id = 1u64;
    let mut selected = String::new();
    shared.notify(7);
    while let Ok(command) = receiver.recv() {
        if shared.stopping.load(Ordering::Acquire) || matches!(command, Command::Shutdown) {
            break;
        }
        let result = (|| -> Result<(), String> {
            if matches!(command, Command::Refresh | Command::Select(_)) {
                let flags = if let Command::Select(id) = command {
                    selected = id;
                    7
                } else {
                    shared.dirty.swap(0, Ordering::AcqRel)
                };
                let mut changed = false;
                let mut players = shared
                    .snapshot
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .players
                    .clone();
                if flags & 4 != 0 {
                    let available = manager.GetSessions().map_err(|e| e.to_string())?;
                    let mut candidates = Vec::new();
                    for index in 0..available.Size().map_err(|e| e.to_string())? {
                        let candidate = available.GetAt(index).map_err(|e| e.to_string())?;
                        let source = candidate
                            .SourceAppUserModelId()
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        candidates.push((candidate, source));
                    }
                    let old_sources: Vec<_> = catalog.iter().map(|p| p.source.clone()).collect();
                    let new_sources: Vec<_> = candidates
                        .iter()
                        .map(|(_, source)| source.clone())
                        .collect();
                    let mut next = Vec::new();
                    let mut next_players = Vec::new();
                    for (candidate, source) in candidates {
                        let status = candidate
                            .GetPlaybackInfo()
                            .and_then(|info| info.PlaybackStatus());
                        let Ok(status) = status else {
                            continue;
                        };
                        if status == PlaybackStatus::Closed {
                            continue;
                        }
                        let exact = catalog.iter().position(|p| p.session == candidate);
                        // Windows can replace every session wrapper on a roster
                        // update. A unique app source is unambiguous; preserve its
                        // ID but rebind subscriptions to the replacement wrapper.
                        // For multiple same-source sessions never guess identity.
                        let unique = !source.is_empty()
                            && old_sources.iter().filter(|s| *s == &source).count() == 1
                            && new_sources.iter().filter(|s| *s == &source).count() == 1;
                        let replaced = unique
                            .then(|| catalog.iter().position(|p| p.source == source))
                            .flatten();
                        let entry = if let Some(old) = exact.or(replaced) {
                            let old = catalog.remove(old);
                            if exact.is_some() {
                                old
                            } else {
                                PlayerSession::new(candidate.clone(), old.id.clone(), shared)?
                            }
                        } else {
                            let entry =
                                PlayerSession::new(candidate.clone(), next_id.to_string(), shared)?;
                            next_id = next_id
                                .checked_add(1)
                                .ok_or("Session identity limit exceeded")?;
                            entry
                        };
                        next_players.push(MediaPlayer {
                            id: entry.id.clone(),
                            name: player_name(&source),
                            playing: status == PlaybackStatus::Playing,
                        });
                        next.push(entry);
                    }
                    // Drops subscriptions of removed sessions; surviving IDs remain stable.
                    catalog = next;
                    for i in 0..next_players.len() {
                        let original = next_players[i].name.clone();
                        if next_players
                            .iter()
                            .filter(|p| {
                                p.name == original || p.name.starts_with(&format!("{original} · "))
                            })
                            .count()
                            > 1
                        {
                            next_players[i].name = format!("{} · {}", original, next_players[i].id);
                        }
                    }
                    players = next_players;
                    let windows_current = manager.GetCurrentSession().ok();
                    let current_index = catalog
                        .iter()
                        .position(|p| Some(&p.session) == windows_current.as_ref());
                    let chosen = choose_player(&players, &mut selected, current_index)
                        .map(|i| catalog[i].session.clone());
                    if chosen.as_ref() != session.as_ref().map(|s| &s.session) {
                        session = None; // revoke the previous publisher's event tokens first
                        if let Some(chosen) = chosen {
                            session = Some(SessionEvents::new(chosen, shared)?);
                        }
                        changed = true;
                    }
                }
                let mut snapshot = if let Some(session) = session.as_ref() {
                    read_snapshot(&session.session, shared, flags & 2 != 0 || changed, changed)?
                } else {
                    MediaSnapshot::default()
                };
                snapshot.players = players;
                snapshot.selected_player = selected.clone();
                shared.publish(snapshot);
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
        snapshot.album = properties
            .AlbumTitle()
            .map_err(|e| e.to_string())?
            .to_string();
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

#[cfg(test)]
mod tests {
    use super::*;
    fn players() -> Vec<MediaPlayer> {
        vec![
            MediaPlayer {
                id: "1".into(),
                name: "Browser".into(),
                playing: false,
            },
            MediaPlayer {
                id: "2".into(),
                name: "Music".into(),
                playing: true,
            },
            MediaPlayer {
                id: "3".into(),
                name: "Browser".into(),
                playing: true,
            },
        ]
    }
    #[test]
    fn automatic_follows_playback_and_windows_current_priority() {
        let players = players();
        let mut selected = String::new();
        assert_eq!(choose_player(&players, &mut selected, Some(0)), Some(1));
        assert_eq!(choose_player(&players, &mut selected, Some(2)), Some(2));
        assert_eq!(choose_player(&players, &mut selected, None), Some(1));
        assert!(selected.is_empty());
    }
    #[test]
    fn explicit_selection_is_session_specific_even_with_same_player_name() {
        let players = players();
        let mut selected = "1".to_owned();
        assert_eq!(choose_player(&players, &mut selected, Some(2)), Some(0));
        assert_eq!(selected, "1");
        selected = "3".into();
        assert_eq!(choose_player(&players, &mut selected, Some(0)), Some(2));
    }
    #[test]
    fn removed_player_falls_back_and_empty_catalog_is_safe() {
        let mut selected = "removed".to_owned();
        assert_eq!(choose_player(&players(), &mut selected, Some(0)), Some(1));
        assert!(selected.is_empty());
        assert_eq!(choose_player(&[], &mut selected, Some(20)), None);
        let mut paused = players();
        for player in &mut paused {
            player.playing = false;
        }
        assert_eq!(choose_player(&paused, &mut selected, Some(2)), Some(2));
        assert_eq!(choose_player(&paused, &mut selected, None), Some(0));
    }
}
