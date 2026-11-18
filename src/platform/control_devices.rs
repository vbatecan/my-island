//! Direct Windows discovery and actions for Control Center. All native waits
//! run off the UI thread and return only the state needed to display each row.
use std::{
    cmp::Reverse,
    collections::BTreeMap,
    ffi::c_void,
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows::{
    Devices::{
        Bluetooth::{BluetoothDevice, BluetoothLEDevice},
        Enumeration::{
            DeviceInformation, DeviceInformationKind, DeviceInformationUpdate,
            DevicePairingResultStatus, DeviceWatcher, DeviceWatcherStatus,
        },
    },
    Foundation::{IPropertyValue, TypedEventHandler},
    Win32::{
        Foundation::{
            ERROR_ACCESS_DENIED, ERROR_NDIS_DOT11_POWER_STATE_INVALID, ERROR_SERVICE_NOT_ACTIVE,
            HANDLE,
        },
        NetworkManagement::WiFi::*,
        System::{
            Com::CoIncrementMTAUsage,
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        },
    },
    core::{GUID, HSTRING, IInspectable, Interface, PCWSTR},
};
use windows_collections::IIterable;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wifi,
    Bluetooth,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub connected: bool,
    pub enabled: bool,
    pub signal: Option<u32>,
}

pub async fn list(kind: Kind, scan: bool) -> Result<Vec<Device>, String> {
    let mut result = worker(move || match kind {
        Kind::Wifi => Wifi::open()?.list(scan),
        Kind::Bluetooth => with_apartment(|| bluetooth_list(scan)),
    })
    .await?;
    result.sort_by_cached_key(|row| {
        (
            Reverse(row.signal),
            kind == Kind::Bluetooth && !row.connected,
            row.label.to_lowercase(),
            row.id.clone(),
        )
    });
    Ok(result)
}

pub async fn activate(kind: Kind, id: &str, current: Arc<AtomicBool>) -> Result<(), String> {
    let id = id.to_owned();
    worker(move || {
        if !current.load(Ordering::Acquire) {
            return Ok(());
        }
        match kind {
            Kind::Wifi => Wifi::open()?.connect(&id, &current),
            Kind::Bluetooth => with_apartment(|| pair_bluetooth(&id, &current)),
        }
    })
    .await
}

async fn worker<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (send, reply) = futures_channel::oneshot::channel();
    std::thread::Builder::new()
        .name("island-control-devices".into())
        .spawn(move || {
            let _ = send.send(run());
        })
        .map_err(|_| "Couldn’t start device discovery.".to_owned())?;
    reply
        .await
        .map_err(|_| "Device discovery stopped.".to_owned())?
}

struct Wifi {
    handle: HANDLE,
    // The allocation stays live until after WlanCloseHandle returns from all callbacks.
    notifications: Option<Box<mpsc::Sender<(GUID, u32)>>>,
}

impl Drop for Wifi {
    fn drop(&mut self) {
        // SAFETY: this is the owned WLAN session handle, closed once.
        unsafe { WlanCloseHandle(self.handle, None) };
    }
}

struct WlanList<T>(*mut T);

impl<T> Drop for WlanList<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this owns a buffer allocated by a WLAN API.
            unsafe { WlanFreeMemory(self.0.cast()) };
        }
    }
}

impl Wifi {
    fn open() -> Result<Self, String> {
        let mut handle = HANDLE::default();
        let mut version = 0;
        // SAFETY: both outputs are local and remain live for the call.
        wifi_status(unsafe { WlanOpenHandle(2, None, &mut version, &mut handle) })?;
        Ok(Self {
            handle,
            notifications: None,
        })
    }

