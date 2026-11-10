use crate::{
    settings::Appearance,
    shape::{FormFactor, ShapeContour},
    state::{IslandState, OVERLAY_WIDTH},
};
use ::windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{
            RemoteDesktop::*,
            Threading::{
                CreateMutexW, GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
        UI::{
            Accessibility::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*,
            WindowsAndMessaging::*,
        },
    },
    core::{BOOL, PCWSTR, w},
};
use std::{cell::RefCell, mem::size_of, rc::Rc};

const TRAY: u32 = WM_APP + 31;
const REPOSITION: u32 = WM_APP + 32;
const BACKDROP_REFRESH: u32 = WM_APP + 33;
const DESKTOP_POLICY: u32 = WM_APP + 34;
const CLOSE_PANEL: u32 = WM_APP + 35;
const EDGE_HOVER: u32 = WM_APP + 36;
const HOTKEY: i32 = 0x5354;
const LAUNCHER_HOTKEY: i32 = 0x5360;
const SUBCLASS: usize = 0x4f524249;

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Show,
    Battery(crate::services::battery::BatterySnapshot),
    Reset,
    Exit,
    State(IslandState),
    DisplayChanged,
    BackdropRefresh,
    OpenSettings,
    ClosePanel,
    HotkeyToggle,
    LauncherToggle,
    EdgeHover(bool),
    DesktopPolicyChanged { covered: bool, locked: bool },
}

#[derive(Clone, Copy)]
struct Shape {
    width: f64,
    height: f64,
    radius: f64,
    offset: f64,
    topmost: bool,
    form: FormFactor,
    ui_scale: f64,
    canvas_width: f64,
    canvas_height: f64,
}
impl Default for Shape {
    fn default() -> Self {
        let geometry = IslandState::Idle.geometry();
        Self {
            width: geometry.width,
            height: geometry.height,
            radius: geometry.radius,
            offset: 6.,
            topmost: true,
            form: FormFactor::Notch,
            ui_scale: 1.,
            canvas_width: crate::state::OVERLAY_WIDTH,
            canvas_height: crate::state::OVERLAY_HEIGHT,
        }
    }
}
#[derive(Default)]
struct Native {
    hwnd: HWND,
    power: Option<super::power::PowerNotifications>,
    icon: HICON,
    shape: Shape,
    appearance: Appearance,
    handler: Option<Rc<dyn Fn(Event)>>,
    taskbar_created: u32,
    hooks: Vec<HWINEVENTHOOK>,
    refresh_pending: bool,
    reposition_pending: bool,
    policy_pending: bool,
    monitor_id: String,
    fullscreen_hide: bool,
    fullscreen_always_hide: Vec<String>,
    fullscreen_always_show: Vec<String>,
    outside_click_collapse: bool,
    shortcut_enabled: bool,
    launcher_shortcut: Option<u32>,
    keyboard_mode: bool,
    previous_foreground: HWND,
    panel_open: bool,
    mouse_hook: HHOOK,
    keyboard_hook: HHOOK,
    hidden: bool,
    desktop_covered: bool,
    edge_inside: bool,
    edge_bounds: RECT,
    locked: bool,
}
thread_local! { static NATIVE: RefCell<Native> = RefCell::new(Native::default()); }

pub struct InstanceGuard(HANDLE);
pub fn initialize() -> Result<InstanceGuard, String> {
    // SAFETY: invoked on the main thread before Slint constructs any HWND.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Explicit verification fixtures own a separate process/window and never
        // displace or send input to an existing user instance.
        let isolated = crate::app::verification_isolated_material();
        let name: Vec<u16> = if isolated {
            format!("Local\\StillIsland.MaterialProbe.{}", std::process::id())
        } else {
            "Local\\StillIsland.Singleton".into()
        }
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let mutex = CreateMutexW(None, false, windows::core::PCWSTR(name.as_ptr()))
            .map_err(|e| e.to_string())?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return Err("Still Island is already running. Use its system tray icon.".into());
        }
        Ok(InstanceGuard(mutex))
    }
}

pub fn set_event_handler(handler: Box<dyn Fn(Event)>) {
    NATIVE.with(|n| n.borrow_mut().handler = Some(Rc::from(handler)));
}
fn emit(event: Event) {
    let handler = NATIVE.with(|n| n.borrow().handler.clone());
    if let Some(handler) = handler {
        handler(event);
    }
}

pub fn attach(hwnd: isize) -> Result<(), String> {
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: caller supplies the live Slint HWND belonging to this UI thread.
    // The subclass never owns the HWND and forwards all unhandled messages.
    unsafe {
        // The overlay owns its morph animation. DWM's activation/show transition
        // can blend an opaque redirection surface into this transparent GL HWND.
        let disabled = BOOL(1);
        let _ = ::windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            ::windows::Win32::Graphics::Dwm::DWMWA_TRANSITIONS_FORCEDISABLED,
            &disabled as *const _ as _,
            size_of::<BOOL>() as u32,
        );
        let styles = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            ((styles | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) & !WS_EX_APPWINDOW.0) as isize,
        );
        let styles = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            (styles & !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_BORDER.0)) as isize,
        );
        if !SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, 0).as_bool() {
            return Err("Could not attach native overlay subclass".into());
        }
        let icon = make_icon()?;
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        NATIVE.with(|n| {
            let mut n = n.borrow_mut();
            n.hwnd = hwnd;
            n.icon = icon;
            n.taskbar_created = taskbar_created;
        });
        let hooks = [
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_MOVESIZEEND,
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_HIDE,
            EVENT_OBJECT_DESTROY,
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
        ]
        .into_iter()
        .map(|event| {
            SetWinEventHook(
                event,
                event,
                None,
                Some(backdrop_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        })
        .filter(|hook| !hook.is_invalid())
        .collect();
        NATIVE.with(|n| n.borrow_mut().hooks = hooks);
        NATIVE.with(|n| n.borrow_mut().power = super::power::PowerNotifications::attach(hwnd).ok());
        add_tray()?;
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).map_err(|e| e.to_string())?;
        position();
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // A narrow bridge from the resolved UI contour and canvas.
pub fn update_shape(
    width: f64,
    height: f64,
    radius: f64,
    offset: f64,
    topmost: bool,
    form: FormFactor,
    ui_scale: f64,
    canvas_width: f64,
    canvas_height: f64,
) {
    let ui_scale = super::display::user_scale(ui_scale);
    let canvas_width = super::display::canvas_extent(canvas_width, crate::state::OVERLAY_WIDTH);
    let canvas_height = super::display::canvas_extent(canvas_height, crate::state::OVERLAY_HEIGHT);
    let offset = if form == FormFactor::Notch {
        0.
    } else {
        offset
    };
    let previous = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        let old = n.shape;
        n.shape = Shape {
            width,
            height,
            radius,
            offset,
            topmost,
            form,
            ui_scale,
            canvas_width,
            canvas_height,
        };
        old
    });
    if previous.offset != offset
        || previous.topmost != topmost
        || previous.form != form
        || previous.ui_scale != ui_scale
        || previous.canvas_width != canvas_width
        || previous.canvas_height != canvas_height
    {
        position();
    } else if previous.width != width || previous.height != height || previous.radius != radius {
        region();
    }
}

