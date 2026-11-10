//! Opt-in regression orchestration kept outside the production UI lifecycle.
use super::*;

const VERIFICATION_FLAGS: [&str; 14] = [
    "--verify-liquid",
    "--verify-illustrations",
    "--verify-launcher",
    "--verify",
    "--verify-notifications",
    "--verify-settings",
    "--verify-interactions",
    "--verify-battery",
    "--verify-plugins",
    "--verify-lyrics",
    "--verify-contracted-corners",
    "--verify-size",
    "--verify-layout",
    "--verify-media-polish",
];
pub(super) fn has_arg(flag: &str) -> bool {
    std::env::args().any(|arg| arg == flag)
}
pub(super) fn enabled() -> bool {
    std::env::args().any(|arg| VERIFICATION_FLAGS.contains(&arg.as_str()))
}
pub(super) fn blocks_live_input() -> bool {
    enabled() || has_arg("--benchmark")
}
pub(super) fn setup(app: &Rc<RefCell<App>>) -> slint::Timer {
    if has_arg("--verify-liquid") {
        liquid_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-illustrations") {
        illustration_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-launcher") {
        launcher::setup_verification(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-media-polish") {
        media_polish_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-layout") {
        layout_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-size") {
        size_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-contracted-corners") {
        contracted_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-lyrics") {
        lyrics_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-plugins") {
        return slint::Timer::default();
    }
    if has_arg("--verify-battery") {
        battery_verification::setup(app);
        return slint::Timer::default();
    }
    if has_arg("--verify-settings") {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(700), move || {
            if let Some(app) = weak.upgrade() {
                show_preferences(&app);
            }
        });
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(1800), move || {
            if let Some(app) = weak.upgrade() {
                let ui = app.borrow().preferences.upgrade();
                if let Some(ui) = ui {
                    match crate::verification::snapshot_preferences(&ui) {
                        Ok(path) => record(&format!("settings snapshot {}", path.display())),
                        Err(e) => record(&format!("settings FAILED {e}")),
                    };
                    use slint::winit_030::WinitWindowAccessor;
                    ui.window().with_winit_window(|window| {
                        if let Ok(handle) = window.window_handle()
                            && let RawWindowHandle::Win32(handle) = handle.as_raw()
                        {
                            let hwnd =
                                windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _);
                            unsafe {
                                record(&format!(
                                    "settings focused={} noactivate={}",
                                    windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow()
                                        == hwnd,
                                    windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                                        hwnd,
                                        windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE
                                    ) as u32
                                        & windows::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE
                                            .0
                                        != 0
                                ));
                            }
                        }
                    });
                }
            }
        });
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(2600), move || {
            if let Some(app) = weak.upgrade() {
                close_preferences(&app, true);
            }
        });
    }
    let verification_timer = slint::Timer::default();
    if has_arg("--verify-interactions") {
        verify_interactions(app);
    } else if app.borrow().verification {
        record(&format!("startup {}", platform::diagnostic_snapshot()));
        let app = app.clone();
        let started = Instant::now();
        let mut stage = 0u32;
        verification_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let t = started.elapsed().as_millis();
                let next = if t < 3000 {
                    0
                } else if t < 6000 {
                    1
                } else if t < 9000 {
                    2
                } else if t < 12000 {
                    3
                } else if t < 14000 {
                    4 + ((t - 12000) / 100) as u32
                } else if t < 18000 {
                    24
                } else {
                    25
                };
                if next != stage {
                    stage = next;
                    if matches!(stage, 1..=4 | 25)
                        && let Some(ui) = app.borrow().ui.upgrade()
                    {
                        if !has_arg("--verify-notifications") {
                            let expected = match stage {
                                1 | 25 => IslandState::Idle,
                                2 => IslandState::Expanded,
                                3 => IslandState::Media,
                                _ => IslandState::Volume,
                            };
                            if app.borrow().state != expected {
                                record(&format!(
                                    "snapshot FAILED state {:?}, expected {expected:?}",
                                    app.borrow().state
                                ));
                            }
                        }
                        let name = if has_arg("--verify-notifications") {
                            match stage {
                                1 => "notification-before",
                                2 => "notification-compact",
                                3 => "notification-expanded",
                                4 => "notification-protected-media",
                                _ => "notification-settled",
                            }
                        } else {
                            match stage {
                                1 => "idle",
                                2 => "expanded",
                                3 => "media",
                                4 => "volume",
                                _ => "settled",
                            }
                        };
                        match crate::verification::snapshot(&ui, name) {
                            Ok(path) => record(&format!("snapshot {}", path.display())),
                            Err(e) => record(&format!("snapshot FAILED {e}")),
                        }
                    }
                    if has_arg("--verify-notifications") {
                        verification_notification_step(&app, stage);
                        return;
                    }
                    match stage {
                        1 => transition(&app, IslandState::Expanded),
                        2 => transition(&app, IslandState::Media),
                        3 => transition(&app, IslandState::Volume),
                        4..=23 => transition(
                            &app,
                            if stage.is_multiple_of(2) {
                                IslandState::Expanded
                            } else {
                                IslandState::Idle
                            },
                        ),
                        24 => transition(&app, IslandState::Idle),
                        _ => {
                            record(&format!(
                                "complete frames={} active={}",
                                app.borrow().frames,
                                app.borrow().timer.running()
                            ));
                            let _ = slint::quit_event_loop();
                        }
                    }
                }
            },
        );
    }
    verification_timer
}
pub(super) fn record(message: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("verification.log")
    {
        let _ = writeln!(file, "{message}");
    }
}