    fn interfaces(&self) -> Result<Vec<GUID>, String> {
        let mut list = WlanList(ptr::null_mut::<WLAN_INTERFACE_INFO_LIST>());
        // SAFETY: the output pointer is valid and wrapped for ownership.
        wifi_status(unsafe { WlanEnumInterfaces(self.handle, None, &mut list.0) })?;
        let Some(header) = (unsafe { list.0.as_ref() }) else {
            return Err("Wi-Fi adapter information is unavailable.".into());
        };
        // SAFETY: WLAN_INTERFACE_INFO_LIST is a variable-length allocation from Windows.
        let entries = unsafe {
            std::slice::from_raw_parts(
                ptr::addr_of!((*list.0).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>(),
                header.dwNumberOfItems as usize,
            )
        };
        Ok(entries.iter().map(|entry| entry.InterfaceGuid).collect())
    }

    fn available(&self, interface: &GUID) -> Result<Vec<WLAN_AVAILABLE_NETWORK>, String> {
        let mut list = WlanList(ptr::null_mut::<WLAN_AVAILABLE_NETWORK_LIST>());
        // SAFETY: the supplied interface and the output pointer remain live for the call.
        wifi_status(unsafe {
            WlanGetAvailableNetworkList(self.handle, interface, 0, None, &mut list.0)
        })?;
        let Some(header) = (unsafe { list.0.as_ref() }) else {
            return Err("Wi-Fi network information is unavailable.".into());
        };
        // SAFETY: WLAN_AVAILABLE_NETWORK_LIST is a variable-length allocation from Windows.
        Ok(unsafe {
            std::slice::from_raw_parts(
                ptr::addr_of!((*list.0).Network).cast::<WLAN_AVAILABLE_NETWORK>(),
                header.dwNumberOfItems as usize,
            )
        }
        .to_vec())
    }

    fn notifications(&mut self) -> Option<mpsc::Receiver<(GUID, u32)>> {
        let (send, receive) = mpsc::channel();
        self.notifications = Some(Box::new(send));
        let sender = ptr::from_ref(self.notifications.as_deref().unwrap());
        // SAFETY: sender is heap allocated and the session keeps it until handle closure.
        let status = unsafe {
            WlanRegisterNotification(
                self.handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                true,
                Some(wlan_event),
                Some(sender.cast()),
                None,
                None,
            )
        };
        if status != 0 {
            self.notifications = None;
            return None;
        }
        Some(receive)
    }

    fn scan(&mut self, interfaces: &[GUID]) {
        let events = self.notifications();
        let mut pending: Vec<_> = interfaces
            .iter()
            .copied()
            .filter(|interface| {
                // SAFETY: the adapter ID is live, optional scan filters are null.
                (unsafe { WlanScan(self.handle, interface, None, None, None) }) == 0
            })
            .collect();
        let Some(events) = events else { return };
        let deadline = Instant::now() + Duration::from_secs(4);
        while !pending.is_empty() {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let Ok((interface, code)) = events.recv_timeout(remaining) else {
                break;
            };
            if code == wlan_notification_acm_scan_complete.0 as u32
                || code == wlan_notification_acm_scan_fail.0 as u32
            {
                pending.retain(|pending| pending != &interface);
            }
        }
    }

    fn list(mut self, scan: bool) -> Result<Vec<Device>, String> {
        let interfaces = self.interfaces()?;
        if interfaces.is_empty() {
            return Err("No Wi-Fi adapter found.".into());
        }
        if scan {
            self.scan(&interfaces);
        }
        let mut groups = BTreeMap::new();
        let mut last_error = None;
        let mut accessible = false;
        for interface in interfaces {
            let networks = match self.available(&interface) {
                Ok(networks) => networks,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            accessible = true;
            for network in networks {
                let row = wifi_device(&interface, &network);
                let quality = (row.connected, row.enabled, network.wlanSignalQuality);
                let entry = groups
                    .entry(network_key(&network))
                    .or_insert_with(|| (quality, row.clone()));
                if quality > entry.0 {
                    *entry = (quality, row);
                }
            }
        }
        if !accessible && let Some(error) = last_error {
            return Err(error);
        }
        Ok(groups.into_values().map(|(_, row)| row).collect())
    }

    fn connect(self, id: &str, current: &AtomicBool) -> Result<(), String> {
        let Some((interface, network)) = self.interfaces()?.into_iter().find_map(|interface| {
            let networks = self.available(&interface).ok()?;
            networks
                .into_iter()
                .filter(|network| wifi_id(&interface, network) == id)
                .max_by_key(|network| {
                    (
                        connected(network),
                        saved(network),
                        network.wlanSignalQuality,
                    )
                })
                .map(|network| (interface, network))
        }) else {
            return Err("This network is no longer available.".into());
        };
        if connected(&network) || !current.load(Ordering::Acquire) {
            return Ok(());
        }
        if !wifi_device(&interface, &network).enabled {
            return Err("Windows can’t connect to this network yet.".into());
        }
        let mut ssid = network.dot11Ssid;
        let params = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: if saved(&network) {
                wlan_connection_mode_profile
            } else {
                wlan_connection_mode_discovery_unsecure
            },
            strProfile: if saved(&network) {
                PCWSTR(network.strProfileName.as_ptr())
            } else {
                PCWSTR::null()
            },
            pDot11Ssid: &mut ssid,
            dot11BssType: network.dot11BssType,
            ..Default::default()
        };
        if !current.load(Ordering::Acquire) {
            return Ok(());
        }
        // SAFETY: the adapter, parameters, SSID and profile name all live through the call.
        wifi_status(unsafe { WlanConnect(self.handle, &interface, &params, None) })?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while current.load(Ordering::Acquire) {
            if self
                .available(&interface)?
                .iter()
                .any(|network| connected(network) && wifi_id(&interface, network) == id)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("Wi-Fi couldn’t confirm the connection.".into());
            }
            std::thread::sleep(Duration::from_millis(350));
        }
        Ok(())
    }
}