/// Apply desktop preferences without timers or changing any Windows preferences.
pub fn configure_desktop(
    monitor_id: &str,
    fullscreen_hide: bool,
    outside_click_collapse: bool,
    shortcut_enabled: bool,
) -> Result<(), String> {
    let (hwnd, was_enabled) = NATIVE.with(|n| {
        let n = n.borrow();
        (n.hwnd, n.shortcut_enabled)
    });
    if shortcut_enabled != was_enabled {
        // SAFETY: UI-owned HWND; errors (including another app owning the chord)
        // are surfaced. MOD_NOREPEAT avoids auto-repeat animation reversals.
        unsafe {
            if shortcut_enabled {
                RegisterHotKey(
                    Some(hwnd),
                    HOTKEY,
                    MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                    VK_SPACE.0 as u32,
                )
                .map_err(|e| format!("Ctrl+Alt+Space unavailable: {e}"))?;
            } else {
                UnregisterHotKey(Some(hwnd), HOTKEY).map_err(|e| e.to_string())?;
            }
        }
    }
    NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        n.monitor_id = monitor_id.into();
        n.fullscreen_hide = fullscreen_hide;
        n.outside_click_collapse = outside_click_collapse;
        n.shortcut_enabled = shortcut_enabled;
    });
    refresh_input_hooks()?;
    position();
    queue_policy();
    Ok(())
}

/// Raw selected HMONITOR for creating a matching Windows Graphics Capture item.
pub fn selected_monitor_handle() -> isize {
    let id = NATIVE.with(|n| n.borrow().monitor_id.clone());
    super::display::selected(&id)
        .map(|m| m.0.0 as isize)
        .unwrap_or(0)
}

/// Selected monitor width in logical pixels. Called when targets/display
/// configuration change, never by an idle timer or rendering loop.
pub fn available_width() -> f64 {
    let id = NATIVE.with(|n| n.borrow().monitor_id.clone());
    super::display::available_width(&id).unwrap_or(OVERLAY_WIDTH)
}

pub fn set_panel_open(open: bool) -> Result<(), String> {
    NATIVE.with(|n| n.borrow_mut().panel_open = open);
    refresh_input_hooks()
}

/// Register a replacement before releasing the old chord, preserving a working
/// registration if another program owns the requested shortcut.
pub fn configure_launcher_shortcut(enabled: bool, chord: u32) -> Result<(), String> {
    let desired = enabled.then_some(chord.min(2));
    let (hwnd, previous) = NATIVE.with(|n| {
        let n = n.borrow();
        (n.hwnd, n.launcher_shortcut)
    });
    if desired == previous {
        return Ok(());
    }
    // SAFETY: registrations belong to this thread's live overlay HWND. IDs are
    // disjoint from the ordinary island hotkey and paired with unregistration.
    unsafe {
        if let Some(index) = desired {
            let modifiers = match index {
                1 => MOD_CONTROL,
                2 => MOD_ALT | MOD_SHIFT,
                _ => MOD_ALT,
            };
            RegisterHotKey(
                Some(hwnd),
                LAUNCHER_HOTKEY + index as i32,
                modifiers | MOD_NOREPEAT,
                VK_SPACE.0 as u32,
            )
            .map_err(|e| {
                format!("Launcher shortcut unavailable (choose another in Settings): {e}")
            })?;
        }
        if let Some(index) = previous {
            let _ = UnregisterHotKey(Some(hwnd), LAUNCHER_HOTKEY + index as i32);
        }
    }
    NATIVE.with(|n| n.borrow_mut().launcher_shortcut = desired);
    Ok(())
}

/// Only an explicit launcher invocation may activate the overlay. Ordinary
/// island mode always regains WS_EX_NOACTIVATE. Never attach input threads or
/// synthesize keys to bypass Windows foreground restrictions.
pub fn set_keyboard_mode(active: bool, restore_focus: bool) -> Result<(), String> {
    let (hwnd, previous) = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        if active && !n.keyboard_mode {
            n.previous_foreground = unsafe { GetForegroundWindow() };
        }
        n.keyboard_mode = active;
        (n.hwnd, n.previous_foreground)
    });
    // SAFETY: UI-thread HWND; changing styles does not transfer ownership. The
    // saved HWND is validated and only restored while our window owns focus.
    unsafe {
        let foreground = GetForegroundWindow();
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            if active {
                style & !WS_EX_NOACTIVATE.0
            } else {
                style | WS_EX_NOACTIVATE.0
            } as isize,
        );
        if active {
            // The persistent overlay is normally already visible. Re-showing
            // it during activation can expose the native class background
            // before the transparent GPU surface is presented.
            if !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            if !SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() != hwnd {
                NATIVE.with(|n| n.borrow_mut().keyboard_mode = false);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_NOACTIVATE.0) as isize);
                return Err("Windows did not grant launcher keyboard focus. Try the launcher shortcut again.".into());
            }
            let _ = SetFocus(Some(hwnd));
        } else if restore_focus
            && foreground == hwnd
            && previous != hwnd
            && IsWindow(Some(previous)).as_bool()
        {
            let _ = SetForegroundWindow(previous);
        }
    }
    refresh_input_hooks()
}

pub fn launcher_has_foreground() -> bool {
    let hwnd = NATIVE.with(|n| n.borrow().hwnd);
    // SAFETY: read-only query; both handles remain owned by Windows.
    unsafe { !hwnd.is_invalid() && GetForegroundWindow() == hwnd }
}

fn refresh_input_hooks() -> Result<(), String> {
    let (mouse_needed, keyboard_needed, mouse, keyboard) = NATIVE.with(|n| {
        let n = n.borrow();
        (
            !n.locked
                && (n.desktop_covered
                    || (n.panel_open
                        && (n.outside_click_collapse || n.keyboard_mode)
                        && !n.hidden)),
            n.panel_open && !n.keyboard_mode && !n.hidden,
            n.mouse_hook,
            n.keyboard_hook,
        )
    });
    // SAFETY: low-level hooks execute on this UI thread; they only enqueue a
    // message, always forward input, and never retain the callback data pointer.
    unsafe {
        if !mouse_needed && !mouse.is_invalid() {
            UnhookWindowsHookEx(mouse).map_err(|e| e.to_string())?;
            NATIVE.with(|n| n.borrow_mut().mouse_hook = HHOOK::default());
        }
        if !keyboard_needed && !keyboard.is_invalid() {
            UnhookWindowsHookEx(keyboard).map_err(|e| e.to_string())?;
            NATIVE.with(|n| n.borrow_mut().keyboard_hook = HHOOK::default());
        }
        if mouse_needed && mouse.is_invalid() {
            let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_input), None, 0)
                .map_err(|e| format!("Outside-click observation unavailable: {e}"))?;
            NATIVE.with(|n| n.borrow_mut().mouse_hook = hook);
        }
        if keyboard_needed && keyboard.is_invalid() {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_input), None, 0)
                .map_err(|e| format!("Escape observation unavailable: {e}"))?;
            NATIVE.with(|n| n.borrow_mut().keyboard_hook = hook);
        }
    }
    Ok(())
}

