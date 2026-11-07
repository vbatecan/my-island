use crate::{
    settings::{Appearance, Bounce},
    state::IslandState,
};
use ::windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{RemoteDesktop::*, Threading::CreateMutexW},
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
const HOTKEY: i32 = 0x5354;
const SUBCLASS: usize = 0x4f524249;

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Show,
    Reset,
    Exit,
    State(IslandState),
    ToggleAnimations,
    ToggleTopmost,
    Offset(f64),
    Appearance(Appearance),
    Opacity(f32),
    Bounce(Bounce),
    DisplayChanged,
    BackdropRefresh,
    OpenSettings,
    ClosePanel,
    HotkeyToggle,
    DesktopPolicyChanged(bool),
}

#[derive(Clone, Copy)]
struct Shape {
    width: f64,
    height: f64,
    radius: f64,
    offset: f64,
    topmost: bool,
}
impl Default for Shape {
    fn default() -> Self {
        Self {
            width: 126.,
            height: 32.,
            radius: 16.,
            offset: 6.,
            topmost: true,
        }
    }
}
#[derive(Default)]
struct Native {
    hwnd: HWND,
    icon: HICON,
    shape: Shape,
    animations: bool,
    appearance: Appearance,
    opacity: f32,
    bounce: Bounce,
    handler: Option<Rc<dyn Fn(Event)>>,
    taskbar_created: u32,
    hooks: Vec<HWINEVENTHOOK>,
    refresh_pending: bool,
    reposition_pending: bool,
    policy_pending: bool,
    monitor_id: String,
    fullscreen_hide: bool,
    outside_click_collapse: bool,
    shortcut_enabled: bool,
    panel_open: bool,
    mouse_hook: HHOOK,
    keyboard_hook: HHOOK,
    hidden: bool,
    locked: bool,
}
thread_local! { static NATIVE: RefCell<Native> = RefCell::new(Native::default()); }

pub struct InstanceGuard(HANDLE);
pub fn initialize() -> Result<InstanceGuard, String> {
    // SAFETY: invoked on the main thread before Slint constructs any HWND.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mutex = CreateMutexW(None, false, w!("Local\\StillIsland.Singleton"))
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
            n.animations = true;
            n.taskbar_created = taskbar_created;
        });
        let hooks = [
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_MOVESIZEEND,
            EVENT_OBJECT_LOCATIONCHANGE,
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
        add_tray()?;
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).map_err(|e| e.to_string())?;
        position();
    }
    Ok(())
}

pub fn update_shape(width: f64, height: f64, radius: f64, offset: f64, topmost: bool) {
    let previous = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        let old = n.shape;
        n.shape = Shape {
            width,
            height,
            radius,
            offset,
            topmost,
        };
        old
    });
    if previous.offset != offset || previous.topmost != topmost {
        position();
    } else if previous.width != width || previous.height != height || previous.radius != radius {
        region();
    }
}