unsafe extern "system" fn wlan_event(data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    // SAFETY: WLAN owns the notification during this callback. The session owns the
    // sender until WlanCloseHandle has finished all callbacks.
    if let (Some(data), Some(sender)) = unsafe {
        (
            data.as_ref(),
            context.cast::<mpsc::Sender<(GUID, u32)>>().as_ref(),
        )
    } {
        let _ = sender.send((data.InterfaceGuid, data.NotificationCode));
    }
}

fn wifi_status(status: u32) -> Result<(), String> {
    match status {
        0 => Ok(()),
        s if s == ERROR_ACCESS_DENIED.0 => {
            Err("Wi-Fi discovery needs Windows location permission.".into())
        }
        s if s == ERROR_NDIS_DOT11_POWER_STATE_INVALID.0 => {
            Err("Turn on Wi-Fi to see networks.".into())
        }
        s if s == ERROR_SERVICE_NOT_ACTIVE.0 => {
            Err("The Windows Wi-Fi service is unavailable.".into())
        }
        _ => Err("Windows couldn’t access Wi-Fi.".into()),
    }
}

fn network_key(network: &WLAN_AVAILABLE_NETWORK) -> (Vec<u8>, i32, i32, i32) {
    (
        ssid(network).to_vec(),
        network.dot11BssType.0,
        network.dot11DefaultAuthAlgorithm.0,
        network.dot11DefaultCipherAlgorithm.0,
    )
}

fn wifi_id(interface: &GUID, network: &WLAN_AVAILABLE_NETWORK) -> String {
    format!("{interface:?}/{:?}", network_key(network))
}

fn ssid(network: &WLAN_AVAILABLE_NETWORK) -> &[u8] {
    let ssid = &network.dot11Ssid;
    &ssid.ucSSID[..(ssid.uSSIDLength as usize).min(ssid.ucSSID.len())]
}

fn connected(network: &WLAN_AVAILABLE_NETWORK) -> bool {
    network.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0
}

fn saved(network: &WLAN_AVAILABLE_NETWORK) -> bool {
    network.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0 && network.strProfileName[0] != 0
}

fn wifi_device(interface: &GUID, network: &WLAN_AVAILABLE_NETWORK) -> Device {
    let connected = connected(network);
    let saved = saved(network);
    let open = !network.bSecurityEnabled.as_bool();
    let name = String::from_utf8_lossy(ssid(network));
    let label: String = name
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect();
    let available = network.bNetworkConnectable.as_bool() && (saved || !label.is_empty());
    let info = if connected {
        "Connected"
    } else if !available {
        "Unavailable"
    } else if open {
        "Open network"
    } else if saved {
        ""
    } else {
        "Setup required"
    };
    Device {
        id: wifi_id(interface, network),
        label: if label.trim().is_empty() {
            "Hidden network".into()
        } else {
            label
        },
        detail: info.into(),
        connected,
        enabled: available && !connected && (saved || open),
        signal: Some(network.wlanSignalQuality.min(100)),
    }
}

