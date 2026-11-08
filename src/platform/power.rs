//! Window-delivered Windows power events. No polling or worker is necessary.
use crate::services::battery::BatterySnapshot;
use windows::Win32::{
    Foundation::{HANDLE, HWND},
    System::{
        Power::{
            GetSystemPowerStatus, HPOWERNOTIFY, RegisterPowerSettingNotification,
            SYSTEM_POWER_STATUS, UnregisterPowerSettingNotification,
        },
        SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING},
    },
    UI::WindowsAndMessaging::{
        DEVICE_NOTIFY_WINDOW_HANDLE, PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC,
        PBT_APMRESUMESUSPEND, PBT_POWERSETTINGCHANGE, WM_POWERBROADCAST,
    },
};

pub struct PowerNotifications {
    handles: Vec<HPOWERNOTIFY>,
}
impl PowerNotifications {
    pub fn attach(hwnd: HWND) -> Result<Self, String> {
        let mut subscriptions = Self {
            handles: Vec::with_capacity(2),
        };
        for guid in [&GUID_ACDC_POWER_SOURCE, &GUID_BATTERY_PERCENTAGE_REMAINING] {
            // SAFETY: caller supplies the live UI-thread HWND. Registration
            // lifetime is bounded by this owner, dropped before window teardown.
            let handle = unsafe {
                RegisterPowerSettingNotification(HANDLE(hwnd.0), guid, DEVICE_NOTIFY_WINDOW_HANDLE)
            }
            .map_err(|e| e.to_string())?;
            subscriptions.handles.push(handle);
        }
        Ok(subscriptions)
    }
}
impl Drop for PowerNotifications {
    fn drop(&mut self) {
        for handle in self.handles.drain(..) {
            // SAFETY: each handle was returned by a successful registration and
            // is unregistered exactly once, including partially failed setup.
            let _ = unsafe { UnregisterPowerSettingNotification(handle) };
        }
    }
}
pub fn snapshot() -> Result<BatterySnapshot, String> {
    let mut raw = SYSTEM_POWER_STATUS::default();
    // SAFETY: raw is a correctly-sized initialized output buffer and remains
    // alive for the synchronous kernel query.
    unsafe { GetSystemPowerStatus(&mut raw) }.map_err(|e| e.to_string())?;
    Ok(BatterySnapshot::from_raw(
        raw.ACLineStatus,
        raw.BatteryFlag,
        raw.BatteryLifePercent,
    ))
}
pub fn is_power_event(message: u32, wparam: usize) -> bool {
    message == WM_POWERBROADCAST
        && matches!(
            wparam as u32,
            PBT_POWERSETTINGCHANGE
                | PBT_APMPOWERSTATUSCHANGE
                | PBT_APMRESUMEAUTOMATIC
                | PBT_APMRESUMESUSPEND
        )
}