unsafe extern "system" fn mouse_input(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: Windows supplies MSLLHOOKSTRUCT only for nonnegative hook codes.
    unsafe {
        if code >= 0 && w.0 as u32 == WM_MOUSEMOVE {
            let point = (*(l.0 as *const MSLLHOOKSTRUCT)).pt;
            let edge = NATIVE.with(|n| {
                let mut n = n.borrow_mut();
                let inside = n.desktop_covered && !n.locked && at_top_edge(point, n.edge_bounds);
                if inside == n.edge_inside {
                    return None;
                }
                n.edge_inside = inside;
                Some((n.hwnd, inside))
            });
            if let Some((hwnd, inside)) = edge {
                let _ = PostMessageW(Some(hwnd), EDGE_HOVER, WPARAM(inside as usize), LPARAM(0));
            }
        }
        if code >= 0
            && NATIVE.with(|n| {
                let n = n.borrow();
                n.panel_open && !n.hidden && (n.outside_click_collapse || n.keyboard_mode)
            })
            && matches!(
                w.0 as u32,
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
            )
        {
            let point = (*(l.0 as *const MSLLHOOKSTRUCT)).pt;
            let hwnd = NATIVE.with(|n| n.borrow().hwnd);
            let target = WindowFromPoint(point);
            if target != hwnd && GetAncestor(target, GA_ROOT) != hwnd {
                let _ = PostMessageW(Some(hwnd), CLOSE_PANEL, WPARAM(0), LPARAM(0));
            }
        }
        CallNextHookEx(None, code, w, l)
    }
}
fn at_top_edge(point: POINT, bounds: RECT) -> bool {
    bounds.right > bounds.left
        && bounds.bottom > bounds.top
        && point.x >= bounds.left
        && point.x < bounds.right
        && point.y >= bounds.top
        && point.y < bounds.top + 2
}
#[cfg(test)]
mod edge_tests {
    use super::*;
    #[test]
    fn top_edge_is_limited_to_selected_monitor_and_supports_negative_origins() {
        let bounds = RECT {
            left: -1920,
            top: -1080,
            right: 0,
            bottom: 0,
        };
        assert!(at_top_edge(POINT { x: -1920, y: -1080 }, bounds));
        assert!(at_top_edge(POINT { x: -1, y: -1079 }, bounds));
        assert!(!at_top_edge(POINT { x: 0, y: -1080 }, bounds));
        assert!(!at_top_edge(POINT { x: -100, y: -1078 }, bounds));
        assert!(!at_top_edge(POINT { x: -100, y: -1081 }, bounds));
        assert!(!at_top_edge(POINT { x: 0, y: 0 }, RECT::default()));
    }
}
unsafe extern "system" fn keyboard_input(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: Windows supplies KBDLLHOOKSTRUCT only for nonnegative hook codes.
    unsafe {
        if code >= 0
            && w.0 as u32 == WM_KEYDOWN
            && (*(l.0 as *const KBDLLHOOKSTRUCT)).vkCode == VK_ESCAPE.0 as u32
        {
            let hwnd = NATIVE.with(|n| n.borrow().hwnd);
            let _ = PostMessageW(Some(hwnd), CLOSE_PANEL, WPARAM(0), LPARAM(0));
        }
        CallNextHookEx(None, code, w, l)
    }
}

pub fn prefers_reduced_motion() -> bool {
    let mut enabled = BOOL(1);
    // SAFETY: SPI_GET only reads an OS preference into a correctly sized BOOL.
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && !enabled.as_bool()
    }
}

pub fn notification_allowed() -> bool {
    let unavailable = NATIVE.with(|n| {
        let n = n.borrow();
        n.locked
    });
    !unavailable
        && unsafe {
            SHQueryUserNotificationState().is_ok_and(|state| {
                matches!(
                    state,
                    QUNS_ACCEPTS_NOTIFICATIONS
                        | QUNS_RUNNING_D3D_FULL_SCREEN
                        | QUNS_PRESENTATION_MODE
                )
            })
        }
}

fn queue_policy() {
    let hwnd = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        if n.policy_pending || n.hwnd.is_invalid() {
            return None;
        }
        n.policy_pending = true;
        Some(n.hwnd)
    });
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = PostMessageW(Some(hwnd), DESKTOP_POLICY, WPARAM(0), LPARAM(0));
        }
    }
}

fn refresh_desktop_policy() {
    let (hwnd, id, fullscreen_hide, locked, was_hidden, always_hide, always_show) =
        NATIVE.with(|n| {
            let mut n = n.borrow_mut();
            n.policy_pending = false;
            (
                n.hwnd,
                n.monitor_id.clone(),
                n.fullscreen_hide,
                n.locked,
                n.hidden,
                n.fullscreen_always_hide.clone(),
                n.fullscreen_always_show.clone(),
            )
        });
    // Query actual foreground/owner and monitor surfaces, not the last window
    // event's source. A background fullscreen surface on another monitor still
    // needs protection when keyboard focus moves away from that monitor.
    let selected = super::display::selected(&id);
    let covered = selected.is_some_and(|(monitor, info)| unsafe {
        monitor_has_fullscreen(
            hwnd,
            monitor,
            info.rcMonitor,
            fullscreen_hide,
            &always_hide,
            &always_show,
        )
    });
    NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        n.desktop_covered = covered;
        n.edge_bounds = selected.map(|(_, info)| info.rcMonitor).unwrap_or_default();
        if !covered || locked {
            n.edge_inside = false;
        }
    });
    let _ = refresh_input_hooks();
    if locked {
        set_overlay_visible(false);
    }
    let _ = was_hidden;
    emit(Event::DesktopPolicyChanged { covered, locked });
}

/// Actual visibility is independent of fullscreen policy during event previews.
pub fn set_overlay_visible(visible: bool) {
    let (hwnd, hidden) = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        n.hidden = !visible || n.locked;
        (n.hwnd, n.hidden)
    });
    unsafe {
        let _ = ShowWindow(hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
    }
    let _ = refresh_input_hooks();
}

/// Explicit per-executable compatibility overrides. Session lock always wins.
pub fn set_fullscreen_overrides(
    always_hide: &[String],
    always_show: &[String],
) -> Result<(), String> {
    let normalize = |values: &[String]| -> Result<Vec<String>, String> {
        if values.len() > 64 {
            return Err("At most 64 application names are supported per fullscreen list.".into());
        }
        let mut result = Vec::new();
        for value in values {
            let name = normalize_fullscreen_executable(value).ok_or(
                "Use executable names such as game.exe, without a directory or arguments.",
            )?;
            if !result.contains(&name) {
                result.push(name);
            }
        }
        Ok(result)
    };
    let hide = normalize(always_hide)?;
    let show = normalize(always_show)?;
    NATIVE.with(|native| {
        let mut native = native.borrow_mut();
        native.fullscreen_always_hide = hide;
        native.fullscreen_always_show = show;
    });
    queue_policy();
    Ok(())
}

