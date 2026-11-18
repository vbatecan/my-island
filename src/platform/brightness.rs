//! Brightness is scoped to the physical display hosting the notch.
use futures_channel::oneshot;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use windows::{
    Win32::{
        Devices::Display::{
            DestroyPhysicalMonitor, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
            GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR, SetMonitorBrightness,
        },
        Foundation::HANDLE,
        Graphics::Gdi::{
            DISPLAY_DEVICE_ACTIVE, DISPLAY_DEVICEW, EnumDisplayDevicesW, GetMonitorInfoW, HMONITOR,
            MONITORINFOEXW,
        },
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
            },
            Variant::{VARIANT, VariantToUInt32Array},
            Wmi::{
                IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_CONNECT_USE_MAX_WAIT,
                WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WbemLocator,
            },
        },
        UI::WindowsAndMessaging::EDD_GET_DEVICE_INTERFACE_NAME,
    },
    core::{BSTR, PCWSTR, w},
};

struct Request {
    monitor: isize,
    value: Option<i32>,
    current: Arc<AtomicBool>,
    reply: oneshot::Sender<Result<Option<i32>, String>>,
}

#[derive(Clone)]
pub struct Service(mpsc::Sender<Request>);

impl Service {
    pub fn start() -> Result<Self, String> {
        let (send, receive) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("island-brightness".into())
            .spawn(move || {
                // SAFETY: the apartment and all interfaces are local to this thread.
                if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
                    return;
                }
                let mut target: Option<(isize, Arc<AtomicBool>, Option<Target>)> = None;
                while let Ok(request) = receive.recv() {
                    if !request.current.load(Ordering::Acquire) {
                        continue;
                    }
                    if target.as_ref().is_none_or(|(monitor, session, _)| {
                        *monitor != request.monitor || !Arc::ptr_eq(session, &request.current)
                    }) {
                        target = Some((
                            request.monitor,
                            request.current.clone(),
                            Target::discover(request.monitor),
                        ));
                    }
                    let result = match target.as_ref().and_then(|(_, _, target)| target.as_ref()) {
                        Some(target) => target.apply(request.value, &request.current),
                        None => Ok(None),
                    };
                    if result.is_err() {
                        target = None;
                    }
                    let _ = request.reply.send(result);
                }
                drop(target);
                // SAFETY: balances initialization on this same thread.
                unsafe { CoUninitialize() };
            })
            .map_err(|error| error.to_string())?;
        Ok(Self(send))
    }

    pub async fn request(
        &self,
        monitor: isize,
        value: Option<i32>,
        current: Arc<AtomicBool>,
    ) -> Result<Option<i32>, String> {
        let (reply, result) = oneshot::channel();
        self.0
            .send(Request {
                monitor,
                value,
                current,
                reply,
            })
            .map_err(|error| error.to_string())?;
        result.await.map_err(|error| error.to_string())?
    }
}

enum Target {
    Wmi { service: IWbemServices, id: String },
    Ddc(DdcDisplay),
}

impl Target {
    fn discover(monitor: isize) -> Option<Self> {
        if let Some(id) = monitor_instance(monitor)
            && let Ok(service) = connect()
            && matches!(read(&service, &id), Ok(Some(_)))
        {
            return Some(Self::Wmi { service, id });
        }
        DdcDisplay::open(monitor).map(Self::Ddc)
    }

    fn apply(&self, value: Option<i32>, current: &AtomicBool) -> Result<Option<i32>, String> {
        if !current.load(Ordering::Acquire) {
            return Ok(None);
        }
        match self {
            Self::Wmi { service, id } => {
                if let Some(value) = value {
                    write(service, id, value, current)?;
                }
                read(service, id)
            }
            Self::Ddc(display) => display.apply(value, current).map(Some),
        }
    }
}

struct DdcDisplay(HANDLE);

