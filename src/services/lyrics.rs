//! Optional, metadata-only LRCLIB lookups and line-synchronized lyrics.
//!
//! Construction never accesses the network. Requests are serialized on one sleeping
//! worker, bounded in time and size, and cancelled when a newer request arrives.
//! Nothing is recorded, written to disk, or sent except the supplied track signature.
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Deserialize;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_LINES: usize = 4096;
const MAX_DOCUMENT_TEXT_BYTES: usize = 128 * 1024;
const CACHE_CAPACITY: usize = 16;
const MAX_CACHE_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrackKey {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
}

impl TrackKey {
    /// Unknown durations and incomplete metadata are deliberately not searched:
    /// accepting a loose title match can display another recording's lyrics.
    pub fn new(title: &str, artist: &str, album: &str, duration_seconds: f64) -> Option<Self> {
        if title.trim().is_empty()
            || artist.trim().is_empty()
            || [title, artist, album].iter().any(|s| s.len() > 512)
            || !duration_seconds.is_finite()
            || !(1.0..=3600.0).contains(&duration_seconds)
        {
            return None;
        }
        Some(Self {
            title: title.trim().to_owned(),
            artist: artist.trim().to_owned(),
            album: album.trim().to_owned(),
            duration_seconds: duration_seconds.round() as u32,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LyricsLine {
    pub timestamp_seconds: f64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LyricsContent {
    Synced(Vec<LyricsLine>),
    Plain(Vec<String>),
    Instrumental,
}

impl LyricsContent {
    /// Binary search remains correct after seeking backwards as well as forwards.
    /// Before the first timestamp there is no active line; empty timed lines clear it.
    pub fn active_line(&self, position_seconds: f64) -> Option<usize> {
        let Self::Synced(lines) = self else {
            return None;
        };
        if !position_seconds.is_finite() {
            return None;
        }
        lines
            .partition_point(|line| line.timestamp_seconds <= position_seconds)
            .checked_sub(1)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LyricsStatus {
    Found(Arc<LyricsContent>),
    NotFound,
    Unavailable(String),
}

#[derive(Clone, Debug)]
pub struct LyricsEvent {
    pub generation: u64,
    pub track: TrackKey,
    pub status: LyricsStatus,
}

struct Request {
    serial: u64,
    generation: u64,
    track: TrackKey,
    refresh: bool,
}
enum Command {
    Lookup(Request),
    Wake,
    ClearCache,
}
struct Cancellation {
    serial: AtomicU64,
    stopping: AtomicBool,
}
impl Cancellation {
    fn cancelled(&self, serial: u64) -> bool {
        self.stopping.load(Ordering::Acquire) || self.serial.load(Ordering::Acquire) != serial
    }
}

pub struct LyricsService {
    sender: mpsc::Sender<Command>,
    cancellation: Arc<Cancellation>,
    worker: Option<JoinHandle<()>>,
}

impl LyricsService {
    /// The caller controls permission and panel visibility. Only call `request`
    /// when the optional online setting is enabled and lyrics are actually needed.
    pub fn new(on_change: impl Fn(LyricsEvent) + Send + Sync + 'static) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let cancellation = Arc::new(Cancellation {
            serial: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
        });
        let state = cancellation.clone();
        let worker = thread::Builder::new()
            .name("still-lyrics".into())
            .spawn(move || run_worker(receiver, &state, on_change, NativeTransport))
            .map_err(|e| format!("Cannot start lyrics lookup: {e}"))?;
        Ok(Self {
            sender,
            cancellation,
            worker: Some(worker),
        })
    }

    pub fn request(&self, generation: u64, track: TrackKey) {
        self.send(generation, track, false);
    }

    /// Deliberate Retry bypasses a cached miss/result, while still honoring the
    /// provider's rate limit and cancelling any previous in-flight request.
    pub fn request_refresh(&self, generation: u64, track: TrackKey) {
        self.send(generation, track, true);
    }

    fn send(&self, generation: u64, track: TrackKey, refresh: bool) {
        let serial = self.cancellation.serial.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.sender.send(Command::Lookup(Request {
            serial,
            generation,
            track,
            refresh,
        }));
    }

    /// Cancels pending work and suppresses its callback. A previously queued UI
    /// callback can still arrive, so consumers must also compare `generation`.
    pub fn cancel(&self) {
        self.cancellation.serial.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.send(Command::Wake);
    }

    /// Drop cached lyric allocations without resetting provider rate limits.
    /// This is queued after cancellation and does not block the GUI thread.
    pub fn clear_cache(&self) {
        let _ = self.sender.send(Command::ClearCache);
    }
}

impl Drop for LyricsService {
    fn drop(&mut self) {
        self.cancellation.stopping.store(true, Ordering::Release);
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker<T: Transport>(
    receiver: mpsc::Receiver<Command>,
    cancellation: &Cancellation,
    on_change: impl Fn(LyricsEvent),
    transport: T,
) {
    let mut cache = LyricsCache::default();
    let mut client = LookupClient::new(transport);
    while let Ok(command) = receiver.recv() {
        if cancellation.stopping.load(Ordering::Acquire) {
            break;
        }
        let mut latest = match command {
            Command::Lookup(request) => Some(request),
            Command::Wake => None,
            Command::ClearCache => {
                cache = LyricsCache::default();
                None
            }
        };
        // Rapid session changes can queue many commands while a request is in flight.
        for command in receiver.try_iter() {
            if matches!(command, Command::ClearCache) {
                cache = LyricsCache::default();
                continue;
            }
            if let Command::Lookup(request) = command
                && latest
                    .as_ref()
                    .is_none_or(|previous| request.serial > previous.serial)
            {
                latest = Some(request);
            }
        }
        // A stop wake may have been consumed while coalescing commands. Check
        // again before blocking on recv, otherwise Drop could wait forever.
        if cancellation.stopping.load(Ordering::Acquire) {
            break;
        }
        let Some(request) = latest.filter(|r| !cancellation.cancelled(r.serial)) else {
            continue;
        };
        if request.refresh {
            cache.remove(&request.track);
        }
        let status = if let Some(status) = cache.get(&request.track) {
            status
        } else {
            let Some(status) = client.lookup(&request.track, cancellation, request.serial) else {
                continue;
            };
            if !matches!(status, LyricsStatus::Unavailable(_)) {
                cache.insert(request.track.clone(), status.clone());
            }
            status
        };
        if !cancellation.cancelled(request.serial) {
            on_change(LyricsEvent {
                generation: request.generation,
                track: request.track,
                status,
            });
        }
    }
}

#[derive(Default)]
struct LyricsCache {
    entries: VecDeque<(TrackKey, LyricsStatus)>,
    bytes: usize,
}
impl LyricsCache {
    fn get(&mut self, key: &TrackKey) -> Option<LyricsStatus> {
        let index = self.entries.iter().position(|(cached, _)| cached == key)?;
        let entry = self.entries.remove(index)?;
        let status = entry.1.clone();
        self.entries.push_back(entry);
        Some(status)
    }
    fn remove(&mut self, key: &TrackKey) {
        if let Some(index) = self.entries.iter().position(|(cached, _)| cached == key)
            && let Some((key, status)) = self.entries.remove(index)
        {
            self.bytes -= cache_entry_bytes(&key, &status);
        }
    }
    fn insert(&mut self, key: TrackKey, status: LyricsStatus) {
        self.remove(&key);
        let entry_bytes = cache_entry_bytes(&key, &status);
        // Reserve space for the ring allocation as well as every retained string
        // and lyric line; the budget deliberately overcounts shared Arc payloads.
        let budget =
            MAX_CACHE_BYTES - CACHE_CAPACITY * std::mem::size_of::<(TrackKey, LyricsStatus)>();
        while !self.entries.is_empty()
            && (self.entries.len() >= CACHE_CAPACITY || self.bytes + entry_bytes > budget)
        {
            let (old_key, old_status) = self.entries.pop_front().expect("cache is not empty");
            self.bytes -= cache_entry_bytes(&old_key, &old_status);
        }
        if entry_bytes <= budget {
            self.bytes += entry_bytes;
            self.entries.push_back((key, status));
        }
    }
}

fn cache_entry_bytes(key: &TrackKey, status: &LyricsStatus) -> usize {
    let content_bytes = match status {
        LyricsStatus::Found(content) => {
            let allocation = match &**content {
                LyricsContent::Synced(lines) => {
                    lines.capacity() * std::mem::size_of::<LyricsLine>()
                        + lines.iter().map(|line| line.text.capacity()).sum::<usize>()
                }
                LyricsContent::Plain(lines) => {
                    lines.capacity() * std::mem::size_of::<String>()
                        + lines.iter().map(String::capacity).sum::<usize>()
                }
                LyricsContent::Instrumental => 0,
            };
            allocation + std::mem::size_of::<LyricsContent>() + 2 * std::mem::size_of::<usize>()
        }
        LyricsStatus::Unavailable(error) => error.capacity(),
        LyricsStatus::NotFound => 0,
    };
    std::mem::size_of::<(TrackKey, LyricsStatus)>()
        + key.title.capacity()
        + key.artist.capacity()
        + key.album.capacity()
        + content_bytes
}

/// Parse ordinary LRC including repeated timestamps, BOM, global signed offsets,
/// fractional seconds, Unicode text and optional hour timestamps. Metadata tags
/// and malformed lines are ignored. Empty timestamped lines remain meaningful.
pub fn parse_lrc(input: &str) -> Vec<LyricsLine> {
    if input.len() > MAX_RESPONSE_BYTES {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut offset_ms = 0_i64;
    let mut text_bytes = 0;
    for raw in input.trim_start_matches('\u{feff}').lines() {
        let raw = raw.trim();
        if let Some(offset) = raw
            .strip_prefix("[offset:")
            .and_then(|s| s.strip_suffix(']'))
        {
            if let Ok(value) = offset.trim().parse::<i64>() {
                offset_ms = value.clamp(-3_600_000, 3_600_000);
            }
            continue;
        }
        let mut rest = raw;
        let mut timestamps = Vec::new();
        while let Some(tag) = rest.strip_prefix('[').and_then(|s| s.split_once(']')) {
            let Some(seconds) = parse_timestamp(tag.0) else {
                break;
            };
            timestamps.push(seconds);
            rest = tag.1;
            if timestamps.len() >= MAX_LINES {
                break;
            }
        }
        for timestamp_seconds in timestamps {
            if lines.len() == MAX_LINES {
                break;
            }
            let text = rest.trim().chars().take(512).collect::<String>();
            if text_bytes + text.len() > MAX_DOCUMENT_TEXT_BYTES {
                continue;
            }
            text_bytes += text.len();
            lines.push(LyricsLine {
                timestamp_seconds,
                // Limit a maliciously long line without cutting a UTF-8 code point.
                text,
            });
        }
    }
    for line in &mut lines {
        // A positive LRC offset advances the lyrics relative to the audio.
        line.timestamp_seconds = (line.timestamp_seconds - offset_ms as f64 / 1000.0).max(0.0);
    }
    lines.sort_by(|a, b| a.timestamp_seconds.total_cmp(&b.timestamp_seconds));
    // Duplicate identical entries are common in downloaded files.
    lines.dedup_by(|a, b| a.timestamp_seconds == b.timestamp_seconds && a.text == b.text);
    lines
}

fn parse_timestamp(tag: &str) -> Option<f64> {
    let parts: Vec<_> = tag.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => ("0", *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    if ![hours, minutes]
        .iter()
        .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        || seconds.is_empty()
        || !seconds.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    let hours = hours.parse::<u32>().ok()?;
    let minutes = minutes.parse::<u32>().ok()?;
    if parts.len() == 3 && minutes >= 60 {
        return None;
    }
    let seconds = seconds.parse::<f64>().ok()?;
    let total = hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds;
    (seconds.is_finite() && seconds < 60.0 && total <= 86_400.0).then_some(total)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LyricsRecord {
    track_name: String,
    artist_name: String,
    #[serde(default)]
    album_name: String,
    duration: f64,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl LyricsRecord {
    fn matches(&self, key: &TrackKey) -> bool {
        normalize(&self.track_name) == normalize(&key.title)
            && normalize(&self.artist_name) == normalize(&key.artist)
            && self.duration.is_finite()
            && (self.duration - key.duration_seconds as f64).abs() <= 2.0
    }

    fn into_content(self) -> Option<LyricsContent> {
        if self.instrumental {
            return Some(LyricsContent::Instrumental);
        }
        if let Some(synced) = self.synced_lyrics {
            let parsed = parse_lrc(&synced);
            if parsed.iter().any(|line| !line.text.is_empty()) {
                return Some(LyricsContent::Synced(parsed));
            }
        }
        self.plain_lyrics.and_then(|text| {
            let mut text_bytes = 0;
            let lines: Vec<_> = text
                .lines()
                .map(|line| line.trim().chars().take(512).collect::<String>())
                .filter(|line| !line.is_empty())
                .take_while(|line| {
                    text_bytes += line.len();
                    text_bytes <= MAX_DOCUMENT_TEXT_BYTES
                })
                .take(MAX_LINES)
                .collect();
            (!lines.is_empty()).then_some(LyricsContent::Plain(lines))
        })
    }
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn encode_query(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 15) as usize] as char);
        }
    }
    encoded
}

struct HttpResponse {
    status: u32,
    retry_after: Option<Duration>,
    json: bool,
    body: Vec<u8>,
}
trait Transport {
    fn get(
        &mut self,
        path: &str,
        cancellation: &Cancellation,
        serial: u64,
    ) -> Result<HttpResponse, String>;
}
struct NativeTransport;
struct LookupClient<T> {
    transport: T,
    next_request: Instant,
    suspended: bool,
}
impl<T: Transport> LookupClient<T> {
    fn new(transport: T) -> Self {
        Self {
            transport,
            next_request: Instant::now(),
            suspended: false,
        }
    }

    fn lookup(
        &mut self,
        key: &TrackKey,
        cancel: &Cancellation,
        serial: u64,
    ) -> Option<LyricsStatus> {
        if cancel.cancelled(serial) {
            return None;
        }
        let exact = format!(
            "/api/get?track_name={}&artist_name={}&album_name={}&duration={}",
            encode_query(&key.title),
            encode_query(&key.artist),
            encode_query(&key.album),
            key.duration_seconds,
        );
        // No automatic retry while rate limited: a later deliberate request may retry.
        if self.suspended
            || self.next_request.saturating_duration_since(Instant::now()) > Duration::from_secs(1)
        {
            return Some(LyricsStatus::Unavailable(
                "Lyrics provider is busy; try again later".into(),
            ));
        }
        let response = self.fetch(&exact, cancel, serial)?;
        match response {
            Err(error) => return Some(LyricsStatus::Unavailable(error)),
            Ok(response) if response.status == 404 => (),
            Ok(response) if response.status == 200 => {
                if !response.json {
                    return Some(LyricsStatus::Unavailable(
                        "Lyrics provider returned a non-JSON response".into(),
                    ));
                }
                match serde_json::from_slice::<LyricsRecord>(&response.body) {
                    Ok(record) if record.matches(key) => {
                        if let Some(content) = record.into_content() {
                            return Some(LyricsStatus::Found(Arc::new(content)));
                        }
                    }
                    Ok(_) => (),
                    Err(_) => {
                        return Some(LyricsStatus::Unavailable(
                            "Lyrics provider returned invalid data".into(),
                        ));
                    }
                }
            }
            Ok(response) => {
                return Some(LyricsStatus::Unavailable(format!(
                    "Lyrics provider returned HTTP {}",
                    response.status
                )));
            }
        }
        let search = format!(
            "/api/search?track_name={}&artist_name={}",
            encode_query(&key.title),
            encode_query(&key.artist),
        );
        let response = match self.fetch(&search, cancel, serial)? {
            Ok(response) => response,
            Err(error) => return Some(LyricsStatus::Unavailable(error)),
        };
        if response.status != 200 || !response.json {
            return Some(LyricsStatus::Unavailable(format!(
                "Lyrics search returned HTTP {}",
                response.status
            )));
        }
        let Ok(mut records) = serde_json::from_slice::<Vec<LyricsRecord>>(&response.body) else {
            return Some(LyricsStatus::Unavailable(
                "Lyrics provider returned invalid data".into(),
            ));
        };
        records.retain(|record| record.matches(key));
        // Prefer the actual album and then synced lyrics, but never strip a version,
        // remix, live or cover suffix from a title to manufacture a match.
        records.sort_by_key(|r| {
            (
                normalize(&r.album_name) != normalize(&key.album),
                r.synced_lyrics.as_ref().is_none_or(|s| s.trim().is_empty()),
            )
        });
        Some(
            records
                .into_iter()
                .find_map(LyricsRecord::into_content)
                .map(|content| LyricsStatus::Found(Arc::new(content)))
                .unwrap_or(LyricsStatus::NotFound),
        )
    }

    fn fetch(
        &mut self,
        path: &str,
        cancel: &Cancellation,
        serial: u64,
    ) -> Option<Result<HttpResponse, String>> {
        // This short, cancellable throttle exists only while an explicit lookup is
        // active; the service's idle path blocks on its command channel indefinitely.
        while Instant::now() < self.next_request {
            if cancel.cancelled(serial) {
                return None;
            }
            thread::sleep(
                self.next_request
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            );
        }
        if cancel.cancelled(serial) {
            return None;
        }
        let response = self.transport.get(path, cancel, serial);
        self.next_request = Instant::now() + Duration::from_millis(250);
        if let Ok(response) = &response
            && matches!(response.status, 429 | 503)
        {
            let delay = response.retry_after.unwrap_or(Duration::from_secs(60));
            let jitter = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_millis()
                % 301;
            if let Some(next) = Instant::now()
                .checked_add(delay)
                .and_then(|next| next.checked_add(Duration::from_millis(jitter as u64)))
            {
                self.next_request = next;
            } else {
                // Never retry before an unrepresentably large Retry-After value.
                self.suspended = true;
            }
        }
        if cancel.cancelled(serial) {
            None
        } else {
            Some(response)
        }
    }
}

#[cfg(not(windows))]
impl Transport for NativeTransport {
    fn get(&mut self, _: &str, _: &Cancellation, _: u64) -> Result<HttpResponse, String> {
        Err("Online lyrics are currently available on Windows".into())
    }
}

#[cfg(windows)]
mod http {
    use super::*;
    use std::{ffi::c_void, ptr, sync::Mutex};
    use windows::{
        Win32::Networking::WinHttp::*,
        core::{PCWSTR, w},
    };

    /// Owned handles never escape the request worker. Asynchronous request closing
    /// is supported by WinHTTP and cancels outstanding operations safely.
    struct Handle(*mut c_void);
    impl Handle {
        fn new(raw: *mut c_void) -> Result<Self, String> {
            if raw.is_null() {
                Err("Cannot initialize the Windows HTTPS connection".into())
            } else {
                Ok(Self(raw))
            }
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: this RAII owner closes its valid handle exactly once. Parent
            // handles are dropped after child requests; no native API uses it again.
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }

    enum Event {
        Sent,
        Headers,
        Read(Vec<u8>),
        Error,
    }
    struct CallbackState {
        sender: mpsc::Sender<Event>,
        // Heap storage stays alive until HANDLE_CLOSING, including cancellation
        // with a pending native read. Only one read is ever outstanding.
        buffer: Mutex<Box<[u8; 8192]>>,
    }

    unsafe extern "system" fn callback(
        _: *mut c_void,
        context: usize,
        status: u32,
        information: *mut c_void,
        length: u32,
    ) {
        if context == 0 {
            return;
        }
        let raw = context as *const CallbackState;
        if status == WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING {
            // SAFETY: exactly one Arc was transferred to this handle. WinHTTP
            // guarantees HANDLE_CLOSING is its last callback, so reclaim it here.
            drop(unsafe { Arc::from_raw(raw) });
            return;
        }
        // SAFETY: the native-owned Arc remains alive until the final callback.
        // A temporary strong reference also protects callbacks already in flight.
        unsafe { Arc::increment_strong_count(raw) };
        let state = unsafe { Arc::from_raw(raw) };
        let event = match status {
            WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE => Event::Sent,
            WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE => Event::Headers,
            WINHTTP_CALLBACK_STATUS_READ_COMPLETE if length <= 8192 => {
                let bytes = if length == 0 {
                    Vec::new()
                } else if information.is_null() {
                    let _ = state.sender.send(Event::Error);
                    return;
                } else {
                    // SAFETY: on READ_COMPLETE WinHTTP supplies the completed read
                    // buffer with this exact length; no next read starts yet.
                    unsafe { std::slice::from_raw_parts(information.cast::<u8>(), length as usize) }
                        .to_vec()
                };
                Event::Read(bytes)
            }
            WINHTTP_CALLBACK_STATUS_REQUEST_ERROR => Event::Error,
            _ => return,
        };
        let _ = state.sender.send(event);
    }

    fn wait_event(
        events: &mpsc::Receiver<Event>,
        deadline: Instant,
        cancellation: &Cancellation,
        serial: u64,
    ) -> Result<Event, String> {
        loop {
            if cancellation.cancelled(serial) {
                return Err("Lyrics lookup cancelled".into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Lyrics lookup timed out".into());
            }
            match events.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(Event::Error) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Lyrics provider could not be reached".into());
                }
                Ok(event) => return Ok(event),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
        }
    }

    fn header(request: &Handle, kind: u32) -> Option<String> {
        let mut buffer = [0_u16; 256];
        let mut bytes = std::mem::size_of_val(&buffer) as u32;
        // SAFETY: the writable buffer is correctly sized in bytes, the request is
        // valid and headers have completed. Missing/oversized headers are ignored.
        unsafe {
            WinHttpQueryHeaders(
                request.0,
                kind,
                PCWSTR::null(),
                Some(buffer.as_mut_ptr().cast()),
                &mut bytes,
                ptr::null_mut(),
            )
            .ok()?;
        }
        Some(
            String::from_utf16_lossy(&buffer[..(bytes as usize / 2).min(buffer.len())])
                .trim_end_matches('\0')
                .to_owned(),
        )
    }

    impl Transport for NativeTransport {
        fn get(
            &mut self,
            path: &str,
            cancellation: &Cancellation,
            serial: u64,
        ) -> Result<HttpResponse, String> {
            let deadline = Instant::now() + REQUEST_TIMEOUT;
            let agent: Vec<u16> = concat!(
                "StillIsland/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/vbatecan/my-island)"
            )
            .encode_utf16()
            .chain(Some(0))
            .collect();
            let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            // SAFETY: all strings are live and NUL terminated, flags select an
            // asynchronous TLS connection with system certificate verification.
            // Handles are kept on this thread and closed in reverse ownership order.
            unsafe {
                let session = Handle::new(WinHttpOpen(
                    PCWSTR(agent.as_ptr()),
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    WINHTTP_FLAG_ASYNC | WINHTTP_FLAG_SECURE_DEFAULTS,
                ))?;
                WinHttpSetTimeouts(session.0, 2000, 2000, 2000, 2000)
                    .map_err(|_| "Cannot set HTTPS timeouts")?;
                let connection = Handle::new(WinHttpConnect(session.0, w!("lrclib.net"), 443, 0))?;
                let request = Handle::new(WinHttpOpenRequest(
                    connection.0,
                    w!("GET"),
                    PCWSTR(path.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    ptr::null(),
                    WINHTTP_FLAG_SECURE,
                ))?;
                // Never forward track metadata to a redirected host.
                WinHttpSetOption(
                    Some(request.0.cast_const()),
                    WINHTTP_OPTION_REDIRECT_POLICY,
                    Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
                )
                .map_err(|_| "Cannot configure HTTPS redirects")?;
                let (sender, events) = mpsc::channel();
                let state = Arc::new(CallbackState {
                    sender,
                    buffer: Mutex::new(Box::new([0; 8192])),
                });
                let raw = Arc::into_raw(state.clone());
                let context = raw as usize;
                if WinHttpSetOption(
                    Some(request.0.cast_const()),
                    WINHTTP_OPTION_CONTEXT_VALUE,
                    Some(&context.to_ne_bytes()),
                )
                .is_err()
                {
                    drop(Arc::from_raw(raw));
                    return Err("Cannot configure HTTPS context".into());
                }
                let flags = WINHTTP_CALLBACK_FLAG_SENDREQUEST_COMPLETE
                    | WINHTTP_CALLBACK_FLAG_HEADERS_AVAILABLE
                    | WINHTTP_CALLBACK_FLAG_READ_COMPLETE
                    | WINHTTP_CALLBACK_FLAG_REQUEST_ERROR
                    | WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING;
                let previous = WinHttpSetStatusCallback(request.0, Some(callback), flags, 0);
                if previous.is_some_and(|function| function as usize == usize::MAX) {
                    drop(Arc::from_raw(raw));
                    return Err("Cannot configure HTTPS callbacks".into());
                }
                WinHttpAddRequestHeaders(
                    request.0,
                    &"Accept: application/json\r\n"
                        .encode_utf16()
                        .collect::<Vec<_>>(),
                    WINHTTP_ADDREQ_FLAG_ADD,
                )
                .map_err(|_| "Cannot configure HTTPS headers")?;
                WinHttpSendRequest(request.0, None, None, 0, 0, context)
                    .map_err(|_| "Lyrics request failed")?;
                if !matches!(
                    wait_event(&events, deadline, cancellation, serial)?,
                    Event::Sent
                ) {
                    return Err("Unexpected HTTPS send response".into());
                }
                WinHttpReceiveResponse(request.0, ptr::null_mut())
                    .map_err(|_| "Lyrics response failed")?;
                if !matches!(
                    wait_event(&events, deadline, cancellation, serial)?,
                    Event::Headers
                ) {
                    return Err("Unexpected HTTPS headers".into());
                }
                let status = header(&request, WINHTTP_QUERY_STATUS_CODE)
                    .and_then(|s| s.parse().ok())
                    .ok_or("Missing HTTPS status")?;
                let json = header(&request, WINHTTP_QUERY_CONTENT_TYPE).is_some_and(|s| {
                    s.split(';')
                        .next()
                        .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"))
                });
                let retry_after = header(&request, WINHTTP_QUERY_RETRY_AFTER).map(|s| {
                    // Numeric Retry-After is the provider's documented format. An
                    // unrecognized value suspends retries for this service instance.
                    Duration::from_secs(s.trim().parse::<u64>().unwrap_or(u64::MAX))
                });
                let mut body = Vec::new();
                if status == 200 {
                    loop {
                        if Instant::now() >= deadline || cancellation.cancelled(serial) {
                            return Err("Lyrics lookup timed out or was cancelled".into());
                        }
                        let buffer = state
                            .buffer
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .as_mut_ptr();
                        WinHttpReadData(request.0, buffer.cast(), 8192, ptr::null_mut())
                            .map_err(|_| "Lyrics response read failed")?;
                        let Event::Read(bytes) =
                            wait_event(&events, deadline, cancellation, serial)?
                        else {
                            return Err("Unexpected HTTPS read response".into());
                        };
                        if bytes.is_empty() {
                            break;
                        }
                        if body.len() + bytes.len() > MAX_RESPONSE_BYTES {
                            return Err("Lyrics response exceeded its size limit".into());
                        }
                        body.extend_from_slice(&bytes);
                    }
                }
                Ok(HttpResponse {
                    status,
                    retry_after,
                    json,
                    body,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackKey {
        TrackKey::new("Quiet Song", "Test Artist", "Test Album", 120.0).unwrap()
    }
    fn cancellation() -> Cancellation {
        Cancellation {
            serial: AtomicU64::new(1),
            stopping: AtomicBool::new(false),
        }
    }
    fn response(status: u32, json: &str) -> HttpResponse {
        HttpResponse {
            status,
            retry_after: None,
            json: true,
            body: json.as_bytes().to_vec(),
        }
    }
    fn record(title: &str, artist: &str, duration: f64, lyrics: serde_json::Value) -> String {
        serde_json::json!({"trackName":title,"artistName":artist,"albumName":"Test Album","duration":duration,"plainLyrics":lyrics,"syncedLyrics":null}).to_string()
    }
    fn repeated_timestamp_fixture() -> String {
        let timestamps = (0..MAX_LINES)
            .map(|index| format!("[{:02}:{:02}]", index / 60, index % 60))
            .collect::<String>();
        format!("{timestamps}{}", "🌙".repeat(512))
    }
    struct FixtureTransport {
        responses: VecDeque<HttpResponse>,
        paths: Vec<String>,
    }
    impl Transport for FixtureTransport {
        fn get(&mut self, path: &str, _: &Cancellation, _: u64) -> Result<HttpResponse, String> {
            self.paths.push(path.to_owned());
            Ok(self
                .responses
                .pop_front()
                .expect("unexpected network request"))
        }
    }
    fn client(responses: Vec<HttpResponse>) -> LookupClient<FixtureTransport> {
        LookupClient::new(FixtureTransport {
            responses: responses.into(),
            paths: Vec::new(),
        })
    }

    #[test]
    fn lrc_multiple_timestamps_unicode_and_global_offset() {
        let parsed = parse_lrc(
            "\u{feff}[ar:Test]\n[00:02.50][00:03.125]你好 🌙\n[offset:+500]\n[00:05.000]\n[bad]ignored",
        );
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].timestamp_seconds, 2.0);
        assert_eq!(parsed[1].timestamp_seconds, 2.625);
        assert_eq!(parsed[0].text, "你好 🌙");
        assert!(parsed[2].text.is_empty());
    }
    #[test]
    fn lrc_malformed_sorting_dedup_and_negative_offset() {
        let lines = parse_lrc(
            "[00:02]second\n[00:01]first\n[00:01]first\n[00:99]bad\n[00:NaN]bad\n[00:-2]bad\n[offset:-250]",
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].timestamp_seconds, 1.25);
        assert_eq!(lines[1].text, "second");
    }
    #[test]
    fn active_line_seeking_and_boundaries() {
        let lyrics = LyricsContent::Synced(parse_lrc("[00:01]first\n[00:03]second\n[00:05]"));
        assert_eq!(lyrics.active_line(0.99), None);
        assert_eq!(lyrics.active_line(1.0), Some(0));
        assert_eq!(lyrics.active_line(4.0), Some(1));
        assert_eq!(lyrics.active_line(1.5), Some(0));
        assert_eq!(lyrics.active_line(10.0), Some(2));
        assert_eq!(lyrics.active_line(f64::NAN), None);
        assert_eq!(LyricsContent::Instrumental.active_line(5.0), None);
    }
    #[test]
    fn metadata_validation_and_query_encoding() {
        assert!(TrackKey::new("", "Artist", "", 120.0).is_none());
        assert!(TrackKey::new("Title", "", "", 120.0).is_none());
        for duration in [0.0, -1.0, f64::NAN, f64::INFINITY, 3601.0] {
            assert!(TrackKey::new("Title", "Artist", "", duration).is_none());
        }
        assert_eq!(encode_query("A&B / 雨"), "A%26B%20%2F%20%E9%9B%A8");
    }
    #[test]
    fn exact_match_synced_precedes_plain() {
        let data = r#"{"trackName":"Quiet Song","artistName":"Test Artist","duration":121,"plainLyrics":"plain","syncedLyrics":"[00:01]timed"}"#;
        let mut client = client(vec![response(200, data)]);
        let Some(LyricsStatus::Found(content)) = client.lookup(&track(), &cancellation(), 1) else {
            panic!("expected lyrics")
        };
        assert!(matches!(&*content, LyricsContent::Synced(lines) if lines[0].text == "timed"));
        assert_eq!(client.transport.paths.len(), 1);
        assert!(client.transport.paths[0].contains("duration=120"));
    }
    #[test]
    fn search_rejects_live_cover_wrong_artist_and_duration() {
        let candidates = [
            record("Quiet Song (Live)", "Test Artist", 120.0, "live".into()),
            record("Quiet Song", "Cover Artist", 120.0, "cover".into()),
            record("Quiet Song", "Test Artist", 124.0, "wrong duration".into()),
            record(" quiet song ", "TEST ARTIST", 119.0, "correct".into()),
        ];
        let mut client = client(vec![
            response(404, ""),
            response(200, &format!("[{}]", candidates.join(","))),
        ]);
        let Some(LyricsStatus::Found(content)) = client.lookup(&track(), &cancellation(), 1) else {
            panic!("expected conservative match")
        };
        assert_eq!(*content, LyricsContent::Plain(vec!["correct".into()]));
        assert_eq!(client.transport.paths.len(), 2);
    }
    #[test]
    fn instrumental_plain_fallback_and_no_matches() {
        let instrumental = r#"{"trackName":"Quiet Song","artistName":"Test Artist","duration":120,"instrumental":true}"#;
        let mut found = client(vec![response(200, instrumental)]);
        assert!(
            matches!(found.lookup(&track(), &cancellation(), 1), Some(LyricsStatus::Found(content)) if *content == LyricsContent::Instrumental)
        );
        let fallback = r#"{"trackName":"Quiet Song","artistName":"Test Artist","duration":120,"plainLyrics":"line one\n\nline two","syncedLyrics":"invalid"}"#;
        let mut plain = client(vec![response(200, fallback)]);
        assert!(
            matches!(plain.lookup(&track(), &cancellation(), 1), Some(LyricsStatus::Found(content)) if *content == LyricsContent::Plain(vec!["line one".into(),"line two".into()]))
        );
        let mut missing = client(vec![response(404, ""), response(200, "[]")]);
        assert_eq!(
            missing.lookup(&track(), &cancellation(), 1),
            Some(LyricsStatus::NotFound)
        );
    }
    #[test]
    fn cancellation_and_rate_limits_never_retry_or_search() {
        let mut limited = response(429, "");
        limited.retry_after = Some(Duration::from_secs(60));
        let mut client = client(vec![limited]);
        assert!(matches!(
            client.lookup(&track(), &cancellation(), 1),
            Some(LyricsStatus::Unavailable(_))
        ));
        assert!(matches!(
            client.lookup(&track(), &cancellation(), 1),
            Some(LyricsStatus::Unavailable(_))
        ));
        assert_eq!(client.transport.paths.len(), 1);
        let cancelled = Cancellation {
            serial: AtomicU64::new(2),
            stopping: AtomicBool::new(false),
        };
        assert_eq!(client.lookup(&track(), &cancelled, 1), None);
    }
    #[test]
    fn invalid_provider_json_reports_error_without_search() {
        let mut client = client(vec![response(200, "<html>not json</html>")]);
        assert!(matches!(
            client.lookup(&track(), &cancellation(), 1),
            Some(LyricsStatus::Unavailable(_))
        ));
        assert_eq!(client.transport.paths.len(), 1);
    }
    #[test]
    fn repeated_timestamps_and_plain_lyrics_have_aggregate_text_budgets() {
        let repeated = repeated_timestamp_fixture();
        assert!(repeated.len() < MAX_RESPONSE_BYTES);
        let lines = parse_lrc(&repeated);
        let text_bytes: usize = lines.iter().map(|line| line.text.len()).sum();
        assert_eq!(text_bytes, MAX_DOCUMENT_TEXT_BYTES);
        assert!(lines.len() <= MAX_LINES);
        let plain = LyricsRecord {
            track_name: "Quiet Song".into(),
            artist_name: "Test Artist".into(),
            album_name: String::new(),
            duration: 120.0,
            instrumental: false,
            synced_lyrics: None,
            plain_lyrics: Some(format!("{}\n", "🌙".repeat(512)).repeat(1000)),
        }
        .into_content()
        .unwrap();
        let LyricsContent::Plain(plain) = plain else {
            panic!("expected plain fallback")
        };
        assert!(plain.iter().map(String::len).sum::<usize>() <= MAX_DOCUMENT_TEXT_BYTES);
    }
    #[test]
    fn cache_enforces_memory_and_entry_limits_and_lru_order() {
        let mut cache = LyricsCache::default();
        for index in 0..17 {
            let key = TrackKey::new(&format!("Song {index}"), "Artist", "", 120.0).unwrap();
            cache.insert(key, LyricsStatus::NotFound);
        }
        assert_eq!(cache.entries.len(), CACHE_CAPACITY);
        assert!(
            cache
                .get(&TrackKey::new("Song 0", "Artist", "", 120.0).unwrap())
                .is_none()
        );
        let retained = TrackKey::new("Song 1", "Artist", "", 120.0).unwrap();
        assert!(cache.get(&retained).is_some());
        cache.insert(track(), LyricsStatus::NotFound);
        assert!(cache.get(&retained).is_some());
        assert!(
            cache
                .get(&TrackKey::new("Song 2", "Artist", "", 120.0).unwrap())
                .is_none()
        );
        let document = LyricsStatus::Found(Arc::new(LyricsContent::Synced(parse_lrc(
            &repeated_timestamp_fixture(),
        ))));
        for index in 0..17 {
            let key = TrackKey::new(&format!("Large {index}"), "Artist", "", 120.0).unwrap();
            cache.insert(key, document.clone());
        }
        assert!(cache.bytes <= MAX_CACHE_BYTES);
        assert!(cache.entries.len() < CACHE_CAPACITY);
        let actual: usize = cache
            .entries
            .iter()
            .map(|(key, status)| cache_entry_bytes(key, status))
            .sum();
        assert_eq!(cache.bytes, actual);
    }
    #[test]
    fn worker_coalesces_uses_cache_and_force_refreshes() {
        use std::sync::atomic::AtomicUsize;
        struct CountingTransport {
            calls: Arc<AtomicUsize>,
            responses: VecDeque<HttpResponse>,
        }
        impl Transport for CountingTransport {
            fn get(&mut self, _: &str, _: &Cancellation, _: u64) -> Result<HttpResponse, String> {
                self.calls.fetch_add(1, Ordering::AcqRel);
                Ok(self.responses.pop_front().expect("unexpected lookup"))
            }
        }
        let state = Arc::new(Cancellation {
            serial: AtomicU64::new(2),
            stopping: AtomicBool::new(false),
        });
        let (commands, receiver) = mpsc::channel();
        let (changes, callbacks) = mpsc::channel();
        for serial in [1, 2] {
            commands
                .send(Command::Lookup(Request {
                    serial,
                    generation: serial,
                    track: track(),
                    refresh: false,
                }))
                .unwrap();
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let transport = CountingTransport {
            calls: calls.clone(),
            responses: vec![
                response(
                    200,
                    &record("Quiet Song", "Test Artist", 120.0, "first".into()),
                ),
                response(
                    200,
                    &record("Quiet Song", "Test Artist", 120.0, "refreshed".into()),
                ),
            ]
            .into(),
        };
        let worker_state = state.clone();
        let worker = thread::spawn(move || {
            run_worker(
                receiver,
                &worker_state,
                |event| {
                    let _ = changes.send(event);
                },
                transport,
            )
        });
        assert_eq!(
            callbacks
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .generation,
            2
        );
        state.serial.store(3, Ordering::Release);
        commands
            .send(Command::Lookup(Request {
                serial: 3,
                generation: 3,
                track: track(),
                refresh: false,
            }))
            .unwrap();
        assert_eq!(
            callbacks
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .generation,
            3
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        state.serial.store(4, Ordering::Release);
        commands
            .send(Command::Lookup(Request {
                serial: 4,
                generation: 4,
                track: track(),
                refresh: true,
            }))
            .unwrap();
        let event = callbacks.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event.generation, 4);
        assert!(
            matches!(event.status,LyricsStatus::Found(content) if *content == LyricsContent::Plain(vec!["refreshed".into()]))
        );
        assert_eq!(calls.load(Ordering::Acquire), 2);
        state.stopping.store(true, Ordering::Release);
        commands.send(Command::Wake).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn in_flight_retarget_suppresses_old_callback() {
        struct InterruptibleTransport {
            calls: usize,
            started: mpsc::Sender<()>,
        }
        impl Transport for InterruptibleTransport {
            fn get(
                &mut self,
                _: &str,
                cancel: &Cancellation,
                serial: u64,
            ) -> Result<HttpResponse, String> {
                self.calls += 1;
                if self.calls == 1 {
                    let _ = self.started.send(());
                    while !cancel.cancelled(serial) {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err("cancelled fixture".into())
                } else {
                    Ok(response(
                        200,
                        &record("Quiet Song", "Test Artist", 120.0, "latest".into()),
                    ))
                }
            }
        }
        let state = Arc::new(cancellation());
        let worker_state = state.clone();
        let (commands, receiver) = mpsc::channel();
        let (changes, callbacks) = mpsc::channel();
        let (started, start) = mpsc::channel();
        let worker = thread::spawn(move || {
            run_worker(
                receiver,
                &worker_state,
                |event| {
                    let _ = changes.send(event);
                },
                InterruptibleTransport { calls: 0, started },
            )
        });
        commands
            .send(Command::Lookup(Request {
                serial: 1,
                generation: 1,
                track: track(),
                refresh: false,
            }))
            .unwrap();
        start.recv_timeout(Duration::from_secs(2)).unwrap();
        state.serial.store(2, Ordering::Release);
        commands
            .send(Command::Lookup(Request {
                serial: 2,
                generation: 2,
                track: track(),
                refresh: false,
            }))
            .unwrap();
        assert_eq!(
            callbacks
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .generation,
            2
        );
        assert!(callbacks.try_recv().is_err());
        state.stopping.store(true, Ordering::Release);
        commands.send(Command::Wake).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn unrepresentable_retry_after_never_panics_or_retries() {
        let mut limited = response(503, "");
        limited.retry_after = Some(Duration::MAX);
        let mut client = client(vec![limited]);
        assert!(matches!(
            client.lookup(&track(), &cancellation(), 1),
            Some(LyricsStatus::Unavailable(_))
        ));
        assert!(matches!(
            client.lookup(&track(), &cancellation(), 1),
            Some(LyricsStatus::Unavailable(_))
        ));
        assert_eq!(client.transport.paths.len(), 1);
    }
    #[test]
    #[ignore = "explicit opt-in live provider check; only reports counts and status"]
    fn live_provider_metadata_smoke() {
        let mut client = LookupClient::new(NativeTransport);
        let track = TrackKey::new(
            "I Want to Live",
            "Borislav Slavov",
            "Baldur's Gate 3 (Original Game Soundtrack)",
            233.0,
        )
        .unwrap();
        let result = client
            .lookup(&track, &cancellation(), 1)
            .expect("not cancelled");
        match result {
            LyricsStatus::Found(content) => match &*content {
                LyricsContent::Synced(lines) => println!(
                    "LRCLIB: synchronized lyrics, {} timestamped lines",
                    lines.len()
                ),
                LyricsContent::Plain(lines) => {
                    println!("LRCLIB: plain lyrics, {} lines", lines.len())
                }
                LyricsContent::Instrumental => println!("LRCLIB: instrumental record"),
            },
            other => panic!("LRCLIB live smoke did not resolve: {other:?}"),
        }
    }
}