pub fn normalize_fullscreen_executable(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 260
        || value
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return None;
    }
    let mut value = value.to_lowercase();
    if std::path::Path::new(&value).extension().is_none() {
        value.push_str(".exe");
    }
    Some(value)
}

fn app_override(name: Option<&str>, hide: &[String], show: &[String]) -> Option<bool> {
    let name = name?;
    if hide.iter().any(|entry| entry == name) {
        Some(true)
    } else if show.iter().any(|entry| entry == name) {
        Some(false)
    } else {
        None
    }
}

fn executable_name(window: HWND) -> Option<String> {
    // SAFETY: read-only, limited-information process handle; all buffers are live.
    // Close the handle before converting the owned UTF-16 result to a basename.
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = [0_u16; 32768];
        let mut length = path.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            ::windows::core::PWSTR(path.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        let path = String::from_utf16_lossy(&path[..length as usize]);
        let name = std::path::Path::new(&path).file_name()?.to_str()?;
        normalize_fullscreen_executable(name)
    }
}

/// A few physical pixels account for DWM/DPI edge rounding without treating a
/// normal taskbar-sized work area as the whole display.
fn covers_monitor(surface: RECT, monitor: RECT) -> bool {
    surface.right > surface.left
        && surface.bottom > surface.top
        && surface.left <= monitor.left + 2
        && surface.top <= monitor.top + 2
        && surface.right >= monitor.right - 2
        && surface.bottom >= monitor.bottom - 2
}

fn intersects_monitor(surface: RECT, monitor: RECT) -> bool {
    surface.left < monitor.right
        && surface.right > monitor.left
        && surface.top < monitor.bottom
        && surface.bottom > monitor.top
}

struct FullscreenQuery {
    overlay: HWND,
    monitor: RECT,
    monitor_handle: HMONITOR,
    foreground_owner: HWND,
    foreground_on_monitor: bool,
    own_process: u32,
    covered: bool,
    automatic: bool,
    always_hide: Vec<String>,
    always_show: Vec<String>,
    matched_override: Option<bool>,
}

unsafe extern "system" fn fullscreen_candidate(candidate: HWND, data: LPARAM) -> BOOL {
    // SAFETY: EnumWindows calls synchronously with our live stack query. Every
    // Win32 output is initialized; no HWND or pointer is retained past this call.
    unsafe {
        let query = &mut *(data.0 as *mut FullscreenQuery);
        let mut process = 0;
        GetWindowThreadProcessId(candidate, Some(&mut process));
        if candidate == query.overlay
            || process == query.own_process
            || !IsWindowVisible(candidate).as_bool()
            || IsIconic(candidate).as_bool()
            || GetWindowLongPtrW(candidate, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0
        {
            return BOOL(1);
        }
        if query.foreground_on_monitor
            && GetAncestor(candidate, GA_ROOTOWNER) != query.foreground_owner
        {
            return BOOL(1);
        }
        let mut class = [0_u16; 128];
        let count = GetClassNameW(candidate, &mut class).max(0) as usize;
        if matches!(
            String::from_utf16_lossy(&class[..count]).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return BOOL(1);
        }
        let mut cloaked = 0_u32;
        let _ = ::windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            candidate,
            ::windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return BOOL(1);
        }
        let mut outer = RECT::default();
        if GetWindowRect(candidate, &mut outer).is_err()
            || !intersects_monitor(outer, query.monitor)
        {
            return BOOL(1);
        }
        // EnumWindows traverses z order. With focus on a different monitor, the
        // first visible app touching this monitor wins; never discover a buried
        // fullscreen window underneath an ordinary window.
        let continue_search = query.foreground_on_monitor;
        if (!query.always_hide.is_empty() || !query.always_show.is_empty())
            && let Some(value) = app_override(
                executable_name(candidate).as_deref(),
                &query.always_hide,
                &query.always_show,
            )
        {
            query.matched_override = Some(value);
            query.covered = value;
            return BOOL(0);
        }
        if !query.automatic {
            return BOOL(continue_search as i32);
        }
        // A maximized app owns the usable display even with a visible taskbar.
        // Per-app overrides above still take precedence.
        if IsZoomed(candidate).as_bool()
            && MonitorFromWindow(candidate, MONITOR_DEFAULTTONULL) == query.monitor_handle
        {
            query.covered = true;
            return BOOL(0);
        }
        if !covers_monitor(outer, query.monitor) {
            return BOOL(continue_search as i32);
        }
        // Client coverage distinguishes true borderless fullscreen from an
        // ordinary maximized frame, especially with an auto-hidden taskbar.
        let mut client = RECT::default();
        if GetClientRect(candidate, &mut client).is_err() {
            return BOOL(continue_search as i32);
        }
        let mut origin = POINT {
            x: client.left,
            y: client.top,
        };
        if !ClientToScreen(candidate, &mut origin).as_bool() {
            return BOOL(continue_search as i32);
        }
        let client = RECT {
            left: origin.x,
            top: origin.y,
            right: origin.x + client.right - client.left,
            bottom: origin.y + client.bottom - client.top,
        };
        if covers_monitor(client, query.monitor) {
            query.covered = true;
            return BOOL(0);
        }
        BOOL(continue_search as i32)
    }
}

unsafe fn monitor_has_fullscreen(
    overlay: HWND,
    monitor: HMONITOR,
    bounds: RECT,
    automatic: bool,
    always_hide: &[String],
    always_show: &[String],
) -> bool {
    // SAFETY: all queries are read-only and EnumWindows retains no callback state.
    unsafe {
        let foreground = GetForegroundWindow();
        let mut foreground_process = 0;
        GetWindowThreadProcessId(foreground, Some(&mut foreground_process));
        let foreground_on_monitor = foreground_process != GetCurrentProcessId()
            && !foreground.is_invalid()
            && MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL) == monitor;
        let mut query = FullscreenQuery {
            overlay,
            monitor: bounds,
            monitor_handle: monitor,
            foreground_owner: GetAncestor(foreground, GA_ROOTOWNER),
            foreground_on_monitor,
            own_process: GetCurrentProcessId(),
            covered: false,
            automatic,
            always_hide: always_hide.to_vec(),
            always_show: always_show.to_vec(),
            matched_override: None,
        };
        let _ = EnumWindows(
            Some(fullscreen_candidate),
            LPARAM(&mut query as *mut _ as isize),
        );
        query.matched_override.unwrap_or_else(|| {
            query.covered
                || (query.automatic
                    && foreground_on_monitor
                    && GetWindowLongPtrW(foreground, GWL_STYLE) as u32
                        & (WS_CAPTION.0 | WS_THICKFRAME.0)
                        == 0
                    && SHQueryUserNotificationState().is_ok_and(|state| {
                        state == QUNS_PRESENTATION_MODE || state == QUNS_RUNNING_D3D_FULL_SCREEN
                    }))
        })
    }
}

