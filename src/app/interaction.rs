//! Hover intent, pointer gestures, and countdown GUI scheduling.
use super::*;

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    {
        let app = app.clone();
        ui.on_hover(move |hover| {
            // Child controls can take has-hover away from their parent TouchArea.
            // Native contour hit testing is authoritative in ordinary operation.
            if app.borrow().verification {
                set_hover(&app, hover);
            }
        });
    }
    {
        let app = app.clone();
        ui.on_pointer_moved(move |x, y| {
            if !app.borrow().verification {
                return;
            }
            let mut a = app.borrow_mut();
            let now = a.now();
            let before = a.hover_intent.deadline(a.settings.hover_delay);
            a.hover_intent.moved(x as f64, y as f64, now);
            let changed = before != a.hover_intent.deadline(a.settings.hover_delay);
            drop(a);
            if changed {
                schedule_hover(&app);
            }
        });
    }
    {
        let app = app.clone();
        ui.on_toggle_pin(move || {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.toggle_pin(now);
            drop(a);
            schedule_collapse(&app);
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_open_timer(move || {
            sync_countdown(&app);
            pages::open(&app, crate::navigation::Page::Timer);
        });
    }
    {
        let app = app.clone();
        ui.on_timer_set_duration(move |seconds| {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.countdown
                .set_duration(seconds.clamp(1, 86400) as u64, now);
            a.coordinator.clear_timer(now);
            a.countdown_engaged = false;
            drop(a);
            sync_countdown(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_timer_toggle(move || {
            let mut a = app.borrow_mut();
            let now = a.now();
            if a.countdown.snapshot(now).running {
                a.countdown.pause(now);
            } else {
                a.countdown.start(now);
            }
            a.countdown_engaged = true;
            drop(a);
            sync_countdown(&app);
            // Starting from controls returns the countdown to its compact form.
            transition(&app, IslandState::Idle);
        });
    }
    {
        let app = app.clone();
        ui.on_timer_reset(move || {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.countdown.reset(now);
            a.coordinator.clear_timer(now);
            a.countdown_engaged = false;
            drop(a);
            sync_countdown(&app);
        });
    }
    sync_countdown(app);
    {
        let app = app.clone();
        ui.on_pointer_interaction(move |pressed| {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.pointer_pressed = pressed;
            let pause = pressed || a.hover;
            a.coordinator.set_hover(pause, now);
            drop(a);
            schedule_collapse(&app);
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_escape(move || transition(&app, IslandState::Idle));
    }
}

fn set_hover(app: &Rc<RefCell<App>>, hover: bool) {
    if app.borrow().launcher.open {
        return;
    }
    let idle = {
        let mut a = app.borrow_mut();
        if a.hover == hover {
            return;
        }
        a.hover = hover;
        let now = a.now();
        let pause = hover || a.pointer_pressed;
        a.coordinator.set_hover(pause, now);
        if hover {
            a.hover_intent.enter(now);
        } else {
            a.hover_intent.leave();
            a.wheel.reset();
            a.page_wheel.reset();
        }
        if verification::has_arg("--trace-hover") {
            record(&format!(
                "hover edge now={now} inside={hover} state={:?} enabled={} pressed={} intent={:?}",
                a.state, a.settings.hover_expand, a.pointer_pressed, a.hover_intent
            ));
        }
        a.state == IslandState::Idle
    };
    if idle {
        let mut a = app.borrow_mut();
        let state = a.state;
        a.target(state);
        drop(a);
        wake(app);
    }
    schedule_hover(app);
    schedule_collapse(app);
    sync_presentation(app);
}

/// Read current screen coordinates rather than a TouchArea's moving local
/// origin. Called only on pointer/window events and existing animation frames.
#[derive(Clone, Copy, Debug)]
struct NativePointer {
    client: (f64, f64),
    screen: (f64, f64),
    on_window: bool,
}

fn base_coordinates(point: (i32, i32), dpi: u32, zoom: f64) -> (f64, f64) {
    let scale = f64::from(dpi.max(96)) / 96. * zoom;
    (f64::from(point.0) / scale, f64::from(point.1) / scale)
}

fn native_pointer(app: &Rc<RefCell<App>>) -> Option<NativePointer> {
    use slint::winit_030::WinitWindowAccessor;
    use windows::Win32::{
        Foundation::*,
        Graphics::Gdi::ScreenToClient,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    };
    let (ui, zoom, hidden) = {
        let a = app.borrow();
        (a.ui.upgrade()?, a.settings.island_scale, a.hidden)
    };
    if hidden {
        return None;
    }
    let mut result = None;
    ui.window().with_winit_window(|window| {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        // SAFETY: this synchronous GUI-thread read uses a live UI-owned HWND and
        // initialized POINT outputs. No hooks, capture, timers, or pointers persist.
        unsafe {
            let mut screen = POINT::default();
            if GetCursorPos(&mut screen).is_err() {
                return;
            }
            let target = WindowFromPoint(screen);
            let on_window = target == hwnd || GetAncestor(target, GA_ROOT) == hwnd;
            let mut client = screen;
            if !ScreenToClient(hwnd, &mut client).as_bool() {
                return;
            }
            let dpi = GetDpiForWindow(hwnd);
            result = Some(NativePointer {
                client: base_coordinates((client.x, client.y), dpi, zoom),
                screen: base_coordinates((screen.x, screen.y), dpi, zoom),
                on_window,
            });
        }
    });
    result
}

/// Reconcile after contour changes too: the silhouette can leave a stationary
/// pointer without producing a fresh CursorMoved event. No work is scheduled idle.
pub(super) fn refresh_pointer(app: &Rc<RefCell<App>>) {
    if app.borrow().verification {
        return;
    }
    let observed = native_pointer(app);
    let inside = {
        let mut a = app.borrow_mut();
        a.pointer = observed.map(|pointer| pointer.client);
        observed.is_some_and(|pointer| {
            pointer.on_window
                && crate::gestures::contains_in_canvas(
                    pointer.client.0,
                    pointer.client.1,
                    a.springs[0].position,
                    a.springs[1].position,
                    a.springs[2].position,
                    a.settings.form_factor,
                    a.canvas_size.0,
                )
        })
    };
    if inside != app.borrow().hover && verification::has_arg("--trace-hover") {
        record(&format!(
            "pointer edge observed={observed:?} inside={inside}"
        ));
    }
    set_hover(app, inside);
}

fn release_pointer(app: &Rc<RefCell<App>>) {
    {
        let mut a = app.borrow_mut();
        a.pointer_pressed = false;
        let now = a.now();
        let hover = a.hover;
        a.coordinator.set_hover(hover, now);
    }
    schedule_collapse(app);
    sync_presentation(app);
}

pub(super) fn schedule_hover(app: &Rc<RefCell<App>>) {
    app.borrow().hover_timer.stop();
    let delay = {
        let a = app.borrow();
        if !a.hover
            || !a.settings.hover_expand
            || a.hidden
            || a.settings_visible
            || a.coordinator.panel_open()
        {
            return;
        }
        a.hover_intent
            .deadline(a.settings.hover_delay)
            .map(|end| end.saturating_sub(a.now()).max(1))
    };
    if let Some(delay) = delay {
        if verification::has_arg("--trace-hover") {
            let a = app.borrow();
            record(&format!(
                "hover dwell arm now={} delay={delay} deadline={:?}",
                a.now(),
                a.hover_intent.deadline(a.settings.hover_delay)
            ));
        }
        let weak = Rc::downgrade(app);
        app.borrow().hover_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(delay),
            move || {
                if let Some(app) = weak.upgrade() {
                    // Revalidate at the deadline: another window may have moved
                    // over a stationary pointer since its last native mouse event.
                    refresh_pointer(&app);
                    let mut a = app.borrow_mut();
                    let now = a.now();
                    let eligible = a.hover
                        && a.settings.hover_expand
                        && !a.hidden
                        && !a.settings_visible
                        && !a.coordinator.panel_open();
                    let ready = a.hover_intent.ready(now, a.settings.hover_delay);
                    if verification::has_arg("--trace-hover") {
                        record(&format!("hover dwell fire now={now} eligible={eligible} ready={ready} inside={} hidden={} settings={} panel={} state={:?} intent={:?}",
                            a.hover, a.hidden, a.settings_visible, a.coordinator.panel_open(), a.state, a.hover_intent));
                    }
                    if eligible && ready {
                        a.coordinator.open_hover(now);
                    }
                    drop(a);
                    sync_presentation(&app);
                    // A platform timer can wake before the millisecond clock
                    // reaches the deadline. Re-arm the remainder rather than
                    // losing the hover visit.
                    if eligible && !ready {
                        schedule_hover(&app);
                    }
                }
            },
        );
    }
}

pub(super) fn schedule_collapse(app: &Rc<RefCell<App>>) {
    app.borrow().collapse_timer.stop();
    let delay = {
        let a = app.borrow();
        if a.hover || a.pointer_pressed || a.coordinator.pinned() || !a.coordinator.panel_open() {
            return;
        }
        a.settings.collapse_delay
    };
    if verification::has_arg("--trace-hover") {
        record(&format!(
            "hover collapse arm now={} delay={delay}",
            app.borrow().now()
        ));
    }
    let weak = Rc::downgrade(app);
    app.borrow().collapse_timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(delay as u64),
        move || {
            if let Some(app) = weak.upgrade() {
                // Reentry can precede delivery of a queued native mouse event.
                refresh_pointer(&app);
                let mut a = app.borrow_mut();
                let now = a.now();
                if verification::has_arg("--trace-hover") {
                    record(&format!(
                        "hover collapse fire now={now} inside={} pressed={} pinned={} state={:?}",
                        a.hover,
                        a.pointer_pressed,
                        a.coordinator.pinned(),
                        a.state
                    ));
                }
                if !a.hover && !a.pointer_pressed {
                    a.coordinator.close_hover(now);
                }
                drop(a);
                sync_presentation(&app);
            }
        },
    );
}

pub(super) fn sync_countdown(app: &Rc<RefCell<App>>) {
    app.borrow().countdown_timer.stop();
    let deadline = {
        let mut a = app.borrow_mut();
        let now = a.now();
        let snapshot = a.countdown.snapshot(now);
        let finished = a.countdown.take_finished(now);
        let active = a.countdown_engaged && snapshot.remaining_seconds > 0;
        a.coordinator.set_timer(active, finished, now);
        if let Some(ui) = a.ui.upgrade() {
            ui.set_timer_draft_minutes(snapshot.duration_seconds.div_ceil(60) as i32);
            ui.set_timer_label(time_label(snapshot.remaining_seconds as f64).into());
            ui.set_timer_running(snapshot.running);
            ui.set_timer_paused(active && !snapshot.running && !snapshot.finished);
            ui.set_timer_finished(snapshot.finished);
            ui.set_timer_progress(
                (1. - snapshot.remaining_seconds as f64 / snapshot.duration_seconds as f64) as f32,
            );
        }
        a.countdown
            .next_deadline(now)
            .map(|end| end.saturating_sub(now).max(1))
    };
    sync_presentation(app);
    if let Some(delay) = deadline {
        let weak = Rc::downgrade(app);
        app.borrow().countdown_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(delay),
            move || {
                if let Some(app) = weak.upgrade() {
                    sync_countdown(&app);
                }
            },
        );
    }
}

pub(super) fn handle_overlay_input(
    app: &Rc<RefCell<App>>,
    event: &slint::winit_030::winit::event::WindowEvent,
) -> slint::winit_030::EventResult {
    use slint::winit_030::{
        EventResult,
        winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    };
    if app.borrow().launcher.open {
        if matches!(event, WindowEvent::Focused(false)) {
            launcher::dismiss(app, false);
        }
        return EventResult::Propagate;
    }
    // Slint invokes this filter before it draws or handles the native geometry
    // event. The attach-time query can therefore run before the HWND is visible,
    // after an initial CursorEntered was already delivered. Reconcile on the next
    // event turn, after presentation, even when the mouse has remained stationary.
    let reconcile_after_event = matches!(
        event,
        WindowEvent::Resized(_)
            | WindowEvent::Moved(_)
            | WindowEvent::Occluded(_)
            | WindowEvent::ScaleFactorChanged { .. }
    ) || (matches!(event, WindowEvent::RedrawRequested)
        && !app.borrow().hover);
    if reconcile_after_event && !app.borrow().verification {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(1), move || {
            if let Some(app) = weak.upgrade() {
                refresh_pointer(&app);
            }
        });
    }
    if matches!(
        event,
        WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
    ) {
        refresh_pointer(app);
        if matches!(event, WindowEvent::CursorMoved { .. })
            && let Some(pointer) = native_pointer(app)
        {
            let mut a = app.borrow_mut();
            let now = a.now();
            let before = a.hover_intent.deadline(a.settings.hover_delay);
            a.hover_intent
                .moved(pointer.screen.0, pointer.screen.1, now);
            let after = a.hover_intent.deadline(a.settings.hover_delay);
            if before != after && verification::has_arg("--trace-hover") {
                record(&format!(
                    "hover dwell movement now={now} screen={:?} before={before:?} after={after:?}",
                    pointer.screen
                ));
            }
            drop(a);
            // SetWindowRgn can synthesize mouse moves every animation frame.
            // Re-arming an unchanged deadline on those events starves the native
            // timer while the hover-feedback spring is running.
            if before != after {
                schedule_hover(app);
            }
        }
    }
    if matches!(
        event,
        WindowEvent::MouseInput {
            button: MouseButton::Left,
            state: ElementState::Released,
            ..
        } | WindowEvent::Focused(false)
    ) {
        refresh_pointer(app);
        release_pointer(app);
    } else if matches!(
        event,
        WindowEvent::MouseInput {
            button: MouseButton::Left,
            state: ElementState::Pressed,
            ..
        }
    ) {
        refresh_pointer(app);
        if app.borrow().hover {
            app.borrow_mut().pointer_pressed = true;
        }
    }
    let inside = {
        let a = app.borrow();
        !a.hidden
            && a.pointer.is_some_and(|(x, y)| {
                crate::gestures::contains_in_canvas(
                    x,
                    y,
                    a.springs[0].position,
                    a.springs[1].position,
                    a.springs[2].position,
                    a.settings.form_factor,
                    a.canvas_size.0,
                )
            })
    };
    if !inside {
        return EventResult::Propagate;
    }
    if matches!(event, WindowEvent::MouseWheel { .. }) {
        let mut a = app.borrow_mut();
        if a.pointer.is_some_and(|(x, y)| {
            a.ui.upgrade()
                .is_some_and(|ui| ui.invoke_lyrics_scroll_hit(x as f32, y as f32))
        }) {
            a.wheel.reset();
            a.page_wheel.reset();
            return EventResult::Propagate;
        }
    }
    if let WindowEvent::MouseWheel { delta, .. } = event {
        let (expanded, protected, picker, pressed) = {
            let a = app.borrow();
            let ui = a.ui.upgrade();
            let protected = a.pointer.is_some_and(|(x, y)| {
                ui.as_ref().is_some_and(|ui| {
                    ui.invoke_volume_scroll_hit(x as f32, y as f32)
                        || ui.invoke_plugin_scroll_hit(x as f32, y as f32)
                })
            });
            (
                a.coordinator.panel_open(),
                protected,
                ui.is_some_and(|ui| ui.get_player_picker_open()),
                a.pointer_pressed,
            )
        };
        if expanded && (picker || pressed) {
            app.borrow_mut().page_wheel.reset();
            return if picker {
                EventResult::Propagate
            } else {
                EventResult::PreventDefault
            };
        }
        if expanded && !protected && !picker && !pressed {
            let notches = match delta {
                MouseScrollDelta::LineDelta(_, y) => f64::from(*y),
                MouseScrollDelta::PixelDelta(p) => p.y / 40.,
            };
            app.borrow_mut().wheel.reset();
            pages::wheel(app, notches);
            return EventResult::PreventDefault;
        }
        let plugin_scroll = {
            let a = app.borrow();
            a.pointer.is_some_and(|(x, y)| {
                a.ui.upgrade()
                    .is_some_and(|ui| ui.invoke_plugin_scroll_hit(x as f32, y as f32))
            })
        };
        if plugin_scroll {
            app.borrow_mut().wheel.reset();
            app.borrow_mut().page_wheel.reset();
            return EventResult::Propagate;
        }
    }
    match event {
        WindowEvent::MouseWheel { delta, .. }
            if app.borrow().settings.scroll_volume
                && !app
                    .borrow()
                    .ui
                    .upgrade()
                    .is_some_and(|ui| ui.get_player_picker_open()) =>
        {
            let notches = match delta {
                MouseScrollDelta::LineDelta(_, y) => *y as f64,
                MouseScrollDelta::PixelDelta(p) => p.y / 40.,
            };
            let (service, change) = {
                let mut a = app.borrow_mut();
                let step = a.settings.volume_step;
                let change = a.wheel.advance(notches, step);
                (a.volume.clone(), change)
            };
            if let Some(service) = service
                && change != 0
            {
                match service
                    .snapshot()
                    .and_then(|s| service.set_volume(s.percent.saturating_add(change)))
                {
                    Ok(()) => {
                        volume::feedback(app);
                    }
                    Err(e) => status(app, &e),
                }
            }
            EventResult::PreventDefault
        }
        WindowEvent::MouseInput {
            button: MouseButton::Middle,
            state,
            ..
        } if app.borrow().settings.middle_mute => {
            if *state == ElementState::Pressed {
                let service = app.borrow().volume.clone();
                if let Some(service) = service {
                    if let Err(e) = service.toggle_mute() {
                        status(app, &e);
                    } else {
                        volume::feedback(app);
                    }
                }
            }
            EventResult::PreventDefault
        }
        _ => EventResult::Propagate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_coordinates_preserve_hit_testing_across_dpi_and_zoom() {
        for dpi in [96, 120, 144, 192] {
            for zoom in [1., 1.5, 2.] {
                let scale = f64::from(dpi) / 96. * zoom;
                let client =
                    base_coordinates(((400. * scale) as i32, (16. * scale) as i32), dpi, zoom);
                assert_eq!(client, (400., 16.));
                assert!(crate::gestures::contains_in_canvas(
                    client.0,
                    client.1,
                    126.,
                    32.,
                    8.,
                    crate::shape::FormFactor::Notch,
                    800.
                ));
                let outside =
                    base_coordinates(((600. * scale) as i32, (16. * scale) as i32), dpi, zoom);
                assert!(!crate::gestures::contains_in_canvas(
                    outside.0,
                    outside.1,
                    126.,
                    32.,
                    8.,
                    crate::shape::FormFactor::Notch,
                    800.
                ));
                assert_eq!(
                    base_coordinates(((-400. * scale) as i32, (16. * scale) as i32), dpi, zoom),
                    (-400., 16.)
                );
            }
        }
    }
}
