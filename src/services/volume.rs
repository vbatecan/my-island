//! Event-driven Windows Core Audio control for the default multimedia render endpoint.
//!
//! COM interfaces stay on a dedicated MTA thread. Its command receiver sleeps when
//! idle; Windows notifications wake it on volume/device changes. No polling is used.
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole,
            Endpoints::{
                IAudioEndpointVolume, IAudioEndpointVolumeCallback,
                IAudioEndpointVolumeCallback_Impl,
            },
            IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
            MMDeviceEnumerator, eMultimedia, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        },
    },
    core::{GUID, PCWSTR, implement},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VolumeSnapshot {
    pub percent: i32,
    pub muted: bool,
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeOrigin {
    Initial,
    Local,
    External,
    DeviceChanged,
}
#[derive(Clone, Copy, Debug)]
pub struct VolumeEvent {
    pub snapshot: VolumeSnapshot,
    pub origin: VolumeOrigin,
}
const CONTEXT: GUID = GUID::from_u128(0x53dcfc9c_01de_4b57_aeae_331dc2bff308);

type Reply<T> = SyncSender<Result<T, String>>;
type ChangeHandler = Arc<dyn Fn(VolumeEvent) + Send + Sync>;
enum Command {
    Snapshot(Reply<VolumeSnapshot>),
    Set(i32, Reply<()>),
    ToggleMute(Reply<()>),
    Refresh(VolumeOrigin),
    Rebind,
    Shutdown,
}

/// Owns a sleeping COM worker. Drop unregisters callbacks and joins that worker.
pub struct VolumeService {
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}
impl VolumeService {
    pub fn new(on_change: impl Fn(VolumeEvent) + Send + Sync + 'static) -> Result<Self, String> {
        let (commands, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let notify: ChangeHandler = Arc::new(on_change);
        let worker_commands = commands.clone();
        let worker = thread::Builder::new()
            .name("island-volume".into())
            .spawn(move || {
                // SAFETY: this newly created thread has not initialized COM. Every
                // interface is created, called, released, and uninitialized on it.
                let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok();
                if let Err(error) = initialized {
                    let _ = ready_tx.send(Err(format!(
                        "Core Audio COM initialization failed: {error}"
                    )));
                    return;
                }
                match Worker::new(worker_commands, notify) {
                    Ok(mut state) => {
                        let _ = ready_tx.send(Ok(()));
                        state.run(receiver);
                        // Drop COM interfaces and unregister callbacks before CoUninitialize.
                        drop(state);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
                // SAFETY: pairs the successful CoInitializeEx on this exact thread.
                unsafe {
                    CoUninitialize();
                }
            })
            .map_err(|error| format!("Could not start Core Audio worker: {error}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                commands,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = commands.send(Command::Shutdown);
                // Do not hold up the GUI if a broken audio driver stalls COM startup.
                drop(worker);
                Err(format!(
                    "Core Audio initialization did not complete: {error}"
                ))
            }
        }
    }
    pub fn snapshot(&self) -> Result<VolumeSnapshot, String> {
        self.request(Command::Snapshot)
    }
    pub fn set_volume(&self, percent: i32) -> Result<(), String> {
        self.request(|reply| Command::Set(percent.clamp(0, 100), reply))
    }
    pub fn toggle_mute(&self) -> Result<(), String> {
        self.request(Command::ToggleMute)
    }
    fn request<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, String> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .send(command(reply_tx))
            .map_err(|_| "Core Audio service stopped".to_owned())?;
        reply_rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|error| format!("Core Audio did not respond: {error}"))?
    }
}
impl Drop for VolumeService {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct EndpointNotification {
    commands: Sender<Command>,
}
impl IAudioEndpointVolumeCallback_Impl for EndpointNotification_Impl {
    fn OnNotify(
        &self,
        notification: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
    ) -> windows::core::Result<()> {
        // SAFETY: Core Audio owns this callback payload for the duration of OnNotify.
        // Copy only its context GUID; no borrowed pointer escapes the callback.
        let origin =
            if !notification.is_null() && unsafe { (*notification).guidEventContext } == CONTEXT {
                VolumeOrigin::Local
            } else {
                VolumeOrigin::External
            };
        let _ = self.commands.send(Command::Refresh(origin));
        Ok(())
    }
}
#[implement(IMMNotificationClient)]
struct DeviceNotification {
    commands: Sender<Command>,
}
impl IMMNotificationClient_Impl for DeviceNotification_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        let _ = self.commands.send(Command::Rebind);
        Ok(())
    }
    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        let _ = self.commands.send(Command::Rebind);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        let _ = self.commands.send(Command::Rebind);
        Ok(())
    }
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eMultimedia {
            let _ = self.commands.send(Command::Rebind);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(
        &self,
        _id: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}
struct Endpoint {
    volume: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        // SAFETY: same MTA thread owns both live COM pointers. Unregister before
        // releasing either pointer; callbacks never wait for this worker.
        let _ = unsafe { self.volume.UnregisterControlChangeNotify(&self.callback) };
    }
}
struct Worker {
    enumerator: IMMDeviceEnumerator,
    device_callback: IMMNotificationClient,
    commands: Sender<Command>,
    endpoint: Option<Endpoint>,
    endpoint_error: String,
    last: Option<VolumeSnapshot>,
    notify: ChangeHandler,
}
impl Worker {
    fn new(commands: Sender<Command>, notify: ChangeHandler) -> Result<Self, String> {
        // SAFETY: COM is initialized as MTA for the lifetime of this worker.
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|error| format!("Cannot access Windows audio devices: {error}"))?;
        let device_callback: IMMNotificationClient = DeviceNotification {
            commands: commands.clone(),
        }
        .into();
        // SAFETY: callback is kept alive until explicit unregister in Drop.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&device_callback) }
            .map_err(|error| format!("Cannot subscribe to audio device changes: {error}"))?;
        let mut worker = Self {
            enumerator,
            device_callback,
            commands,
            endpoint: None,
            endpoint_error: "No default audio output device".into(),
            last: None,
            notify,
        };
        worker.rebind();
        Ok(worker)
    }
    fn bind(&self) -> windows::core::Result<Endpoint> {
        // SAFETY: all returned interfaces stay on this MTA worker. The endpoint
        // callback only enqueues commands and remains alive while registered.
        unsafe {
            let device = self
                .enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            let callback: IAudioEndpointVolumeCallback = EndpointNotification {
                commands: self.commands.clone(),
            }
            .into();
            volume.RegisterControlChangeNotify(&callback)?;
            Ok(Endpoint { volume, callback })
        }
    }
    fn rebind(&mut self) {
        self.endpoint.take();
        match self.bind() {
            Ok(endpoint) => {
                self.endpoint = Some(endpoint);
                self.endpoint_error.clear();
            }
            Err(error) => {
                self.endpoint_error = format!("Default audio output unavailable: {error}")
            }
        }
        self.publish(if self.last.is_none() {
            VolumeOrigin::Initial
        } else {
            VolumeOrigin::DeviceChanged
        });
    }
    fn read(&self) -> Result<VolumeSnapshot, String> {
        let Some(endpoint) = &self.endpoint else {
            return Ok(VolumeSnapshot::default());
        };
        // SAFETY: endpoint belongs to this MTA and returns scalar/BOOL by value.
        unsafe {
            let scalar = endpoint
                .volume
                .GetMasterVolumeLevelScalar()
                .map_err(|e| format!("Cannot read output volume: {e}"))?;
            let muted = endpoint
                .volume
                .GetMute()
                .map_err(|e| format!("Cannot read output mute: {e}"))?
                .as_bool();
            Ok(VolumeSnapshot {
                percent: (scalar.clamp(0.0, 1.0) * 100.0).round() as i32,
                muted,
                available: true,
            })
        }
    }
    fn publish(&mut self, origin: VolumeOrigin) {
        let value = self.read().unwrap_or_default();
        if self.last != Some(value) {
            self.last = Some(value);
            (self.notify)(VolumeEvent {
                snapshot: value,
                origin,
            });
        }
    }
    fn run(&mut self, receiver: Receiver<Command>) {
        while let Ok(command) = receiver.recv() {
            match command {
                Command::Shutdown => break,
                Command::Rebind => self.rebind(),
                Command::Refresh(origin) => self.publish(origin),
                Command::Snapshot(reply) => {
                    let _ = reply.send(self.read());
                }
                Command::Set(percent, reply) => {
                    // SAFETY: endpoint is live on this MTA; scalar is clamped.
                    let result = if let Some(endpoint) = &self.endpoint {
                        unsafe {
                            endpoint
                                .volume
                                .SetMasterVolumeLevelScalar(percent as f32 / 100.0, &CONTEXT)
                        }
                        .map_err(|e| format!("Cannot change output volume: {e}"))
                    } else {
                        Err(self.endpoint_error.clone())
                    };
                    self.publish(VolumeOrigin::Local);
                    let _ = reply.send(result);
                }
                Command::ToggleMute(reply) => {
                    let result = (|| {
                        let Some(endpoint) = &self.endpoint else {
                            return Err(self.endpoint_error.clone());
                        };
                        // SAFETY: both operations use this thread's live endpoint;
                        // null context is allowed by Core Audio.
                        unsafe {
                            let muted = endpoint
                                .volume
                                .GetMute()
                                .map_err(|e| e.to_string())?
                                .as_bool();
                            endpoint
                                .volume
                                .SetMute(!muted, &CONTEXT)
                                .map_err(|e| format!("Cannot mute output: {e}"))
                        }
                    })();
                    self.publish(VolumeOrigin::Local);
                    let _ = reply.send(result);
                }
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // SAFETY: same thread that registered owns the enumerator and callback.
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.device_callback)
        };
        self.endpoint.take();
    }
}