/// Read-back evidence for the opt-in runtime verifier; never used as a timer.
pub fn diagnostic_snapshot() -> String {
    let (hwnd, shape) = NATIVE.with(|n| {
        let n = n.borrow();
        (n.hwnd, n.shape)
    });
    if hwnd.is_invalid() {
        return "native: HWND not attached".into();
    }
    // SAFETY: this diagnostic runs on the owning UI thread with stack output
    // buffers. The temporary copied region is owned here and deleted exactly once.
    unsafe {
        let mut rect = RECT::default();
        let rect_ok = GetWindowRect(hwnd, &mut rect).is_ok();
        let dpi = GetDpiForWindow(hwnd);
        let scale = dpi.max(96) as f64 / 96. * shape.ui_scale;
        let styles = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let foreground = GetForegroundWindow() == hwnd;
        let mut backdrop = 0i32;
        let _ = ::windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
            hwnd,
            ::windows::Win32::Graphics::Dwm::DWMWA_SYSTEMBACKDROP_TYPE,
            &mut backdrop as *mut _ as _,
            size_of::<i32>() as u32,
        );
        let mut affinity = 0u32;
        let _ = GetWindowDisplayAffinity(hwnd, &mut affinity);
        let rgn = CreateRectRgn(0, 0, 0, 0);
        let region_kind = GetWindowRgn(hwnd, rgn);
        let center_x = (shape.canvas_width * 0.5 * scale).round() as i32;
        let center_y = (shape.height * 0.5 * scale).round() as i32;
        let center_in = PtInRegion(rgn, center_x, center_y).as_bool();
        let corner_in = PtInRegion(rgn, 0, 0).as_bool();
        let center_target = WindowFromPoint(POINT {
            x: rect.left + center_x,
            y: rect.top + center_y,
        });
        let center_hit = center_target == hwnd;
        let mut target_class = [0u16; 256];
        let class_len = GetClassNameW(center_target, &mut target_class).max(0) as usize;
        let class_name = String::from_utf16_lossy(&target_class[..class_len]);
        let mut target_pid = 0;
        GetWindowThreadProcessId(center_target, Some(&mut target_pid));
        let corner_hit = WindowFromPoint(POINT {
            x: rect.left + 1,
            y: rect.top + 1,
        }) == hwnd;
        let _ = DeleteObject(HGDIOBJ(rgn.0));
        format!(
            "native: hwnd={:#x} rect_ok={} rect=({},{})-({},{}) dpi={} ui_scale={} exstyles={:#x} toolwindow={} noactivate={} appwindow={} topmost={} foreground={} region_kind={} region_center={} region_canvas_corner={} hit_center={} hit_canvas_corner={} backdrop={} affinity={:#x} center_target={:#x} center_class={} center_pid={}",
            hwnd.0 as usize,
            rect_ok,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            dpi,
            shape.ui_scale,
            styles,
            styles & WS_EX_TOOLWINDOW.0 != 0,
            styles & WS_EX_NOACTIVATE.0 != 0,
            styles & WS_EX_APPWINDOW.0 != 0,
            styles & WS_EX_TOPMOST.0 != 0,
            foreground,
            region_kind.0,
            center_in,
            corner_in,
            center_hit,
            corner_hit,
            backdrop,
            affinity,
            center_target.0 as usize,
            class_name,
            target_pid
        )
    }
}

