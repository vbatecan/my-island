//! Display identity and enumeration, queried only on configuration/display events.
use ::windows::Win32::{Foundation::*, Graphics::Gdi::*, UI::HiDpi::*};
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

/// Effective monitor scaling is shared by native positioning and logical sizing.
pub(crate) fn scale(monitor: HMONITOR) -> f64 {
    let mut dpi_x = 96;
    let mut dpi_y = 96;
    // SAFETY: the monitor came from synchronous enumeration; outputs are live
    // stack values. If it disappears, initialized 96-DPI values remain the fallback.
    unsafe {
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    dpi_scale(dpi_x)
}

fn dpi_scale(dpi: u32) -> f64 {
    f64::from(dpi.max(96)) / 96.
}

/// User zoom is independent of Windows DPI. Never replace the OS scale factor:
/// the backend still needs it to transform physical pointer coordinates correctly.
pub(crate) fn user_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        scale.clamp(0.75, 1.5)
    } else {
        1.
    }
}

pub(crate) fn canvas_size(dpi_scale: f64, zoom: f64, width: f64, height: f64) -> (i32, i32) {
    let effective = dpi_scale * user_scale(zoom);
    (
        (canvas_extent(width, crate::state::OVERLAY_WIDTH) * effective).round() as i32,
        (canvas_extent(height, crate::state::OVERLAY_HEIGHT) * effective).round() as i32,
    )
}

pub(crate) fn canvas_extent(value: f64, minimum: f64) -> f64 {
    if value.is_finite() {
        value.clamp(minimum, 4096.)
    } else {
        minimum
    }
}

/// Query only on UI/configuration events; no display or DPI polling is needed.
pub(crate) fn available_width(id: &str) -> Option<f64> {
    let (monitor, info) = selected(id)?;
    let width = info.rcMonitor.right - info.rcMonitor.left;
    (width > 0).then(|| f64::from(width) / scale(monitor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_monitor_extent_becomes_logical_at_each_windows_scale() {
        for (dpi, expected) in [(96, 2560.), (120, 2048.), (144, 2560. / 1.5), (192, 1280.)] {
            assert!((2560. / dpi_scale(dpi) - expected).abs() < 1e-9);
        }
        assert_eq!(dpi_scale(0), 1.);
    }

    #[test]
    fn canvas_zoom_multiplies_dpi_without_affecting_design_coordinates() {
        for dpi in [1., 1.25, 1.5, 2., 2.5, 3., 4.] {
            for zoom in [0.75, 1., 1.25, 1.5] {
                let (width, height) = canvas_size(dpi, zoom, 800., 300.);
                assert!((f64::from(width) / (dpi * zoom) - 800.).abs() < 1.);
                assert!((f64::from(height) / (dpi * zoom) - 300.).abs() < 1.);
            }
        }
        assert_eq!(canvas_size(1., 1.5, 800., 300.), (1200, 450));
        assert_eq!(canvas_size(1.25, 1.5, 1200., 560.), (2250, 1050));
        assert_eq!(canvas_size(1., 1., f64::NAN, -1.), (800, 300));
        assert_eq!(user_scale(f64::NAN), 1.);
        assert_eq!(user_scale(0.), 0.75);
        assert_eq!(user_scale(8.), 1.5);
    }
}
