//! Window-delivered Windows power events. No polling or worker is necessary.
use crate::services::battery::BatterySnapshot;
use windows::Win32::{
    Foundation::{CloseHandle, ERROR_SUCCESS, GetLastError, HANDLE, HWND},
    Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    },
    System::{
        Power::{
            GetSystemPowerStatus, HPOWERNOTIFY, RegisterPowerSettingNotification,
            SYSTEM_POWER_STATUS, SetSuspendState, UnregisterPowerSettingNotification,
        },
        Shutdown::{
            EWX_POWEROFF, EWX_REBOOT, ExitWindowsEx, LockWorkStation, SHTDN_REASON_FLAG_PLANNED,
            SHTDN_REASON_MAJOR_APPLICATION, SHTDN_REASON_MINOR_MAINTENANCE,
        },
        SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING},
        Threading::{GetCurrentProcess, OpenProcessToken},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Lock,
    Sleep,
    Restart,
    ShutDown,
}

pub fn request(action: Action) -> windows::core::Result<()> {
    // SAFETY: only the interactive user's session is targeted. The controller
    // serializes requests and checks confirmation before calling this function.
    unsafe {
        if action == Action::Lock {
            return LockWorkStation();
        }
    }
    let _privilege = ShutdownPrivilege::acquire()?;
    // SAFETY: the current token has the scoped Windows privilege. Keep wake
    // events enabled and allow applications to respond normally to shutdown.
    unsafe {
        if action == Action::Sleep {
            if SetSuspendState(false, false, false) {
                Ok(())
            } else {
                Err(windows::core::Error::from_thread())
            }
        } else {
            ExitWindowsEx(
                if action == Action::Restart {
                    EWX_REBOOT
                } else {
                    EWX_POWEROFF
                },
                SHTDN_REASON_MAJOR_APPLICATION
                    | SHTDN_REASON_MINOR_MAINTENANCE
                    | SHTDN_REASON_FLAG_PLANNED,
            )
        }
    }
}

struct ShutdownPrivilege {
    token: HANDLE,
    previous: TOKEN_PRIVILEGES,
}

impl ShutdownPrivilege {
    fn acquire() -> windows::core::Result<Self> {
        let mut privilege = Self {
            token: HANDLE::default(),
            previous: TOKEN_PRIVILEGES::default(),
        };
        // SAFETY: all buffers are initialized and match the required Win32
        // layout. The handle is closed and any change is restored on every exit.
        unsafe {
            let mut luid = Default::default();
            LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &mut luid)?;
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut privilege.token,
            )?;
            let desired = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut returned = 0;
            AdjustTokenPrivileges(
                privilege.token,
                false,
                Some(&desired),
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                Some(&mut privilege.previous),
                Some(&mut returned),
            )?;
            let status = GetLastError();
            if status != ERROR_SUCCESS {
                return Err(windows::core::Error::from_hresult(status.to_hresult()));
            }
        }
        Ok(privilege)
    }
}

impl Drop for ShutdownPrivilege {
    fn drop(&mut self) {
        if self.token.is_invalid() {
            return;
        }
        // SAFETY: token belongs to this guard; the previous state was filled by
        // AdjustTokenPrivileges when a privilege changed.
        unsafe {
            if self.previous.PrivilegeCount != 0 {
                let _ =
                    AdjustTokenPrivileges(self.token, false, Some(&self.previous), 0, None, None);
            }
            let _ = CloseHandle(self.token);
        }
    }
}