fn position() {
    let (hwnd, shape) = NATIVE.with(|n| {
        let n = n.borrow();
        (n.hwnd, n.shape)
    });
    if hwnd.is_invalid() {
        return;
    }
    // SAFETY: stack output buffers are initialized; selected monitor handle remains
    // valid for the duration of the call. SetWindowPos never activates the overlay.
    unsafe {
        let id = NATIVE.with(|n| n.borrow().monitor_id.clone());
        let Some((monitor, info)) = super::display::selected(&id) else {
            return;
        };
        let dpi_scale = super::display::scale(monitor);
        let (width, height) = super::display::canvas_size(
            dpi_scale,
            shape.ui_scale,
            shape.canvas_width,
            shape.canvas_height,
        );
        let x = info.rcMonitor.left + (info.rcMonitor.right - info.rcMonitor.left - width) / 2;
        let y = info.rcMonitor.top
            + (if shape.form == FormFactor::Notch {
                0.
            } else {
                shape.offset
            } * dpi_scale)
                .round() as i32;
        let _ = SetWindowPos(
            hwnd,
            Some(if shape.topmost {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    region();
}

fn region() {
    let (hwnd, shape) = NATIVE.with(|n| {
        let n = n.borrow();
        (n.hwnd, n.shape)
    });
    if hwnd.is_invalid() {
        return;
    }
    // SAFETY: successful SetWindowRgn transfers ownership to Windows. On failure
    // we delete the unowned region. The one-pixel margin preserves antialiasing.
    // A window region excludes desktop pixels from both painting and input,
    // including clicks to other processes (HTTRANSPARENT alone cannot do this).
    unsafe {
        let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96. * shape.ui_scale;
        let contour = ShapeContour::new(shape.width, shape.height, shape.radius, shape.form);
        let raw = contour.flattened(scale);
        let left = (shape.canvas_width - contour.total_width()) * scale / 2.;
        let count = raw.len();
        let points: Vec<POINT> = raw
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // Expand one physical pixel along the outward polygon bisector,
                // preserving the antialias fringe without a rectangular input mask.
                let previous = raw[(i + count - 1) % count];
                let next = raw[(i + 1) % count];
                let v = [next[0] - previous[0], next[1] - previous[1]];
                let length = v[0].hypot(v[1]).max(1e-6);
                POINT {
                    x: (left + p[0] + v[1] / length).round() as i32,
                    y: (p[1] - v[0] / length).round() as i32,
                }
            })
            .collect();
        let r = CreatePolygonRgn(&points, WINDING);
        if r.0.is_null() {
            return;
        }
        if SetWindowRgn(hwnd, Some(r), false) == 0 {
            let _ = DeleteObject(HGDIOBJ(r.0));
        }
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    // SAFETY: Windows invokes this only for the subclassed UI-thread HWND.
    unsafe {
        if super::power::is_power_event(msg, wparam.0) {
            if let Ok(snapshot) = super::power::snapshot() {
                emit(Event::Battery(snapshot));
            }
            return LRESULT(1);
        }
        match msg {
            EDGE_HOVER => {
                emit(Event::EdgeHover(wparam.0 != 0));
                return LRESULT(0);
            }
            WM_NCACTIVATE => {
                // Forward activation to Winit so its focus state stays correct,
                // but use the documented -1 to prevent DefWindowProc painting
                // native active/inactive frame colors over our frameless view.
                return DefSubclassProc(hwnd, msg, wparam, LPARAM(-1));
            }
            WM_SYSCOMMAND if (wparam.0 as u32 & 0xfff0) == SC_KEYMENU => {
                // This frameless overlay has no system menu. Alt+Space is our
                // registered launcher chord, not a request for the standard
                // opaque Windows menu (including delayed activation messages).
                return LRESULT(0);
            }
            WM_SYSCHAR if wparam.0 == VK_SPACE.0 as usize => {
                return LRESULT(0);
            }
            WM_ERASEBKGND => {
                // Slint paints the complete transparent surface. A separate
                // Win32 class-brush erase would show an opaque white frame
                // between activation/style changes and the next GPU present.
                // WM_PAINT still reaches the backend below.
                return LRESULT(1);
            }
            WM_STYLECHANGING => {
                // Winit recomputes styles when visibility or DPI changes. Keep
                // overlay invariants in the proposed styles before Windows applies
                // them, rather than fighting them with an idle polling loop.
                // SAFETY: WM_STYLECHANGING supplies a writable STYLESTRUCT for
                // the synchronous duration of this message.
                let proposed = &mut *(lparam.0 as *mut STYLESTRUCT);
                if wparam.0 as i32 == GWL_EXSTYLE.0 {
                    let activating = NATIVE.with(|n| n.borrow().keyboard_mode);
                    proposed.styleNew =
                        (proposed.styleNew | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
                    if activating {
                        proposed.styleNew &= !WS_EX_NOACTIVATE.0;
                    } else {
                        proposed.styleNew |= WS_EX_NOACTIVATE.0;
                    }
                    let topmost = NATIVE.with(|n| n.borrow().shape.topmost);
                    if topmost {
                        proposed.styleNew |= WS_EX_TOPMOST.0;
                    } else {
                        proposed.styleNew &= !WS_EX_TOPMOST.0;
                    }
                } else if wparam.0 as i32 == GWL_STYLE.0 {
                    proposed.styleNew &= !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_BORDER.0);
                    if NATIVE.with(|n| n.borrow().hidden) {
                        proposed.styleNew &= !WS_VISIBLE.0;
                    }
                }
            }
            WM_WINDOWPOSCHANGING => {
                // SAFETY: this message carries a valid writable WINDOWPOS. It
                // belongs to Windows and is not retained after the callback.
                let proposed = &mut *(lparam.0 as *mut WINDOWPOS);
                proposed.flags |= SWP_NOACTIVATE;
                // Backend DPI/style/visibility updates must not undo desktop
                // suppression after the policy has hidden this persistent HWND.
                if NATIVE.with(|n| n.borrow().hidden) {
                    proposed.flags &= !SWP_SHOWWINDOW;
                    proposed.flags |= SWP_HIDEWINDOW;
                }
            }
            WM_MOUSEACTIVATE => {
                return LRESULT(if NATIVE.with(|n| n.borrow().keyboard_mode) {
                    MA_ACTIVATE
                } else {
                    MA_NOACTIVATE
                } as isize);
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                let post = NATIVE.with(|n| {
                    let mut n = n.borrow_mut();
                    if n.reposition_pending {
                        false
                    } else {
                        n.reposition_pending = true;
                        true
                    }
                });
                if post {
                    let _ = PostMessageW(Some(hwnd), REPOSITION, WPARAM(0), LPARAM(0));
                }
                return result;
            }
            WM_HOTKEY if wparam.0 as i32 == HOTKEY => {
                emit(Event::HotkeyToggle);
                return LRESULT(0);
            }
            WM_HOTKEY if (LAUNCHER_HOTKEY..=LAUNCHER_HOTKEY + 2).contains(&(wparam.0 as i32)) => {
                emit(Event::LauncherToggle);
                return LRESULT(0);
            }
            CLOSE_PANEL => {
                emit(Event::ClosePanel);
                return LRESULT(0);
            }
            DESKTOP_POLICY => {
                refresh_desktop_policy();
                return LRESULT(0);
            }
            WM_WTSSESSION_CHANGE => {
                if wparam.0 as u32 == WTS_SESSION_LOCK {
                    NATIVE.with(|n| n.borrow_mut().locked = true);
                }
                if wparam.0 as u32 == WTS_SESSION_UNLOCK {
                    NATIVE.with(|n| n.borrow_mut().locked = false);
                }
                queue_policy();
                return LRESULT(0);
            }
            BACKDROP_REFRESH => {
                NATIVE.with(|n| n.borrow_mut().refresh_pending = false);
                emit(Event::BackdropRefresh);
                return LRESULT(0);
            }
            REPOSITION => {
                NATIVE.with(|n| n.borrow_mut().reposition_pending = false);
                position();
                queue_policy();
                emit(Event::DisplayChanged);
                return LRESULT(0);
            }
            TRAY => {
                let event = (lparam.0 as u32) & 0xffff;
                if event == WM_RBUTTONUP || event == WM_LBUTTONUP || event == WM_CONTEXTMENU {
                    popup();
                }
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                NATIVE.with(|n| n.borrow_mut().power.take());
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
            }
            _ => {}
        }
        let restored = NATIVE.with(|n| msg == n.borrow().taskbar_created);
        if restored {
            let _ = add_tray();
            return LRESULT(0);
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

fn tray_data() -> NOTIFYICONDATAW {
    let (hwnd, icon) = NATIVE.with(|n| {
        let n = n.borrow();
        (n.hwnd, n.icon)
    });
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: TRAY,
        hIcon: icon,
        ..Default::default()
    };
    let tip: Vec<u16> = "Still Island".encode_utf16().collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    data
}
fn add_tray() -> Result<(), String> {
    // SAFETY: NOTIFYICONDATAW has the exact ABI size and a valid persistent HWND/icon.
    unsafe {
        if Shell_NotifyIconW(NIM_ADD, &tray_data()).as_bool() {
            Ok(())
        } else {
            Err("Could not create system tray icon".into())
        }
    }
}

unsafe fn make_icon() -> Result<HICON, String> {
    let mut pixels = vec![0u8; 32 * 32 * 4];
    let mut mask = [255u8; 128];
    for y in 0..32usize {
        for x in 0..32usize {
            let dx = if x < 10 { 10 - x } else { x.saturating_sub(21) };
            let dy = (y as i32 - 16).unsigned_abs() as usize;
            if dx * dx + dy * dy <= 49 {
                let i = (y * 32 + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[245, 245, 245, 255]);
                mask[y * 4 + x / 8] &= !(0x80 >> (x % 8));
            }
        }
    }
    // SAFETY: 32-bit XOR bitmap and 1-bit AND bitmap have 32x32 pixels and
    // DWORD-aligned scanlines. CreateIcon copies both arrays synchronously.
    unsafe {
        CreateIcon(None, 32, 32, 1, 32, mask.as_ptr(), pixels.as_ptr()).map_err(|e| e.to_string())
    }
}

fn popup() {
    // SAFETY: menus are created and consumed on the UI thread. DestroyMenu
    // releases the root and its attached development submenu after tracking.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let hwnd = NATIVE.with(|n| n.borrow().hwnd);
        let add = |menu, id, label: &str, flags| {
            let text: Vec<u16> = label.encode_utf16().chain([0]).collect();
            AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()))
        };
        let _ = add(menu, 1, "Show Island", MF_STRING);
        let _ = add(menu, 6, "Countdown timer", MF_STRING);
        let _ = add(menu, 5, "Settings…", MF_STRING);
        let _ = add(menu, 2, "Reset Island", MF_STRING);
        if (cfg!(debug_assertions) || std::env::args().any(|a| a == "--demo"))
            && let Ok(demo) = CreatePopupMenu()
        {
            for (id, label) in [
                (20, "Idle"),
                (21, "Compact"),
                (22, "Expanded"),
                (23, "Volume"),
                (24, "Media"),
            ] {
                let _ = add(demo, id, label, MF_STRING);
            }
            if add(menu, demo.0 as usize, "Development states", MF_POPUP).is_err() {
                // Failed attachment leaves ownership with this call.
                let _ = DestroyMenu(demo);
            }
        }
        let _ = add(menu, 0, "", MF_SEPARATOR);
        let _ = add(menu, 4, "Exit", MF_STRING);
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        )
        .0;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match command {
            1 => emit(Event::Show),
            2 => emit(Event::Reset),
            4 => emit(Event::Exit),
            5 => emit(Event::OpenSettings),
            6 => emit(Event::State(IslandState::Timer)),
            20 => emit(Event::State(IslandState::Idle)),
            21 => emit(Event::State(IslandState::Compact)),
            22 => emit(Event::State(IslandState::Expanded)),
            23 => emit(Event::State(IslandState::Volume)),
            24 => emit(Event::State(IslandState::Media)),
            _ => {}
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let (hwnd, icon) = NATIVE.with(|n| {
            let mut n = n.borrow_mut();
            n.handler = None;
            n.power.take();
            for hook in n.hooks.drain(..) {
                unsafe {
                    let _ = UnhookWinEvent(hook);
                }
            }
            (n.hwnd, n.icon)
        });
        // SAFETY: resources were created by this module. Cleanup occurs on the
        // owning UI thread after Slint's event loop exits; stale HWND removal is
        // harmless. Windows owns all successfully assigned regions.
        unsafe {
            if !hwnd.is_invalid() {
                let _ = WTSUnRegisterSessionNotification(hwnd);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY);
                for index in 0..3 {
                    let _ = UnregisterHotKey(Some(hwnd), LAUNCHER_HOTKEY + index);
                }
                let (mouse, keyboard) = NATIVE.with(|n| {
                    let n = n.borrow();
                    (n.mouse_hook, n.keyboard_hook)
                });
                if !mouse.is_invalid() {
                    let _ = UnhookWindowsHookEx(mouse);
                }
                if !keyboard.is_invalid() {
                    let _ = UnhookWindowsHookEx(keyboard);
                }
                let _ = Shell_NotifyIconW(NIM_DELETE, &tray_data());
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
            }
            if !icon.is_invalid() {
                let _ = DestroyIcon(icon);
            }
            let _ = CloseHandle(self.0);
        }
    }
}

/// Preserve the backend's per-pixel-transparent composition surface.
/// Acrylic is a clipped, cached frosted texture in Slint, not a system backdrop:
/// DWMSBT_TRANSIENTWINDOW applies behind the entire persistent canvas and can
/// expose an opaque gray panel outside the currently drawn island.
pub fn set_appearance(appearance: Appearance) -> Result<(), String> {
    use ::windows::Win32::Graphics::Dwm::*;
    let hwnd = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        n.appearance = appearance;
        n.hwnd
    });
    if hwnd.is_invalid() {
        return Ok(());
    }
    // SAFETY: live UI-thread HWND and correctly sized, stack-owned DWM attribute values.
    unsafe {
        let dark = 1i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            size_of::<i32>() as u32,
        );
        let kind = DWMSBT_NONE;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &kind as *const _ as _,
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
        .map_err(|e| format!("Windows backdrop unavailable: {e}"))?;
        // Reset any previous full-frame extension. Do not change Winit's empty
        // blur-region registration: it is what preserves per-pixel alpha on its
        // transparent OpenGL window; legacy blur-behind is not Windows 11 Acrylic.
        let margins = ::windows::Win32::UI::Controls::MARGINS::default();
        DwmExtendFrameIntoClientArea(hwnd, &margins).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn capture_bounds() -> Result<(i32, i32, u32, u32), String> {
    let hwnd = NATIVE.with(|n| n.borrow().hwnd);
    let mut rect = RECT::default();
    // SAFETY: valid attached HWND and stack output rectangle.
    unsafe {
        GetWindowRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
    }
    Ok((
        rect.left,
        rect.top,
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
    ))
}
pub fn exclude_from_capture(exclude: bool) -> Result<(), String> {
    let hwnd = NATIVE.with(|n| n.borrow().hwnd);
    // SAFETY: this process owns the attached top-level window. Reversed when glass stops.
    unsafe {
        SetWindowDisplayAffinity(
            hwnd,
            if exclude {
                WDA_EXCLUDEFROMCAPTURE
            } else {
                WDA_NONE
            },
        )
        .map_err(|e| e.to_string())
    }
}