/// Exercises GUI callbacks without desktop input, audio changes, or permissions.
fn verify_interactions(app: &Rc<RefCell<App>>) {
    // Repeated short visits must never accumulate into a completed dwell.
    for (at, inside) in [
        (100, true),
        (130, false),
        (220, true),
        (250, false),
        (330, true),
        (360, false),
    ] {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            if let Some(app) = weak.upgrade() {
                let ui = app.borrow().ui.upgrade();
                if let Some(ui) = ui {
                    ui.invoke_hover(inside);
                }
            }
        });
    }
    for (step, at) in [
        700, 800, 850, 1000, 1200, 1300, 1500, 1800, 2100, 2350, 2600, 3200, 3300, 3800, 4200,
        5300, 8000, 11500, 11900, 12200, 12700, 14500,
    ]
    .into_iter()
    .enumerate()
    {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            let check = |ok: bool, name: &str| {
                record(&format!(
                    "interaction {} {name}",
                    if ok { "PASS" } else { "FAILED" }
                ))
            };
            let capture = |name: &str| match crate::verification::snapshot(&ui, name) {
                Ok(path) => record(&format!("snapshot {}", path.display())),
                Err(e) => record(&format!("snapshot FAILED {e}")),
            };
            match step {
                0 => {
                    check(
                        !app.borrow().coordinator.panel_open(),
                        "rapid entry exit never accumulates dwell",
                    );
                    ui.invoke_hover(true);
                }
                1 => ui.invoke_pointer_moved(0., 0.),
                2 => ui.invoke_pointer_moved(20., 0.),
                3 => check(
                    !app.borrow().coordinator.panel_open(),
                    "moving pointer delays expansion",
                ),
                4 => {
                    check(
                        app.borrow().coordinator.panel_open(),
                        "hover expands after dwell",
                    );
                    ui.invoke_hover(false);
                }
                5 => {
                    check(
                        app.borrow().coordinator.panel_open(),
                        "collapse waits for delay",
                    );
                    ui.invoke_hover(true);
                }
                6 => {
                    check(
                        app.borrow().coordinator.panel_open(),
                        "reentry cancels collapse",
                    );
                    ui.invoke_hover(false);
                }
                7 => {
                    check(!app.borrow().coordinator.panel_open(), "unhover collapses");
                    ui.invoke_hover(true);
                }
                8 => {
                    ui.invoke_toggle_pin();
                    ui.invoke_hover(false);
                }
                9 => {
                    check(
                        app.borrow().coordinator.pinned() && app.borrow().coordinator.panel_open(),
                        "pin survives unhover",
                    );
                    ui.invoke_toggle_pin();
                }
                10 => {
                    check(
                        !app.borrow().coordinator.panel_open(),
                        "unpin restores delayed collapse",
                    );
                    ui.invoke_timer_set_duration(3);
                    // Opening controls models a click inside the notch. Pages
                    // now retain hover lifetime and correctly close on leave.
                    ui.invoke_hover(true);
                    ui.invoke_open_timer();
                }
                11 => {
                    check(
                        app.borrow().state == IslandState::Timer,
                        "timer controls expand",
                    );
                    capture("timer-controls");
                }
                12 => {
                    ui.invoke_timer_toggle();
                    ui.invoke_hover(false);
                }
                13 => {
                    check(
                        ui.get_compact_timer() && ui.get_timer_running(),
                        "running timer stays compact",
                    );
                    capture("timer-compact");
                }
                14 => ui.invoke_timer_toggle(),
                15 => {
                    check(
                        !ui.get_timer_running() && ui.get_timer_label() == "0:03",
                        "paused timer preserves remainder",
                    );
                    ui.invoke_timer_toggle();
                }
                16 => {
                    check(
                        ui.get_timer_finished() && app.borrow().state == IslandState::Timer,
                        "completion morph",
                    );
                    capture("timer-finished");
                }
                17 => {
                    check(
                        ui.get_compact_timer() && app.borrow().state == IslandState::Compact,
                        "completion returns to selected compact timer page",
                    );
                    ui.invoke_hover(true);
                }
                18 => {
                    check(
                        app.borrow().coordinator.panel_open(),
                        "new hover visit opens after countdown",
                    );
                    ui.invoke_pointer_interaction(true);
                    ui.invoke_hover(false);
                }
                19 => {
                    check(
                        app.borrow().coordinator.panel_open(),
                        "pressed child protects pointer exit",
                    );
                    interaction::handle_overlay_input(
                        &app,
                        &slint::winit_030::winit::event::WindowEvent::Focused(false),
                    );
                }
                20 => {
                    check(
                        !app.borrow().pointer_pressed && !app.borrow().coordinator.panel_open(),
                        "lost focus releases pressed state and collapses hover",
                    );
                }
                _ => {
                    check(
                        ui.get_compact_timer() && app.borrow().state == IslandState::Compact,
                        "collapse retains selected compact timer page",
                    );
                    check(
                        !app.borrow().timer.running() && !app.borrow().countdown_timer.running(),
                        "timers stop after completion",
                    );
                    record("interaction verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}

pub(super) fn verification_notification_step(app: &Rc<RefCell<App>>, stage: u32) {
    let fixture = |id| NotificationRecord {
        id,
        app_id: "StillIsland.SilentFixture".into(),
        app_name: "Integration fixture".into(),
        title: format!("Notification {id}"),
        body: "A synthetic preview. Hiding this never removes a Windows notification.".into(),
        created_ticks: 0,
        icon: None,
    };
    match stage {
        1 => {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.push(fixture(1), now);
        }
        2 => {
            let ui = app.borrow().ui.upgrade();
            if let Some(ui) = ui {
                ui.invoke_expand_notification();
            }
        }
        3 => {
            transition(app, IslandState::Media);
            let mut a = app.borrow_mut();
            let now = a.now();
            for id in 2..32 {
                a.coordinator.push(fixture(id), now);
            }
            record("notification fixture queued while user Media panel remains pinned");
        }
        4 => {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.close(now);
            a.coordinator.volume(now);
        }
        24 => {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.clear_notifications(now);
            a.coordinator.close(now);
        }
        25 => {
            record(&format!(
                "complete frames={} active={}",
                app.borrow().frames,
                app.borrow().timer.running()
            ));
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
    sync_presentation(app);
}
