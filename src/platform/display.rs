//! Display identity and enumeration, queried only on configuration/display events.
use ::windows::Win32::{Foundation::*, Graphics::Gdi::*};
use ::windows::core::BOOL;
use std::mem::size_of;

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    /// Windows display device name; remains stable across DPI/resolution changes.
    pub id: String,
    pub name: String,
    pub primary: bool,
}

pub(crate) fn entries() -> Vec<(MonitorInfo, HMONITOR, MONITORINFO)> {
    unsafe extern "system" fn enumerate(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: EnumDisplayMonitors invokes synchronously with our live Vec pointer.
        unsafe {
            let result = &mut *(data.0 as *mut Vec<(MonitorInfo, HMONITOR, MONITORINFO)>);
            let mut info = MONITORINFOEXW {
                monitorInfo: MONITORINFO {
                    cbSize: size_of::<MONITORINFOEXW>() as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
                let end = info
                    .szDevice
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(info.szDevice.len());
                let id = String::from_utf16_lossy(&info.szDevice[..end]);
                let primary = info.monitorInfo.dwFlags & 1 != 0; // MONITORINFOF_PRIMARY
                result.push((
                    MonitorInfo {
                        id: id.clone(),
                        name: format!("{}{}", id, if primary { " (primary)" } else { "" }),
                        primary,
                    },
                    monitor,
                    info.monitorInfo,
                ));
            }
            BOOL(1)
        }
    }
    let mut result = Vec::new();
    // SAFETY: callback never retains the pointer; outputs belong to this Vec.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enumerate),
            LPARAM(&mut result as *mut _ as isize),
        );
    }
    result
}

pub fn monitors() -> Vec<MonitorInfo> {
    entries().into_iter().map(|entry| entry.0).collect()
}

pub(crate) fn selected(id: &str) -> Option<(HMONITOR, MONITORINFO)> {
    let entries = entries();
    entries
        .iter()
        .find(|e| e.0.id == id)
        .or_else(|| entries.iter().find(|e| e.0.primary))
        .or_else(|| entries.first())
        .map(|e| (e.1, e.2))
}