// Out-of-context callbacks are delivered on the hook-installing UI thread. Skip
// our own process to avoid a render -> location event -> capture feedback cycle.
unsafe extern "system" fn backdrop_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    source: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    // Only top-level movement affects the desktop backdrop. Child controls emit
    // many unrelated location events that must not wake the overlay.
    if source.is_invalid() {
        return;
    }
    if event == EVENT_OBJECT_LOCATIONCHANGE && unsafe { GetAncestor(source, GA_ROOT) } != source {
        return;
    }
    if event >= EVENT_OBJECT_CREATE && (object != OBJID_WINDOW.0 || child != 0) {
        return;
    }
    queue_policy();
    let hwnd = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        if !matches!(n.appearance, Appearance::Glass | Appearance::Acrylic) || n.refresh_pending {
            return None;
        }
        n.refresh_pending = true;
        Some(n.hwnd)
    });
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = PostMessageW(Some(hwnd), BACKDROP_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

/// Low-overhead, on-demand sampling of the real desktop beneath the excluded
/// overlay. Eco glass calls this on interaction/window events, never on a timer.
#[allow(dead_code)] // Retained synchronous adapter for opt-in native diagnostics.
pub fn capture_snapshot() -> Result<super::capture::BackdropFrame, String> {
    capture_snapshot_at(capture_bounds()?)
}

/// Capture a previously resolved physical-pixel rectangle. This helper does not
/// access UI-thread state: every GDI handle is created and released on its caller
/// thread, allowing snapshot sampling to run off the GUI event loop.
pub fn capture_snapshot_at(
    (left, top, width, height): (i32, i32, u32, u32),
) -> Result<super::capture::BackdropFrame, String> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Invalid desktop sample size".into());
    }
    struct Resources {
        screen: HDC,
        memory: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl Drop for Resources {
        fn drop(&mut self) {
            unsafe {
                if !self.old.is_invalid() {
                    SelectObject(self.memory, self.old);
                }
                if !self.bitmap.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
                }
                if !self.memory.is_invalid() {
                    let _ = DeleteDC(self.memory);
                }
                if !self.screen.is_invalid() {
                    ReleaseDC(None, self.screen);
                }
            }
        }
    }
    // SAFETY: all GDI handles are owned by this stack guard. The DIB's mapped memory
    // has exactly width*height*4 bytes and remains valid until copied before Drop.
    unsafe {
        let screen = GetDC(None);
        let mut resources = Resources {
            screen,
            memory: HDC::default(),
            bitmap: HBITMAP::default(),
            old: HGDIOBJ::default(),
        };
        if screen.is_invalid() {
            return Err("Desktop DC unavailable".into());
        }
        resources.memory = CreateCompatibleDC(Some(screen));
        if resources.memory.is_invalid() {
            return Err("Capture DC unavailable".into());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        resources.bitmap =
            CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .map_err(|e| e.to_string())?;
        if bits.is_null() {
            return Err("Desktop pixel buffer unavailable".into());
        }
        resources.old = SelectObject(resources.memory, HGDIOBJ(resources.bitmap.0));
        BitBlt(
            resources.memory,
            0,
            0,
            width as i32,
            height as i32,
            Some(screen),
            left,
            top,
            SRCCOPY | CAPTUREBLT,
        )
        .map_err(|e| e.to_string())?;
        let _ = GdiFlush();
        let mut pixels =
            std::slice::from_raw_parts(bits as *const u8, width as usize * height as usize * 4)
                .to_vec();
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Ok(super::capture::BackdropFrame {
            width,
            height,
            pixels,
        })
    }
}