fn with_apartment<T>(run: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    // The Windows bindings cache the Bluetooth activation factory for this
    // process. Keep its MTA alive between short-lived discovery workers.
    static MTA: OnceLock<Result<(), String>> = OnceLock::new();
    MTA.get_or_init(|| {
        // SAFETY: the process retains one MTA usage reference until exit.
        unsafe { CoIncrementMTAUsage() }
            .map(|_| ())
            .map_err(bluetooth_error)
    })
    .clone()?;
    // SAFETY: this helper runs on a private worker thread and balances initialization.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
        .map_err(|_| "Bluetooth discovery is unavailable.".to_owned())?;
    let result = run();
    // SAFETY: the same worker owns the apartment and its WinRT objects are dropped.
    unsafe { RoUninitialize() };
    result
}

const AEP_CONNECTED: &str = "System.Devices.Aep.IsConnected";
const AEP_PRESENT: &str = "System.Devices.Aep.IsPresent";
const AEP_CONTAINER: &str = "System.Devices.Aep.ContainerId";
const AEP_ADDRESS: &str = "System.Devices.Aep.DeviceAddress";

fn watch_bluetooth(inquiry: bool) -> Result<BTreeMap<String, DeviceInformation>, String> {
    let classic =
        BluetoothDevice::GetDeviceSelectorFromPairingState(!inquiry).map_err(bluetooth_error)?;
    let le =
        BluetoothLEDevice::GetDeviceSelectorFromPairingState(!inquiry).map_err(bluetooth_error)?;
    let selector = HSTRING::from(format!("({classic}) OR ({le})"));
    let properties: IIterable<HSTRING> = [AEP_CONNECTED, AEP_PRESENT, AEP_CONTAINER, AEP_ADDRESS]
        .map(HSTRING::from)
        .to_vec()
        .into();
    let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &selector,
        &properties,
        DeviceInformationKind::AssociationEndpoint,
    )
    .map_err(bluetooth_error)?;
    let entries = Arc::new(Mutex::new(BTreeMap::new()));
    let found = entries.clone();
    let add = watcher
        .Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
            move |_, device| {
                let device = device.ok()?;
                found
                    .lock()
                    .unwrap()
                    .insert(device.Id()?.to_string(), device.clone());
                Ok(())
            },
        ))
        .map_err(bluetooth_error)?;
    let updated = entries.clone();
    let update = watcher
        .Updated(
            &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, device| {
                let device = device.ok()?;
                if let Some(entry) = updated.lock().unwrap().get(&device.Id()?.to_string()) {
                    entry.Update(device)?;
                }
                Ok(())
            }),
        )
        .map_err(bluetooth_error)?;
    let removed = entries.clone();
    let remove = watcher
        .Removed(
            &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, device| {
                removed
                    .lock()
                    .unwrap()
                    .remove(&device.ok()?.Id()?.to_string());
                Ok(())
            }),
        )
        .map_err(bluetooth_error)?;
    let (send, done) = mpsc::channel();
    let complete = watcher
        .EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(
            move |_, _| {
                let _ = send.send(());
                Ok(())
            },
        ))
        .map_err(bluetooth_error)?;
    watcher.Start().map_err(bluetooth_error)?;
    if inquiry {
        // Initial enumeration can finish before nearby radios advertise.
        std::thread::sleep(Duration::from_secs(8));
    } else {
        let _ = done.recv_timeout(Duration::from_secs(2));
    }
    let status = watcher.Status().map_err(bluetooth_error)?;
    let _ = watcher.Stop();
    let _ = watcher.RemoveAdded(add);
    let _ = watcher.RemoveUpdated(update);
    let _ = watcher.RemoveRemoved(remove);
    let _ = watcher.RemoveEnumerationCompleted(complete);
    if status == DeviceWatcherStatus::Aborted {
        return Err("Windows stopped Bluetooth discovery.".into());
    }
    let snapshot = std::mem::take(&mut *entries.lock().unwrap());
    Ok(snapshot)
}