impl DdcDisplay {
    fn open(monitor: isize) -> Option<Self> {
        let monitor = HMONITOR(monitor as _);
        let mut count = 0;
        // SAFETY: the size query receives a live output buffer. Cloned displays
        // can map to several physical devices, so only accept a single target.
        unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count) }.ok()?;
        if count != 1 {
            return None;
        }
        let mut physical = [PHYSICAL_MONITOR::default()];
        // SAFETY: the monitor count was checked and the slice has that length.
        unsafe { GetPhysicalMonitorsFromHMONITOR(monitor, &mut physical) }.ok()?;
        let display = Self(physical[0].hPhysicalMonitor);
        // The brightness command can succeed on monitors with an incomplete
        // capabilities response. Require a valid value and range from the device.
        display.level().ok()?;
        Some(display)
    }

    fn level(&self) -> Result<DdcLevel, String> {
        let (mut min, mut max, mut current) = (0, 0, 0);
        // SAFETY: the service owns this live monitor handle and the output buffers.
        let read = unsafe { GetMonitorBrightness(self.0, &mut min, &mut current, &mut max) } != 0;
        if !read || min >= max || !(min..=max).contains(&current) {
            return Err("Monitor did not report a valid brightness range".into());
        }
        Ok(DdcLevel { min, max, current })
    }

    fn apply(&self, value: Option<i32>, current: &AtomicBool) -> Result<i32, String> {
        let level = self.level()?;
        let Some(value) = value.filter(|_| current.load(Ordering::Acquire)) else {
            return Ok(level.percent());
        };
        let raw = level.raw(value);
        if raw == level.current {
            return Ok(level.percent());
        }
        // SAFETY: raw is within the brightness range returned by this same
        // owned physical monitor.
        if unsafe { SetMonitorBrightness(self.0, raw) } == 0 {
            return Err(windows::core::Error::from_thread().to_string());
        }
        Ok(self.level()?.percent())
    }
}

impl Drop for DdcDisplay {
    fn drop(&mut self) {
        // SAFETY: the brightness worker owns the physical monitor handle.
        let _ = unsafe { DestroyPhysicalMonitor(self.0) };
    }
}

struct DdcLevel {
    min: u32,
    max: u32,
    current: u32,
}

impl DdcLevel {
    fn percent(&self) -> i32 {
        let range = u64::from(self.max - self.min);
        ((100 * u64::from(self.current - self.min) + range / 2) / range) as i32
    }

    fn raw(&self, percent: i32) -> u32 {
        let range = u64::from(self.max - self.min);
        self.min + ((range * percent.clamp(0, 100) as u64 + 50) / 100) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddc_values_map_the_reported_range_to_slider_percentages() {
        let level = DdcLevel {
            min: 20,
            max: 220,
            current: 70,
        };
        assert_eq!(level.percent(), 25);
        assert_eq!(level.raw(-10), 20);
        assert_eq!(level.raw(25), 70);
        assert_eq!(level.raw(120), 220);
    }
}

fn monitor_instance(monitor: isize) -> Option<String> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: Windows only reads the handle and fills the size-tagged structure.
    if !unsafe { GetMonitorInfoW(HMONITOR(monitor as _), &mut info.monitorInfo) }.as_bool() {
        return None;
    }
    let mut found = None;
    for index in 0.. {
        let mut device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: szDevice is a terminated GDI name and device has its Win32 byte size.
        if !unsafe {
            EnumDisplayDevicesW(
                PCWSTR(info.szDevice.as_ptr()),
                index,
                &mut device,
                EDD_GET_DEVICE_INTERFACE_NAME,
            )
        }
        .as_bool()
        {
            break;
        }
        if device.StateFlags.contains(DISPLAY_DEVICE_ACTIVE) {
            let end = device.DeviceID.iter().position(|c| *c == 0)?;
            let path = String::from_utf16_lossy(&device.DeviceID[..end]);
            let id = interface_instance(&path)?;
            // A cloned output has no single brightness target.
            if found.replace(id).is_some() {
                return None;
            }
        }
    }
    found
}

