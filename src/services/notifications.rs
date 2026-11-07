//! Opt-in notification listener for an identity-bearing MSIX build.
//!
//! Portable builds do not request consent, subscribe, enumerate, or emulate access.
//! Content remains in memory only and is discarded on disable/revocation/shutdown.
use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Foundation::{Size, TypedEventHandler},
    Security::Authorization::AppCapabilityAccess::AppCapability,
    Storage::Streams::DataReader,
    UI::Notifications::{
        Management::{
            UserNotificationListener as Listener, UserNotificationListenerAccessStatus as OsAccess,
        },
        NotificationKinds, UserNotification, UserNotificationChangedKind,
    },
    Win32::{
        Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
        Storage::Packaging::Appx::GetCurrentPackageFullName,
        System::{
            Com::{
                CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            WinRT::{RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
        },
        UI::Shell::{
            AO_NONE, ApplicationActivationManager, FOLDERID_AppsFolder,
            IApplicationActivationManager, IShellItem, KNOWN_FOLDER_FLAG,
            SHCreateItemInKnownFolder,
        },
    },
    core::{HSTRING, PCWSTR},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessStatus {
    UnavailablePortable,
    NotRequested,
    Denied,
    Allowed,
    Error(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationRecord {
    pub id: u32,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub created_ticks: i64,
    pub icon: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationEvent {
    Access(AccessStatus),
    Added(NotificationRecord),
    Updated(NotificationRecord),
    Removed(u32),
}

/// Identity is required even on Windows versions where Current/RequestAccessAsync
/// happen to succeed without it: the event registration is not portable-supported.
pub fn has_package_identity() -> Result<bool, String> {
    let mut count = 0u32;
    // SAFETY: this size-only query passes a valid length output and no buffer.
    let status = unsafe { GetCurrentPackageFullName(&mut count, None) };
    if status == APPMODEL_ERROR_NO_PACKAGE {
        Ok(false)
    } else if status == ERROR_INSUFFICIENT_BUFFER || status == ERROR_SUCCESS {
        Ok(true)
    } else {
        Err(format!(
            "Cannot query package identity (Win32 {})",
            status.0
        ))
    }
}
struct Shared {
    status: Mutex<AccessStatus>,
    callback: Box<dyn Fn(NotificationEvent) + Send + Sync>,
    enabled: AtomicBool,
    stopping: AtomicBool,
    initial: AtomicBool,
    sender: mpsc::Sender<Command>,
}
impl Shared {
    fn access(&self, status: AccessStatus) {
        let first = !self.initial.swap(true, Ordering::AcqRel);
        let changed = {
            let mut old = self.status.lock().unwrap_or_else(|e| e.into_inner());
            if *old == status {
                false
            } else {
                *old = status.clone();
                true
            }
        };
        if (first || changed) && !self.stopping.load(Ordering::Acquire) {
            (self.callback)(NotificationEvent::Access(status));
        }
    }
    fn emit(&self, event: NotificationEvent) {
        if !self.stopping.load(Ordering::Acquire) {
            (self.callback)(event);
        }
    }
}
enum Command {
    RefreshAccess,
    Enabled,
    Changed(u32, UserNotificationChangedKind),
    Stop,
}
pub struct NotificationService {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl NotificationService {
    /// Starts disabled; permission is never requested here. `set_enabled(true)`
    /// collects only if Windows already reports Allowed.
    pub fn new(
        on_event: impl Fn(NotificationEvent) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let shared = Arc::new(Shared {
            status: Mutex::new(AccessStatus::NotRequested),
            callback: Box::new(on_event),
            enabled: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            initial: AtomicBool::new(false),
            sender,
        });
        match has_package_identity() {
            Ok(false) => {
                shared.access(AccessStatus::UnavailablePortable);
                return Ok(Self {
                    shared,
                    worker: None,
                });
            }
            Err(error) => {
                shared.access(AccessStatus::Error(error));
                return Ok(Self {
                    shared,
                    worker: None,
                });
            }
            Ok(true) => {}
        }
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("still-notifications".into())
            .spawn(move || {
                if let Err(error) = worker(&state, receiver) {
                    state.access(AccessStatus::Error(error));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn access_status(&self) -> AccessStatus {
        self.shared
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn set_enabled(&self, enabled: bool) {
        self.shared.enabled.store(enabled, Ordering::Release);
        if self.worker.is_some() {
            let _ = self.shared.sender.send(Command::Enabled);
        }
    }
    pub fn refresh_access(&self) {
        if self.worker.is_some() {
            let _ = self.shared.sender.send(Command::RefreshAccess);
        }
    }
    /// Must be invoked and awaited on the foreground GUI thread after a direct
    /// user action. It is !Send to prevent moving its COM apartment guard between
    /// threads. Caller refreshes the service after completion. No request occurs
    /// in a portable process, and this method never opens Windows Settings.
    pub async fn request_access() -> Result<AccessStatus, String> {
        if !has_package_identity()? {
            return Ok(AccessStatus::UnavailablePortable);
        }
        let _apartment = Apartment::ui()?;
        let listener = Listener::Current().map_err(|e| e.to_string())?;
        let result = listener
            .RequestAccessAsync()
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        Ok(map_access(result))
    }
}
impl Drop for NotificationService {
    fn drop(&mut self) {
        self.shared.enabled.store(false, Ordering::Release);
        self.shared.stopping.store(true, Ordering::Release);
        let _ = self.shared.sender.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn map_access(status: OsAccess) -> AccessStatus {
    if status == OsAccess::Allowed {
        AccessStatus::Allowed
    } else if status == OsAccess::Denied {
        AccessStatus::Denied
    } else {
        AccessStatus::NotRequested
    }
}
struct Apartment {
    uninitialize: bool,
    _thread: PhantomData<Rc<()>>,
}
impl Apartment {
    fn worker() -> Result<Self, String> {
        Self::initialize(RO_INIT_MULTITHREADED)
    }
    fn ui() -> Result<Self, String> {
        Self::initialize(RO_INIT_SINGLETHREADED)
    }
    fn initialize(kind: windows::Win32::System::WinRT::RO_INIT_TYPE) -> Result<Self, String> {
        // SAFETY: initialized and balanced on this same thread. Existing COM
        // apartments are accepted without changing or uninitializing their mode.
        match unsafe { RoInitialize(kind) } {
            Ok(()) => Ok(Self {
                uninitialize: true,
                _thread: PhantomData,
            }),
            Err(e) if e.code().0 == 0x80010106u32 as i32 => Ok(Self {
                uninitialize: false,
                _thread: PhantomData,
            }),
            Err(e) => Err(e.to_string()),
        }
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { RoUninitialize() }
        }
    }
}
macro_rules! wait {
    ($operation:expr,$shared:expr) => {{
        let operation = $operation.map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(1);
        operation
            .when(move |result| {
                let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
            })
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            if $shared.stopping.load(Ordering::Acquire)
                || !$shared.enabled.load(Ordering::Acquire)
                || start.elapsed() > Duration::from_secs(3)
            {
                let _ = operation.Cancel();
                break Err("Notification operation cancelled or timed out".to_string());
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => {
                    result?;
                    break operation.GetResults().map_err(|e| e.to_string());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break Err("Notification completion channel closed".into()),
            }
        }
    }};
}
struct AccessEvents {
    capability: AppCapability,
    token: i64,
}
impl AccessEvents {
    fn new(shared: &Arc<Shared>) -> Result<Self, String> {
        let capability = AppCapability::Create(&HSTRING::from("userNotificationListener"))
            .map_err(|e| e.to_string())?;
        let state = shared.clone();
        let token = capability
            .AccessChanged(&TypedEventHandler::new(move |_, _| {
                if !state.stopping.load(Ordering::Acquire) {
                    let _ = state.sender.send(Command::RefreshAccess);
                }
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        Ok(Self { capability, token })
    }
}
impl Drop for AccessEvents {
    fn drop(&mut self) {
        let _ = self.capability.RemoveAccessChanged(self.token);
    }
}
struct NotificationEvents {
    listener: Listener,
    token: i64,
}
impl NotificationEvents {
    fn new(listener: &Listener, shared: &Arc<Shared>) -> Result<Self, String> {
        let state = shared.clone();
        let token = listener
            .NotificationChanged(&TypedEventHandler::new(
                move |_,
                      args: windows::core::Ref<
                    windows::UI::Notifications::UserNotificationChangedEventArgs,
                >| {
                    if state.enabled.load(Ordering::Acquire)
                        && !state.stopping.load(Ordering::Acquire)
                    {
                        if let Some(args) = args.as_ref() {
                            if let (Ok(id), Ok(kind)) =
                                (args.UserNotificationId(), args.ChangeKind())
                            {
                                let _ = state.sender.send(Command::Changed(id, kind));
                            }
                        }
                    }
                    Ok(())
                },
            ))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            listener: listener.clone(),
            token,
        })
    }
}
impl Drop for NotificationEvents {
    fn drop(&mut self) {
        let _ = self.listener.RemoveNotificationChanged(self.token);
    }
}
struct State {
    events: Option<NotificationEvents>,
    known: HashMap<u32, NotificationRecord>,
    displayed: HashSet<u32>,
    icons: HashMap<String, Vec<u8>>,
}
impl State {
    fn clear(&mut self, shared: &Shared) {
        self.events = None;
        for id in self.displayed.drain() {
            shared.emit(NotificationEvent::Removed(id));
        }
        self.known.clear();
        self.icons.clear();
    }
    fn refresh(&mut self, listener: &Listener, shared: &Arc<Shared>) -> Result<(), String> {
        let status = map_access(listener.GetAccessStatus().map_err(|e| e.to_string())?);
        if status != AccessStatus::Allowed || !shared.enabled.load(Ordering::Acquire) {
            self.clear(shared);
            shared.access(status);
            return Ok(());
        }
        if self.events.is_none() {
            self.events = Some(NotificationEvents::new(listener, shared)?);
            let baseline_started = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| {
                    ((d.as_secs() as i128 + 11_644_473_600) * 10_000_000
                        + d.subsec_nanos() as i128 / 100)
                        .min(i64::MAX as i128) as i64
                })
                .unwrap_or(i64::MAX);
            let existing = wait!(
                listener.GetNotificationsAsync(NotificationKinds::Toast),
                shared
            )?;
            // Suppress the existing baseline. Bodies are retained only in this
            // bounded in-memory diff cache; no UI history or disk log is created.
            let count = existing.Size().map_err(|e| e.to_string())?.min(512);
            for index in 0..count {
                if let Ok(notification) = existing.GetAt(index) {
                    if let Ok(record) = record(&notification) {
                        if record.created_ticks <= baseline_started {
                            self.known.insert(record.id, record);
                        }
                    }
                }
            }
            if listener.GetAccessStatus().map_err(|e| e.to_string())? != OsAccess::Allowed
                || !shared.enabled.load(Ordering::Acquire)
            {
                self.clear(shared);
                shared.access(map_access(
                    listener.GetAccessStatus().map_err(|e| e.to_string())?,
                ));
                return Ok(());
            }
        }
        shared.access(AccessStatus::Allowed);
        Ok(())
    }
    fn changed(
        &mut self,
        listener: &Listener,
        shared: &Arc<Shared>,
        id: u32,
        kind: UserNotificationChangedKind,
    ) -> Result<(), String> {
        // Always recheck consent before using listener data; Windows can return
        // an empty collection rather than throwing when permission is revoked.
        self.refresh(listener, shared)?;
        if self.events.is_none() || !shared.enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        if kind == UserNotificationChangedKind::Removed {
            self.known.remove(&id);
            if self.displayed.remove(&id) {
                shared.emit(NotificationEvent::Removed(id));
            }
            return Ok(());
        }
        let notification = match listener.GetNotification(id) {
            Ok(value) => value,
            Err(_) => {
                self.known.remove(&id);
                if self.displayed.remove(&id) {
                    shared.emit(NotificationEvent::Removed(id));
                }
                return self.refresh(listener, shared);
            }
        };
        // A provider can disappear or expose malformed metadata between reads.
        // Skip only that record; it must not revoke an otherwise healthy listener.
        let mut value = match record(&notification) {
            Ok(value) => value,
            Err(_) => return Ok(()),
        };
        if self
            .known
            .get(&id)
            .is_some_and(|old| same_content(old, &value))
        {
            return Ok(());
        }
        let updated = self.known.contains_key(&id);
        if let Some(icon) = self.icons.get(&value.app_id) {
            value.icon = Some(icon.clone());
        } else if let Ok(Some(icon)) = icon(&notification, shared) {
            cache_icon(&mut self.icons, &value.app_id, icon.clone());
            value.icon = Some(icon);
        }
        if !shared.enabled.load(Ordering::Acquire)
            || listener.GetAccessStatus().map_err(|e| e.to_string())? != OsAccess::Allowed
        {
            return self.refresh(listener, shared);
        }
        if self.known.len() >= 512 && !self.known.contains_key(&id) {
            if let Some(oldest) = self
                .known
                .values()
                .min_by_key(|value| value.created_ticks)
                .map(|value| value.id)
            {
                self.known.remove(&oldest);
                if self.displayed.remove(&oldest) {
                    shared.emit(NotificationEvent::Removed(oldest));
                }
            }
        }
        let mut cached = value.clone();
        cached.icon = None;
        self.known.insert(id, cached);
        self.displayed.insert(id);
        shared.emit(if updated {
            NotificationEvent::Updated(value)
        } else {
            NotificationEvent::Added(value)
        });
        Ok(())
    }
}
fn worker(shared: &Arc<Shared>, receiver: mpsc::Receiver<Command>) -> Result<(), String> {
    let _apartment = Apartment::worker()?;
    let listener = Listener::Current().map_err(|e| e.to_string())?;
    let _access_events = AccessEvents::new(shared)?;
    let mut state = State {
        events: None,
        known: HashMap::new(),
        displayed: HashSet::new(),
        icons: HashMap::new(),
    };
    state.refresh(&listener, shared)?;
    while let Ok(command) = receiver.recv() {
        if shared.stopping.load(Ordering::Acquire) || matches!(command, Command::Stop) {
            break;
        }
        let result = match command {
            Command::Changed(id, kind) => state.changed(&listener, shared, id, kind),
            _ => state.refresh(&listener, shared),
        };
        if let Err(error) = result {
            state.clear(shared);
            if shared.enabled.load(Ordering::Acquire) {
                shared.access(AccessStatus::Error(error));
            } else {
                let status = listener
                    .GetAccessStatus()
                    .map(map_access)
                    .unwrap_or_else(|e| AccessStatus::Error(e.to_string()));
                shared.access(status);
            }
        }
    }
    state.clear(shared);
    Ok(())
}
fn cache_icon(cache: &mut HashMap<String, Vec<u8>>, app_id: &str, bytes: Vec<u8>) {
    if bytes.len() > 64 * 1024 {
        return;
    }
    if cache.len() >= 32 && !cache.contains_key(app_id) {
        if let Some(key) = cache.keys().next().cloned() {
            cache.remove(&key);
        }
    }
    cache.insert(app_id.to_owned(), bytes);
}
fn same_content(a: &NotificationRecord, b: &NotificationRecord) -> bool {
    a.id == b.id
        && a.app_id == b.app_id
        && a.app_name == b.app_name
        && a.title == b.title
        && a.body == b.body
        && a.created_ticks == b.created_ticks
}
fn bounded_text(text: impl AsRef<str>, limit: usize) -> String {
    text.as_ref()
        .chars()
        .filter(|c| *c != '\0')
        .take(limit)
        .collect::<String>()
        .trim()
        .to_string()
}
fn record(notification: &UserNotification) -> Result<NotificationRecord, String> {
    let app = notification.AppInfo().ok();
    let app_id = app
        .as_ref()
        .and_then(|app| app.AppUserModelId().ok())
        .map(|id| id.to_string())
        .unwrap_or_default();
    let app_name = app
        .as_ref()
        .and_then(|app| app.DisplayInfo().ok())
        .and_then(|info| info.DisplayName().ok())
        .map(|name| name.to_string())
        .unwrap_or_else(|| "Application".into());
    let visual = notification
        .Notification()
        .and_then(|value| value.Visual())
        .ok();
    let binding = visual.as_ref().and_then(|visual| {
        visual
            .GetBinding(&HSTRING::from("ToastGeneric"))
            .or_else(|_| visual.Bindings().and_then(|bindings| bindings.GetAt(0)))
            .ok()
    });
    let elements = binding.and_then(|binding| binding.GetTextElements().ok());
    let mut text = Vec::new();
    if let Some(elements) = elements {
        for index in 0..elements.Size().unwrap_or(0).min(12) {
            if let Ok(item) = elements.GetAt(index) {
                if let Ok(value) = item.Text() {
                    text.push(value.to_string());
                }
            }
        }
    }
    let title = bounded_text(text.first().map_or("Notification", String::as_str), 512);
    let body = bounded_text(
        text.iter().skip(1).cloned().collect::<Vec<_>>().join("\n"),
        4096,
    );
    Ok(NotificationRecord {
        id: notification.Id().map_err(|e| e.to_string())?,
        app_id,
        app_name: bounded_text(app_name, 256),
        title,
        body,
        created_ticks: notification
            .CreationTime()
            .map_err(|e| e.to_string())?
            .UniversalTime,
        icon: None,
    })
}
fn icon(notification: &UserNotification, shared: &Shared) -> Result<Option<Vec<u8>>, String> {
    let reference = notification
        .AppInfo()
        .and_then(|v| v.DisplayInfo())
        .and_then(|v| {
            v.GetLogo(Size {
                Width: 48.0,
                Height: 48.0,
            })
        })
        .map_err(|e| e.to_string())?;
    let stream = wait!(reference.OpenReadAsync(), shared)?;
    let result = (|| -> Result<Option<Vec<u8>>, String> {
        let size = stream.Size().map_err(|e| e.to_string())?;
        if size == 0 || size > 64 * 1024 {
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
struct ComGuard(bool);
impl ComGuard {
    fn new() -> Result<Self, String> {
        // SAFETY: balance only this call's successful COM initialization on the same thread.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result.0 == 0x80010106u32 as i32 {
            Ok(Self(false))
        } else {
            Err(result.to_string())
        }
    }
}
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}
fn aumid(app_id: &str) -> Option<Vec<u16>> {
    if app_id.is_empty()
        || app_id.len() > 4096
        || app_id.chars().any(char::is_control)
        || app_id.contains(['\\', '/', ':'])
        || matches!(app_id, "." | "..")
    {
        None
    } else {
        Some(app_id.encode_utf16().chain([0]).collect())
    }
}
/// Resolve an actual registered AppsFolder entry; arbitrary strings/paths are not
/// treated as launch commands. No process is started by this availability check.
pub fn can_open_source(app_id: &str) -> bool {
    let Some(id) = aumid(app_id) else {
        return false;
    };
    let Ok(_com) = ComGuard::new() else {
        return false;
    }; // SAFETY: a null-terminated AUMID and known-folder GUID live through this synchronous call.
    unsafe {
        SHCreateItemInKnownFolder::<_, IShellItem>(
            &FOLDERID_AppsFolder,
            KNOWN_FOLDER_FLAG(0),
            PCWSTR(id.as_ptr()),
        )
        .is_ok()
    }
}
/// Activates the registered app's normal entry point. Notification deep links and
/// arbitrary toast activation arguments are deliberately not guessed or replayed.
pub fn open_source_app(app_id: &str) -> Result<(), String> {
    let id = aumid(app_id).ok_or("Notification source is not a valid app identity")?;
    if !can_open_source(app_id) {
        return Err("This notification source has no registered application entry".into());
    }
    let _com = ComGuard::new()?; // SAFETY: COM is initialized and the AUMID buffer lives through ActivateApplication.
    unsafe {
        let activation = (|| -> windows::core::Result<()> {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)?;
            manager.ActivateApplication(PCWSTR(id.as_ptr()), PCWSTR::null(), AO_NONE)?;
            Ok(())
        })();
        if activation.is_ok() {
            return Ok(());
        }
        // Some registered desktop applications are AppsFolder entries without
        // package activation. Invoke only the already-resolved shell namespace
        // item, with no arguments, rather than interpreting a path or command.
        let target: Vec<u16> = format!("shell:AppsFolder\\{app_id}")
            .encode_utf16()
            .chain([0])
            .collect();
        let result = windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
        if result.0 as isize <= 32 {
            return Err(format!(
                "Registered source application could not be opened (Shell code {})",
                result.0 as isize
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn icon_cache_has_a_two_mib_ceiling() {
        let mut cache = HashMap::new();
        for index in 0..80 {
            cache_icon(&mut cache, &index.to_string(), vec![0; 64 * 1024]);
        }
        assert_eq!(cache.len(), 32);
        assert!(cache.values().map(Vec::len).sum::<usize>() <= 2 * 1024 * 1024);
        cache_icon(&mut cache, "oversize", vec![0; 64 * 1024 + 1]);
        assert!(!cache.contains_key("oversize"));
    }
    #[test]
    fn strips_nul_and_bounds_text() {
        assert_eq!(bounded_text(" a\0bc ", 2), "a");
        assert_eq!(bounded_text("αβγ", 2), "αβ");
    }
    #[test]
    fn rejects_unsafe_source_ids() {
        assert!(aumid("").is_none());
        assert!(aumid("App\nCommand").is_none());
        assert!(aumid("App!Main").is_some());
    }
    #[test]
    fn diff_ignores_optional_icon() {
        let a = NotificationRecord {
            id: 1,
            app_id: "app".into(),
            app_name: "App".into(),
            title: "Title".into(),
            body: "Body".into(),
            created_ticks: 10,
            icon: None,
        };
        let mut b = a.clone();
        b.icon = Some(vec![1]);
        assert!(same_content(&a, &b));
        b.body = "Changed".into();
        assert!(!same_content(&a, &b));
    }
}
