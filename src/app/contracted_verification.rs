//! Opt-in native contour regression; never sends volume or media commands.
use super::*;

fn check(condition: bool, label: &str) {
    record(&format!(
        "contracted {} {label}",
        if condition { "PASS" } else { "FAILED" }
    ));
}

fn radius(ui: &IslandWindow, expected: f32, label: &str) {
    check((ui.get_shape_radius() - expected).abs() < 0.25, label);
}

fn capture(ui: &IslandWindow, label: &str) {
    match crate::verification::snapshot(ui, label) {
        Ok(path) => record(&format!("snapshot {}", path.display())),
        Err(error) => record(&format!("snapshot FAILED {error}")),
    }
    record(&format!("contracted {}", platform::diagnostic_snapshot()));
}

fn settings(app: &Rc<RefCell<App>>, contracted: f64, expanded: f64) {
    let mut settings = app.borrow().settings.clone();
    settings.notch_contracted_radius = contracted;
    settings.notch_corner_radius = expanded;
    apply_settings(app, settings, false);
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    let saved_preferences = Rc::new(std::fs::read(Settings::path()).ok());
    let settled_frames = Rc::new(std::cell::Cell::new(0));
    {
        let mut settings = app.borrow().settings.clone();
        settings.form_factor = crate::shape::FormFactor::Notch;
        settings.island_scale = 1.;
        settings.appearance = Appearance::Solid;
        settings.notch_contracted_radius = 8.;
        settings.notch_corner_radius = 8.;
        settings.animations = true;
        settings.follow_reduced_motion = false;
        settings.compact_now_playing = false;
        settings.auto_volume = false;
        settings.battery_previews = false;
        settings.notifications_enabled = false;
        settings.hover_expand = false;
        settings.outside_click = false;
        settings.shortcut_enabled = false;
        settings.fullscreen_hide = false;
        settings.topmost = true;
        apply_settings(app, settings, false);
    }
    // The compact volume preview expires at 4200ms; allow its content fade and
    // contour springs to settle before checking the capped idle radius.
    for (step, at) in [700, 1700, 2700, 3500, 5500, 6800, 8000, 9400, 10300]
        .into_iter()
        .enumerate()
    {
        let weak = Rc::downgrade(app);
        let saved_preferences = saved_preferences.clone();
        let settled_frames = settled_frames.clone();
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            match step {
                0 => {
                    check(app.borrow().state == IslandState::Idle, "startup is idle");
                    check(
                        Settings::default().notch_contracted_radius == 8.,
                        "contracted default is eight pixels",
                    );
                    radius(&ui, 8., "startup resolves eight pixel contracted corners");
                    settings(&app, 3., 8.);
                }
                1 => {
                    radius(
                        &ui,
                        3.,
                        "live preference retargets idle corners to three pixels",
                    );
                    capture(&ui, "notch-contracted-3");
                    transition(&app, IslandState::Compact);
                }
                2 => {
                    check(
                        app.borrow().state == IslandState::Compact,
                        "compact demonstration is active",
                    );
                    radius(&ui, 3., "compact uses the contracted corner preference");
                    capture(&ui, "notch-compact-3");
                    // Renew the short preview before changing its contour so it
                    // stays compact until the next deterministic assertion.
                    transition(&app, IslandState::Compact);
                    settings(&app, 24., 8.);
                }
                3 => {
                    radius(
                        &ui,
                        24.,
                        "compact accepts half-height twenty-four pixel corners",
                    );
                    check(
                        (ui.get_shape_height() - 48.).abs() < 0.5,
                        "compact retains forty-eight pixel height",
                    );
                    capture(&ui, "notch-compact-24");
                    transition(&app, IslandState::Idle);
                }
                4 => {
                    radius(&ui, 16., "idle caps contracted corners at half-height");
                    check(
                        (ui.get_shape_height() - 32.).abs() < 0.5,
                        "idle retains thirty-two pixel height",
                    );
                    capture(&ui, "notch-contracted-capped");
                    transition(&app, IslandState::Expanded);
                }
                5 => {
                    radius(
                        &ui,
                        8.,
                        "expanded corners remain independent of contracted preference",
                    );
                    settings(&app, 3., 14.);
                }
                6 => {
                    radius(&ui, 14., "expanded preference retargets to fourteen pixels");
                    capture(&ui, "notch-expanded-independent");
                    transition(&app, IslandState::Idle);
                }
                7 => {
                    radius(
                        &ui,
                        3.,
                        "collapse restores separately configured three pixel corners",
                    );
                    check(
                        !app.borrow().timer.running(),
                        "morph scheduling stops after settling",
                    );
                    capture(&ui, "notch-contracted-final");
                    settled_frames.set(app.borrow().frames);
                }
                _ => {
                    let a = app.borrow();
                    check(
                        !a.timer.running() && !a.media_clock.running() && !a.deadline.running(),
                        "animation and presentation timers remain stopped",
                    );
                    check(
                        a.frames == settled_frames.get(),
                        "settled idle schedules no further animation updates",
                    );
                    check(
                        std::fs::read(Settings::path()).ok() == *saved_preferences,
                        "user preferences are unchanged on disk",
                    );
                    record("contracted verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}
