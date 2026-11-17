//! Opt-in WGC probe. Repaints a small owned window beneath a settled island.
use super::*;
use slint::winit_030::WinitWindowAccessor;
use std::sync::atomic::AtomicU32;
use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect,
            PAINTSTRUCT, UpdateWindow,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_HINSTANCE, GetWindowLongPtrW,
            LWA_ALPHA, RegisterClassW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
            SetLayeredWindowAttributes, SetWindowPos, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
    core::{Error, w},
};

const WARM: [u8; 3] = [221, 43, 72];
const COOL: [u8; 3] = [31, 80, 224];
static COLOR: AtomicU32 = AtomicU32::new(0);

struct BackdropWindow(HWND);

impl BackdropWindow {
    fn new(overlay: HWND, (left, top, width, height): (i32, i32, u32, u32)) -> Result<Self, Error> {
        // SAFETY: the Slint window is owned by this process and this UI thread.
        // The class uses a static name and callback. Positioning never activates either window.
        unsafe {
            let instance = HINSTANCE(GetWindowLongPtrW(overlay, GWLP_HINSTANCE) as *mut _);
            let class = w!("StillIslandLiveMaterialProbe");
            let definition = WNDCLASSW {
                lpfnWndProc: Some(paint_backdrop),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassW(&definition) == 0 {
                return Err(Error::from_thread());
            }
            let test_width = width.min(360) as i32;
            let test_height = height.min(140) as i32;
            let handle = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class,
                w!("Still Island live material verification"),
                WS_POPUP,
                left + (width as i32 - test_width) / 2,
                top,
                test_width,
                test_height,
                None,
                None,
                Some(instance),
                None,
            )?;
            let backdrop = Self(handle);
            // The layered, transparent window passes mouse input through to the desktop.
            SetLayeredWindowAttributes(handle, COLORREF(0), 255, LWA_ALPHA)?;
            SetWindowPos(
                handle,
                Some(overlay),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
            Ok(backdrop)
        }
    }

    fn color(&self, rgb: [u8; 3]) {
        COLOR.store(
            u32::from_le_bytes([rgb[0], rgb[1], rgb[2], 0]),
            Ordering::Relaxed,
        );
        // SAFETY: invalidate and update only the owned backdrop; its position never changes.
        unsafe {
            let _ = InvalidateRect(Some(self.0), None, false);
            let _ = UpdateWindow(self.0);
        }
    }
}

impl Drop for BackdropWindow {
    fn drop(&mut self) {
        // SAFETY: this is an owned HWND and is released on the creating UI thread.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

unsafe extern "system" fn paint_backdrop(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Windows supplies a valid HWND on this callback. GDI resources live
    // only for this paint, and each BeginPaint is paired with EndPaint.
    unsafe {
        if message != WM_PAINT {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let mut paint = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut paint);
        let brush = CreateSolidBrush(COLORREF(COLOR.load(Ordering::Relaxed)));
        if !target.is_invalid() && !brush.is_invalid() {
            let _ = FillRect(target, &paint.rcPaint, brush);
        }
        if !brush.is_invalid() {
            let _ = DeleteObject(brush.into());
        }
        let _ = EndPaint(hwnd, &paint);
        LRESULT(0)
    }
}

struct Run {
    backdrop: BackdropWindow,
    bounds: (i32, i32, u32, u32),
    appearance: Appearance,
    warm: Option<(u64, [u8; 3])>,
    repainted: Instant,
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(750), move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let Some(ui) = app.borrow().ui.upgrade() else {
            return;
        };
        let appearance = app.borrow().settings.appearance;
        if !verification_isolated_material()
            || !matches!(
                appearance,
                Appearance::Acrylic
                    | Appearance::Glass
                    | Appearance::LiveGlass
                    | Appearance::LiquidGlass
            )
        {
            finish(Err("requires an isolated live material fixture".into()));
            return;
        }
        let run = (|| -> Result<Run, String> {
            let overlay = ui
                .window()
                .with_winit_window(|window| match window.window_handle().ok()?.as_raw() {
                    RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
                    _ => None,
                })
                .flatten()
                .ok_or("missing owned overlay HWND")?;
            let bounds = platform::capture_bounds()?;
            let backdrop = BackdropWindow::new(overlay, bounds).map_err(|e| e.to_string())?;
            let repainted = Instant::now();
            backdrop.color(WARM);
            Ok(Run {
                backdrop,
                bounds,
                appearance,
                warm: None,
                repainted,
            })
        })();
        match run {
            Ok(run) => await_capture(&app, run),
            Err(error) => finish(Err(error)),
        }
    });
}

fn await_capture(app: &Rc<RefCell<App>>, mut run: Run) {
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(25), move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let Some(ui) = app.borrow().ui.upgrade() else {
            return;
        };
        let expected = if run.warm.is_some() { COOL } else { WARM };
        let captured = {
            let app = app.borrow();
            let glass = app.glass.borrow();
            let (frame, revision) = if run.appearance == Appearance::LiquidGlass {
                (glass.liquid_backdrop.as_deref(), glass.liquid_revision)
            } else {
                (glass.backdrop.as_ref(), glass.backdrop_revision())
            };
            frame.and_then(|frame| {
                let y = 24_u64 * u64::from(frame.height) / u64::from(run.bounds.3.max(1));
                let index = (y as usize * frame.width as usize + frame.width as usize / 2) * 4;
                let rgb = frame.pixels.get(index..index + 3)?;
                let matches_fixture = rgb
                    .iter()
                    .zip(expected)
                    .all(|(actual, wanted)| actual.abs_diff(wanted) <= 24);
                (app.capture.is_some() && matches_fixture).then_some(revision)
            })
        };
        let Some(revision) = captured else {
            if run.repainted.elapsed() >= Duration::from_secs(6) {
                finish(Err(format!(
                    "{} did not deliver the owned backdrop color {expected:?}",
                    run.appearance.key()
                )));
            } else {
                await_capture(&app, run);
            }
            return;
        };
        let published_ms = run.repainted.elapsed().as_secs_f64() * 1000.;
        let steady = {
            let app = app.borrow();
            app.state == IslandState::Idle && !app.hidden && !app.timer.running()
        };
        if !steady || platform::capture_bounds().ok() != Some(run.bounds) {
            finish(Err(
                "overlay moved or changed state while waiting for capture".into(),
            ));
            return;
        }
        let Some(rendered) = rendered_sample(&ui) else {
            finish(Err("could not read the rendered overlay pixel".into()));
            return;
        };
        if run.appearance == Appearance::LiquidGlass {
            // Capture and the offscreen renderer publish independently.
            let difference = i16::from(rendered[0]) - i16::from(rendered[2]);
            let rendered_expected = if run.warm.is_some() {
                difference <= -8
            } else {
                difference >= 8
            };
            if !rendered_expected {
                if run.repainted.elapsed() >= Duration::from_secs(6) {
                    finish(Err(format!(
                        "Liquid Glass rendered {rendered:?} after capture delivered {expected:?}"
                    )));
                } else {
                    await_capture(&app, run);
                }
                return;
            }
        }
        let snapshot_name = if run.warm.is_some() {
            "live-material-cool"
        } else {
            "live-material-warm"
        };
        if let Err(error) = crate::verification::snapshot(&ui, snapshot_name) {
            finish(Err(format!("renderer snapshot {snapshot_name}: {error}")));
            return;
        }
        let Some((warm_revision, warm)) = run.warm else {
            run.warm = Some((revision, rendered));
            run.repainted = Instant::now();
            run.backdrop.color(COOL);
            await_capture(&app, run);
            return;
        };
        let color_changed = i16::from(warm[0]) - i16::from(warm[2]) >= 8
            && i16::from(rendered[2]) - i16::from(rendered[0]) >= 8;
        if revision == warm_revision || !color_changed {
            finish(Err(format!(
                "{} rendered {warm:?} then {rendered:?}, revisions {warm_revision}/{revision}",
                run.appearance.key()
            )));
            return;
        }
        finish(Ok(format!(
            "{} updated a settled overlay from {warm:?} to {rendered:?}, gpu_material={}; capture published within {published_ms:.1} ms (25 ms probe sampling)",
            run.appearance.key(),
            ui.get_gpu_material(),
        )));
    });
}

fn rendered_sample(ui: &IslandWindow) -> Option<[u8; 3]> {
    let frame = ui.window().take_snapshot().ok()?;
    let scale = frame.width() as f32 / ui.get_canvas_width();
    let x = ((ui.get_canvas_width() + ui.get_shape_width()) * 0.5 - 14.) * scale;
    let y = ui.get_shape_height() * 0.55 * scale;
    let pixel = frame
        .as_slice()
        .get(y as usize * frame.width() as usize + x as usize)?;
    Some([pixel.r, pixel.g, pixel.b])
}

fn finish(result: Result<String, String>) {
    match result {
        Ok(message) => record(&format!("live material PASS {message}")),
        Err(message) => record(&format!("live material FAILED {message}")),
    }
    record("live material verification complete");
    let _ = slint::quit_event_loop();
}