pub fn set_animation_preference(enabled: bool) {
    NATIVE.with(|n| n.borrow_mut().animations = enabled);
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

pub fn set_panel_open(open: bool) -> Result<(), String> {
    NATIVE.with(|n| n.borrow_mut().panel_open = open);
    refresh_input_hooks()
}

fn refresh_input_hooks() -> Result<(), String> {
    let (mouse_needed, keyboard_needed, mouse, keyboard) = NATIVE.with(|n| {
        let n = n.borrow();
        (
            n.panel_open && n.outside_click_collapse && !n.hidden,
            n.panel_open && !n.hidden,
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
        if code >= 0
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
        n.hidden || n.locked
    });
    !unavailable
        && unsafe {
            SHQueryUserNotificationState().is_ok_and(|state| state == QUNS_ACCEPTS_NOTIFICATIONS)
        }
}

fn queue_policy() {
    let hwnd = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        if n.policy_pending {
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
    let (hwnd, id, fullscreen_hide, locked, was_hidden) = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        n.policy_pending = false;
        (
            n.hwnd,
            n.monitor_id.clone(),
            n.fullscreen_hide,
            n.locked,
            n.hidden,
        )
    });
    let covered = unsafe {
        let foreground = GetForegroundWindow();
        let mut rect = RECT::default();
        let mut class = [0u16; 128];
        let class_len = GetClassNameW(foreground, &mut class).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..class_len]);
        let shell = matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        );
        let geometric = foreground != hwnd
            && !shell
            && !foreground.is_invalid()
            && !IsIconic(foreground).as_bool()
            && GetWindowRect(foreground, &mut rect).is_ok()
            && super::display::selected(&id).is_some_and(|(_, m)| {
                rect.left <= m.rcMonitor.left
                    && rect.top <= m.rcMonitor.top
                    && rect.right >= m.rcMonitor.right
                    && rect.bottom >= m.rcMonitor.bottom
            });
        let presentation = SHQueryUserNotificationState().is_ok_and(|state| {
            state == QUNS_PRESENTATION_MODE || state == QUNS_RUNNING_D3D_FULL_SCREEN
        });
        geometric || presentation
    };
    let hidden = locked || (fullscreen_hide && covered);
    if hidden != was_hidden {
        NATIVE.with(|n| n.borrow_mut().hidden = hidden);
        unsafe {
            let _ = ShowWindow(hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
        }
        let _ = refresh_input_hooks();
        emit(Event::DesktopPolicyChanged(hidden));
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
        let scale = dpi as f64 / 96.;
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
        let center_x = (210. * scale).round() as i32;
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
            "native: hwnd={:#x} rect_ok={} rect=({},{})-({},{}) dpi={} exstyles={:#x} toolwindow={} noactivate={} appwindow={} topmost={} foreground={} region_kind={} region_center={} region_canvas_corner={} hit_center={} hit_canvas_corner={} backdrop={} affinity={:#x} center_target={:#x} center_class={} center_pid={}",
            hwnd.0 as usize,
            rect_ok,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            dpi,
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
    // SAFETY: stack output buffers are initialized; primary monitor handle remains
    // valid for the duration of the call. SetWindowPos never activates the overlay.
    unsafe {
        let id = NATIVE.with(|n| n.borrow().monitor_id.clone());
        let Some((monitor, info)) = super::display::selected(&id) else {
            return;
        };
        let mut dpi = 96;
        let mut dy = 96;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dy);
        let scale = dpi as f64 / 96.;
        let width = (420. * scale).round() as i32;
        let height = (220. * scale).round() as i32;
        let x = info.rcMonitor.left + (info.rcMonitor.right - info.rcMonitor.left - width) / 2;
        let y = info.rcMonitor.top + (shape.offset * scale).round() as i32;
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
        let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.;
        let left = ((420. - shape.width) * 0.5 * scale).floor() as i32;
        let right = ((420. + shape.width) * 0.5 * scale).ceil() as i32;
        let height = (shape.height * scale).ceil() as i32;
        let diameter = (shape.radius.min(shape.height / 2.) * 2. * scale).round() as i32;
        let r = CreateRoundRectRgn(left - 1, -1, right + 1, height + 1, diameter, diameter);
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
        match msg {
            WM_STYLECHANGING => {
                // Winit recomputes styles when visibility or DPI changes. Keep
                // overlay invariants in the proposed styles before Windows applies
                // them, rather than fighting them with an idle polling loop.
                // SAFETY: WM_STYLECHANGING supplies a writable STYLESTRUCT for
                // the synchronous duration of this message.
                let proposed = &mut *(lparam.0 as *mut STYLESTRUCT);
                if wparam.0 as i32 == GWL_EXSTYLE.0 {
                    proposed.styleNew =
                        (proposed.styleNew | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0)
                            & !WS_EX_APPWINDOW.0;
                    let topmost = NATIVE.with(|n| n.borrow().shape.topmost);
                    if topmost {
                        proposed.styleNew |= WS_EX_TOPMOST.0;
                    } else {
                        proposed.styleNew &= !WS_EX_TOPMOST.0;
                    }
                } else if wparam.0 as i32 == GWL_STYLE.0 {
                    proposed.styleNew &= !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_BORDER.0);
                }
            }
            WM_WINDOWPOSCHANGING => {
                // SAFETY: this message carries a valid writable WINDOWPOS. It
                // belongs to Windows and is not retained after the callback.
                let proposed = &mut *(lparam.0 as *mut WINDOWPOS);
                proposed.flags |= SWP_NOACTIVATE;
            }
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
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
    // SAFETY: all menus are local to this call and destroyed after the modal
    // popup returns. DestroyMenu recursively destroys attached submenus.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let Ok(settings) = CreatePopupMenu() else {
            let _ = DestroyMenu(menu);
            return;
        };
        let Ok(demo) = CreatePopupMenu() else {
            let _ = DestroyMenu(settings);
            let _ = DestroyMenu(menu);
            return;
        };
        let (hwnd, shape, animations) = NATIVE.with(|n| {
            let n = n.borrow();
            (n.hwnd, n.shape, n.animations)
        });
        let add = |m, id, label: &str, flags| {
            let text: Vec<u16> = label.encode_utf16().chain([0]).collect();
            let _ = AppendMenuW(m, flags, id, PCWSTR(text.as_ptr()));
        };
        add(menu, 1, "Show Island", MF_STRING);
        add(menu, 5, "Settings…", MF_STRING);
        add(menu, 2, "Reset Island", MF_STRING);
        add(
            settings,
            10,
            "Animations",
            MF_STRING | if animations { MF_CHECKED } else { MF_UNCHECKED },
        );
        add(
            settings,
            11,
            "Always on top",
            MF_STRING
                | if shape.topmost {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                },
        );
        add(settings, 12, "Top offset: flush", MF_STRING);
        add(settings, 13, "Top offset: 6 px", MF_STRING);
        add(settings, 14, "Top offset: 12 px", MF_STRING);
        let (appearance, opacity, bounce) = NATIVE.with(|n| {
            let n = n.borrow();
            (n.appearance, n.opacity, n.bounce)
        });
        // All submenus are owned by the root menu after attachment.
        if let Ok(material) = CreatePopupMenu() {
            for (id, label, value) in [
                (30, "Solid black", Appearance::Solid),
                (31, "Transparent", Appearance::Transparent),
                (32, "Acrylic blur", Appearance::Acrylic),
                (33, "Eco liquid glass", Appearance::Glass),
                (34, "Live liquid glass (higher cost)", Appearance::LiveGlass),
            ] {
                add(
                    material,
                    id,
                    label,
                    MF_STRING
                        | if appearance == value {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        },
                );
            }
            add(settings, material.0 as usize, "Material", MF_POPUP);
        }
        if let Ok(tint) = CreatePopupMenu() {
            for (id, label, value) in [
                (40, "Light tint (25%)", 0.25f32),
                (41, "Balanced tint (55%)", 0.55),
                (42, "Dark tint (80%)", 0.8),
            ] {
                add(
                    tint,
                    id,
                    label,
                    MF_STRING
                        | if (opacity - value).abs() < 0.01 {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        },
                );
            }
            add(settings, tint.0 as usize, "Transparency", MF_POPUP);
        }
        if let Ok(motion) = CreatePopupMenu() {
            for (id, label, value) in [
                (50, "Soft", Bounce::Soft),
                (51, "Balanced bounce", Bounce::Balanced),
                (52, "Lively bounce", Bounce::Lively),
            ] {
                add(
                    motion,
                    id,
                    label,
                    MF_STRING
                        | if bounce == value {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        },
                );
            }
            add(settings, motion.0 as usize, "Spring response", MF_POPUP);
        }
        add(menu, settings.0 as usize, "Settings", MF_POPUP);
        if cfg!(debug_assertions) || std::env::args().any(|a| a == "--demo") {
            for (id, label) in [
                (20, "Idle"),
                (21, "Compact"),
                (22, "Expanded"),
                (23, "Volume"),
                (24, "Media"),
            ] {
                add(demo, id, label, MF_STRING);
            }
            add(menu, demo.0 as usize, "Demo states", MF_POPUP);
        } else {
            let _ = DestroyMenu(demo);
        }
        add(menu, 0, "", MF_SEPARATOR);
        add(menu, 4, "Exit", MF_STRING);
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
            10 => emit(Event::ToggleAnimations),
            11 => emit(Event::ToggleTopmost),
            12 => emit(Event::Offset(0.)),
            13 => emit(Event::Offset(6.)),
            14 => emit(Event::Offset(12.)),
            20 => emit(Event::State(IslandState::Idle)),
            21 => emit(Event::State(IslandState::Compact)),
            22 => emit(Event::State(IslandState::Expanded)),
            23 => emit(Event::State(IslandState::Volume)),
            24 => emit(Event::State(IslandState::Media)),
            30 => emit(Event::Appearance(Appearance::Solid)),
            31 => emit(Event::Appearance(Appearance::Transparent)),
            32 => emit(Event::Appearance(Appearance::Acrylic)),
            33 => emit(Event::Appearance(Appearance::Glass)),
            34 => emit(Event::Appearance(Appearance::LiveGlass)),
            40 => emit(Event::Opacity(0.25)),
            41 => emit(Event::Opacity(0.55)),
            42 => emit(Event::Opacity(0.8)),
            50 => emit(Event::Bounce(Bounce::Soft)),
            51 => emit(Event::Bounce(Bounce::Balanced)),
            52 => emit(Event::Bounce(Bounce::Lively)),
            _ => {}
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let (hwnd, icon) = NATIVE.with(|n| {
            let mut n = n.borrow_mut();
            n.handler = None;
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

/// Apply documented Windows 11 compositor backdrops. Success indicates DWM accepted
/// the material; Windows may choose a solid accessibility/battery-saver fallback.
pub fn set_appearance(appearance: Appearance, opacity: f32, bounce: Bounce) -> Result<(), String> {
    use ::windows::Win32::Graphics::Dwm::*;
    let hwnd = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        n.appearance = appearance;
        n.opacity = opacity;
        n.bounce = bounce;
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
        let kind = if matches!(appearance, Appearance::Acrylic) {
            DWMSBT_TRANSIENTWINDOW
        } else {
            DWMSBT_NONE
        };
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &kind as *const _ as _,
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
        .map_err(|e| format!("Windows backdrop unavailable: {e}"))?;
        let margins = ::windows::Win32::UI::Controls::MARGINS {
            cxLeftWidth: if appearance == Appearance::Acrylic {
                -1
            } else {
                0
            },
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 0,
        };
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
    _child: i32,
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
    if event == EVENT_OBJECT_LOCATIONCHANGE && object != OBJID_WINDOW.0 {
        return;
    }
    queue_policy();
    let hwnd = NATIVE.with(|n| {
        let mut n = n.borrow_mut();
        if n.appearance != Appearance::Glass || n.refresh_pending {
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
pub fn capture_snapshot() -> Result<super::capture::BackdropFrame, String> {
    let (left, top, width, height) = capture_bounds()?;
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
        for pixel in pixels.chunks_exact_mut(4) {
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