fn bluetooth_list(scan: bool) -> Result<Vec<Device>, String> {
    let mut entries = watch_bluetooth(false)?;
    if scan {
        entries.extend(watch_bluetooth(true)?);
    }
    let mut groups = BTreeMap::new();
    for (id, device) in &entries {
        let Ok(pairing) = device.Pairing() else {
            continue;
        };
        let paired = pairing.IsPaired().unwrap_or(false);
        let property = |name| {
            device
                .Properties()
                .ok()?
                .Lookup(&HSTRING::from(name))
                .ok()?
                .cast::<IPropertyValue>()
                .ok()
        };
        let connected = property(AEP_CONNECTED)
            .and_then(|value| value.GetBoolean().ok())
            .unwrap_or(false);
        let present = property(AEP_PRESENT)
            .and_then(|value| value.GetBoolean().ok())
            .unwrap_or(true);
        if !paired && !connected && !present {
            continue;
        }
        let enabled = !paired && !connected && pairing.CanPair().unwrap_or(false);
        let label = device
            .Name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let has_name = !label.trim().is_empty();
        let row = Device {
            id: id.clone(),
            label: if has_name {
                label
            } else {
                "Unknown Bluetooth device".into()
            },
            detail: if connected {
                "Connected"
            } else if paired && !present {
                "Paired · out of range"
            } else if paired {
                "Paired"
            } else if enabled {
                "Available to pair"
            } else {
                "Pairing unavailable"
            }
            .into(),
            connected,
            enabled,
            signal: None,
        };
        let key = property(AEP_CONTAINER)
            .and_then(|value| value.GetGuid().ok())
            .filter(|guid| *guid != GUID::zeroed())
            .map(|guid| format!("container:{guid:?}"))
            .or_else(|| {
                property(AEP_ADDRESS)
                    .and_then(|value| value.GetString().ok())
                    .filter(|address| !address.is_empty())
                    .map(|address| format!("address:{address}"))
            })
            .unwrap_or_else(|| id.clone());
        let priority = (connected, paired, enabled, has_name);
        let entry = groups.entry(key).or_insert_with(|| (priority, row.clone()));
        if priority > entry.0 {
            *entry = (priority, row);
        }
    }
    Ok(groups.into_values().map(|(_, row)| row).collect())
}

fn pair_bluetooth(id: &str, current: &AtomicBool) -> Result<(), String> {
    let properties: IIterable<HSTRING> = Vec::new().into();
    let device = DeviceInformation::CreateFromIdAsyncWithKindAndAdditionalProperties(
        &HSTRING::from(id),
        &properties,
        DeviceInformationKind::AssociationEndpoint,
    )
    .and_then(|operation| operation.join())
    .map_err(bluetooth_error)?;
    let pairing = device.Pairing().map_err(bluetooth_error)?;
    if pairing.IsPaired().map_err(bluetooth_error)? || !current.load(Ordering::Acquire) {
        return Ok(());
    }
    if !pairing.CanPair().map_err(bluetooth_error)? {
        return Err("Put the device in pairing mode and refresh.".into());
    }
    if !current.load(Ordering::Acquire) {
        return Ok(());
    }
    match pairing
        .PairAsync()
        .and_then(|operation| operation.join())
        .and_then(|result| result.Status())
        .map_err(bluetooth_error)?
    {
        DevicePairingResultStatus::Paired | DevicePairingResultStatus::AlreadyPaired => Ok(()),
        DevicePairingResultStatus::AccessDenied => Err("Windows denied Bluetooth pairing.".into()),
        DevicePairingResultStatus::PairingCanceled => {
            Err("Bluetooth pairing was cancelled.".into())
        }
        _ => Err("Bluetooth couldn’t pair with this device.".into()),
    }
}

fn bluetooth_error(error: windows::core::Error) -> String {
    if error.code() == windows::core::HRESULT::from_win32(ERROR_ACCESS_DENIED.0) {
        "Windows denied access to Bluetooth devices.".into()
    } else {
        "Windows couldn’t access Bluetooth devices.".into()
    }
}