pub fn settle_exclusion() {
    // SAFETY: waits once for DWM to apply the window's display-affinity change.
    // Used only when a material is selected, never as a frame scheduler.
    unsafe {
        let _ = ::windows::Win32::Graphics::Dwm::DwmFlush();
    }
}

/// Copy Unicode text. Windows owns the allocation only after SetClipboardData
/// succeeds; every earlier failure releases it and closes the clipboard.
pub fn copy_text(text: &str) -> Result<(), String> {
    use windows::Win32::System::{DataExchange::*, Memory::*};
    let hwnd = NATIVE.with(|n| n.borrow().hwnd);
    let data: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    // SAFETY: UI-owned HWND, moveable allocation sized for all UTF16 code units.
    unsafe {
        OpenClipboard(Some(hwnd)).map_err(|e| e.to_string())?;
        struct Close;
        impl Drop for Close {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseClipboard();
                }
            }
        }
        let _close = Close;
        let memory = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(memory);
        if ptr.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err("Cannot allocate clipboard text".into());
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast::<u16>(), data.len());
        let _ = GlobalUnlock(memory);
        let result = EmptyClipboard().and_then(|_| SetClipboardData(13, Some(HANDLE(memory.0))));
        if let Err(e) = result {
            let _ = GlobalFree(Some(memory));
            return Err(e.to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod fullscreen_tests {
    use super::*;
    #[test]
    fn executable_overrides_are_normalized_and_conflicts_hide_first() {
        assert_eq!(
            normalize_fullscreen_executable(" Game.EXE ").as_deref(),
            Some("game.exe")
        );
        assert_eq!(
            normalize_fullscreen_executable("Game").as_deref(),
            Some("game.exe")
        );
        assert_eq!(
            normalize_fullscreen_executable("My Game.exe").as_deref(),
            Some("my game.exe")
        );
        for invalid in [
            "",
            "C:\\Games\\Game.exe",
            "../game.exe",
            "game.exe\n--flag",
            "\"game.exe\"",
        ] {
            assert!(normalize_fullscreen_executable(invalid).is_none());
        }
        let hide = vec!["game.exe".to_string()];
        let show = vec!["game.exe".to_string(), "player.exe".to_string()];
        assert_eq!(app_override(Some("game.exe"), &hide, &show), Some(true));
        assert_eq!(app_override(Some("player.exe"), &hide, &show), Some(false));
        assert_eq!(app_override(Some("other.exe"), &hide, &show), None);
        assert_eq!(app_override(None, &hide, &show), None);
    }
    #[test]
    fn fullscreen_coverage_accepts_dpi_rounding_and_spanning_surfaces() {
        let monitor = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        assert!(covers_monitor(monitor, monitor));
        assert!(covers_monitor(
            RECT {
                left: -1919,
                top: 1,
                right: -1,
                bottom: 1079
            },
            monitor
        ));
        assert!(covers_monitor(
            RECT {
                left: -1920,
                top: 0,
                right: 1920,
                bottom: 1080
            },
            monitor
        ));
    }
    #[test]
    fn maximized_work_area_and_captioned_client_are_not_fullscreen() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(!covers_monitor(
            RECT {
                bottom: 1040,
                ..monitor
            },
            monitor
        ));
        assert!(!covers_monitor(RECT { top: 31, ..monitor }, monitor));
        assert!(!covers_monitor(RECT::default(), monitor));
    }
}

#[cfg(test)]
mod acrylic_native_tests {
    use super::*;

    #[test]
    #[ignore = "Requires Windows 11 with an interactive DWM desktop"]
    fn acrylic_keeps_compositor_off_canvas_and_exact_native_hit_region() {
        // A hidden owned fixture exercises the real DWM/region APIs without
        // changing user preferences, showing windows, or sending desktop input.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("Still Island acrylic clipping fixture"),
                WS_POPUP,
                0,
                0,
                1040,
                648,
                None,
                None,
                None,
                None,
            )
            .expect("create owned hidden window")
        };
        struct Fixture(HWND, Option<Native>);
        impl Drop for Fixture {
            fn drop(&mut self) {
                if let Some(previous) = self.1.take() {
                    NATIVE.with(|n| *n.borrow_mut() = previous);
                }
                // SAFETY: this fixture owns the HWND on its creating thread.
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let previous = NATIVE.with(|n| {
            n.replace(Native {
                hwnd,
                shape: Shape {
                    ui_scale: 1.3,
                    canvas_height: 498.,
                    ..Shape::default()
                },
                ..Native::default()
            })
        });
        let _fixture = Fixture(hwnd, Some(previous));
        for appearance in [
            Appearance::Solid,
            Appearance::Acrylic,
            Appearance::Transparent,
        ] {
            set_appearance(appearance).expect("configure transparent native composition");
            let mut backdrop = 0i32;
            // SAFETY: live hidden fixture and a correctly sized output integer.
            unsafe {
                ::windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                    hwnd,
                    ::windows::Win32::Graphics::Dwm::DWMWA_SYSTEMBACKDROP_TYPE,
                    &mut backdrop as *mut _ as _,
                    size_of::<i32>() as u32,
                )
                .expect("read actual compositor configuration");
            }
            assert_eq!(
                backdrop, 1,
                "no full-canvas DWM material for {appearance:?}"
            );
        }
        for (width, height, radius) in [(126., 32., 16.), (560., 400., 8.), (220., 48., 24.)] {
            NATIVE.with(|n| {
                let mut n = n.borrow_mut();
                n.shape.width = width;
                n.shape.height = height;
                n.shape.radius = radius;
            });
            region();
            // SAFETY: temporary copied region is owned here and deleted below.
            unsafe {
                let copy = CreateRectRgn(0, 0, 0, 0);
                assert!(GetWindowRgn(hwnd, copy).0 > 1);
                assert!(PtInRegion(copy, 520, (height * 0.65) as i32).as_bool());
                assert!(
                    !PtInRegion(copy, 1, 1).as_bool(),
                    "canvas corner stays outside native mask"
                );
                assert!(
                    !PtInRegion(copy, 520, (height * 1.3 + 4.) as i32).as_bool(),
                    "area below island stays outside mask"
                );
                let _ = DeleteObject(HGDIOBJ(copy.0));
            }
        }
    }
}