fn interface_instance(path: &str) -> Option<String> {
    let path = path.strip_prefix(r"\\?\")?.to_ascii_lowercase();
    let (instance, _) = path.rsplit_once("#{")?;
    (instance.starts_with("display#") && instance.matches('#').count() == 2)
        .then(|| instance.replace('#', "\\"))
}

fn matches_monitor(instance: &str, id: &str) -> bool {
    instance.eq_ignore_ascii_case(id)
        || instance.rsplit_once('_').is_some_and(|(name, index)| {
            name.eq_ignore_ascii_case(id)
                && !index.is_empty()
                && index.bytes().all(|digit| digit.is_ascii_digit())
        })
}

fn connect() -> windows::core::Result<IWbemServices> {
    // SAFETY: called by the initialized MTA. WMI owns the resulting proxy.
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let empty = BSTR::new();
        let service = locator.ConnectServer(
            &BSTR::from(r"ROOT\WMI"),
            &empty,
            &empty,
            &empty,
            WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
            &empty,
            None,
        )?;
        CoSetProxyBlanket(
            &service,
            10, // RPC_C_AUTHN_WINNT
            0,  // RPC_C_AUTHZ_NONE
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        Ok(service)
    }
}

fn property(object: &IWbemClassObject, name: PCWSTR) -> windows::core::Result<VARIANT> {
    let mut value = VARIANT::default();
    // SAFETY: property names are static strings; VARIANT initializes and clears its owned value.
    unsafe { object.Get(name, 0, &mut value, None, None)? };
    Ok(value)
}

fn monitor_object(
    service: &IWbemServices,
    query: &str,
    id: &str,
) -> Result<Option<IWbemClassObject>, String> {
    // SAFETY: the query and output objects live only on the WMI thread.
    unsafe {
        let objects = service
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .map_err(|error| error.to_string())?;
        let mut found = None;
        loop {
            let mut slot = [None];
            let mut returned = 0;
            let status = objects.Next(1200, &mut slot, &mut returned);
            status.ok().map_err(|error| error.to_string())?;
            let Some(object) = slot[0].take() else {
                return if status.0 == 1 {
                    Ok(found)
                } else {
                    Err("Brightness provider did not respond".into())
                };
            };
            let name = property(&object, w!("InstanceName"))
                .and_then(|value| BSTR::try_from(&value))
                .map_err(|error| error.to_string())?;
            if matches_monitor(&name.to_string(), id) && found.replace(object).is_some() {
                return Ok(None);
            }
        }
    }
}

const LEVELS: &str = "SELECT InstanceName, CurrentBrightness, Level, Levels FROM WmiMonitorBrightness WHERE Active = TRUE";
const METHODS: &str = "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active = TRUE";

fn read(service: &IWbemServices, id: &str) -> Result<Option<i32>, String> {
    let Some(object) = monitor_object(service, LEVELS, id)? else {
        return Ok(None);
    };
    property(&object, w!("CurrentBrightness"))
        .and_then(|value| u32::try_from(&value))
        .map(|value| (value <= 100).then_some(value as i32))
        .map_err(|error| error.to_string())
}

fn write(
    service: &IWbemServices,
    id: &str,
    value: i32,
    current: &AtomicBool,
) -> Result<(), String> {
    let levels = monitor_object(service, LEVELS, id)?.ok_or("Display is no longer available")?;
    let object =
        monitor_object(service, METHODS, id)?.ok_or("Display brightness is unavailable")?;
    let value = value.clamp(0, 100) as u32;
    let value = nearest_level(&levels, value).map_err(|error| error.to_string())?;
    // SAFETY: all WMI objects and named parameters belong to this MTA. Use the
    // provider's own object path for the matched monitor instance.
    let status = unsafe {
        (|| {
            let path = BSTR::try_from(&property(&object, w!("__RELPATH"))?)?;
            let method = BSTR::from("WmiSetBrightness");
            let mut class = None;
            service.GetObject(
                &BSTR::from("WmiMonitorBrightnessMethods"),
                Default::default(),
                None,
                Some(&mut class),
                None,
            )?;
            let mut input = None;
            let mut output = None;
            class.ok_or_else(windows::core::Error::empty)?.GetMethod(
                w!("WmiSetBrightness"),
                0,
                &mut input,
                &mut output,
            )?;
            let parameters = input
                .ok_or_else(windows::core::Error::empty)?
                .SpawnInstance(0)?;
            // WMI represents CIM uint32 inputs as VT_I4.
            parameters.Put(w!("Timeout"), 0, &VARIANT::from(0i32), 0)?;
            parameters.Put(w!("Brightness"), 0, &VARIANT::from(value as u8), 0)?;
            if !current.load(Ordering::Acquire) {
                return Ok(0);
            }
            service.ExecMethod(
                &path,
                &method,
                Default::default(),
                None,
                &parameters,
                Some(&mut output),
                None,
            )?;
            u32::try_from(&property(
                &output.ok_or_else(windows::core::Error::empty)?,
                w!("ReturnValue"),
            )?)
        })()
        .map_err(|error| error.to_string())?
    };
    if status == 0 {
        Ok(())
    } else {
        Err(format!("Brightness provider returned {status}"))
    }
}

fn nearest_level(object: &IWbemClassObject, requested: u32) -> windows::core::Result<u32> {
    let count = u32::try_from(&property(object, w!("Levels"))?)?.min(101);
    let values = property(object, w!("Level"))?;
    let mut levels = [0; 101];
    let mut returned = 0;
    // SAFETY: the slice has space for the provider's reported number of levels.
    if unsafe { VariantToUInt32Array(&values, &mut levels[..count as usize], &mut returned) }
        .is_ok()
    {
        Ok(levels
            .into_iter()
            .take(returned.min(count) as usize)
            .filter(|level| *level <= 100)
            .min_by_key(|level| level.abs_diff(requested))
            .unwrap_or(requested))
    } else {
        Ok(requested)
    }
}
