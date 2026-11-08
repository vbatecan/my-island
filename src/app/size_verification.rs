//! Whole-island size regression using original, static media content.
use super::*;

fn check(condition: bool, label: &str) {
    record(&format!(
        "size {} {label}",
        if condition { "PASS" } else { "FAILED" }
    ));
}

fn set_scale(app: &Rc<RefCell<App>>, scale: f64) {
    let mut settings = app.borrow().settings.clone();
    settings.island_scale = scale;
    apply_settings(app, settings, false);
}

fn audit(app: &Rc<RefCell<App>>, ui: &IslandWindow, scale: f64, name: &str) {
    let geometry = app.borrow().geometry(IslandState::Media);
    check(
        (f64::from(ui.get_ui_scale()) - scale).abs() < 0.001,
        &format!("{name} UI scale is applied"),
    );
    check(
        (f64::from(ui.get_shape_width()) - geometry.width).abs() < 0.5
            && (f64::from(ui.get_shape_height()) - geometry.height).abs() < 0.5,
        &format!("{name} retains logical media geometry"),
    );
    let left = (crate::state::OVERLAY_WIDTH - geometry.width) as f32 / 2.;
    if geometry.width >= 480. {
        check(
            ui.invoke_lyrics_scroll_hit(left + 36., 100.),
            &format!("{name} lyrics viewport hit remains aligned"),
        );
    } else {
        check(
            !ui.invoke_lyrics_scroll_hit(left + 36., 100.),
            &format!("{name} narrow layout omits the lyrics viewport"),
        );
    }
    check(
        !ui.invoke_lyrics_scroll_hit(left + geometry.width as f32 - 36., 100.),
        &format!("{name} playback controls stay outside lyrics wheel routing"),
    );
    audit_window(ui, scale, name);
    lyrics_verification::audit_native_bounds(ui);
    match crate::verification::snapshot(ui, name) {
        Ok(path) => record(&format!("snapshot {}", path.display())),
        Err(error) => record(&format!("snapshot FAILED {error}")),
    }
}

fn audit_window(ui: &IslandWindow, scale: f64, name: &str) {
    use slint::winit_030::WinitWindowAccessor;
    use windows::Win32::{
        Foundation::*,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::GetWindowRect},
    };
    ui.window().with_winit_window(|window| {
        let Ok(handle) = window.window_handle() else {
            check(false, "native size window handle available");
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            check(false, "native size HWND available");
            return;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        // SAFETY: the UI owns this live HWND for the synchronous read. The output
        // rectangle is initialized and Windows writes only its declared fields.
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                check(false, "native scaled bounds readable");
                return;
            }
            let effective = scale * f64::from(GetDpiForWindow(hwnd).max(96)) / 96.;
            let expected_width = crate::state::OVERLAY_WIDTH * effective;
            let expected_height = crate::state::OVERLAY_HEIGHT * effective;
            check(
                (f64::from(rect.right - rect.left) - expected_width).abs() <= 2.
                    && (f64::from(rect.bottom - rect.top) - expected_height).abs() <= 2.,
                &format!("{name} native canvas includes UI size and Windows DPI"),
            );
        }
    });
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    let saved_preferences = Rc::new(std::fs::read(Settings::path()).ok());
    let settled_frames = Rc::new(std::cell::Cell::new(0));
    {
        let mut settings = app.borrow().settings.clone();
        settings.form_factor = crate::shape::FormFactor::Notch;
        settings.appearance = Appearance::Solid;
        settings.island_scale = 1.;
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
    for (step, at) in [600, 1800, 2800, 3800, 4800, 6800, 7800]
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
                    check(
                        app.borrow().media.is_none(),
                        "no live media service or player commands",
                    );
                    ui.set_media_available(true);
                    ui.set_playing(false);
                    ui.set_track_title("Windowlight".into());
                    ui.set_artist_name("Still Island Studio".into());
                    ui.set_media_source("Original size verification fixture".into());
                    ui.set_player_label("Fixture player".into());
                    ui.set_can_play_pause(true);
                    ui.set_can_previous(true);
                    ui.set_can_next(true);
                    ui.set_can_seek(true);
                    ui.set_has_artwork(false);
                    ui.set_position_seconds(42.);
                    ui.set_duration_seconds(180.);
                    ui.set_progress(42. / 180.);
                    ui.set_elapsed_label("0:42".into());
                    ui.set_duration_label("3:00".into());
                    ui.set_lyrics_enabled(true);
                    ui.set_lyrics_loading(false);
                    ui.set_lyrics_available(true);
                    ui.set_lyrics_synced(false);
                    ui.set_lyrics_plain("The evening gathers at the door\nA gentle rhythm on the floor\nWe leave a little room for more".into());
                    ui.set_lyrics_status("".into());
                    set_scale(&app, 0.75);
                    transition(&app, IslandState::Media);
                }
                1 => {
                    audit(&app, &ui, 0.75, "size-75");
                    set_scale(&app, 1.);
                }
                2 => {
                    audit(&app, &ui, 1., "size-100");
                    set_scale(&app, 1.25);
                }
                3 => {
                    audit(&app, &ui, 1.25, "size-125");
                    set_scale(&app, 1.5);
                }
                4 => {
                    audit(&app, &ui, 1.5, "size-150");
                    set_scale(&app, 1.);
                    transition(&app, IslandState::Idle);
                }
                5 => {
                    check(
                        app.borrow().state == IslandState::Idle,
                        "close restores idle at one hundred percent",
                    );
                    check(
                        !app.borrow().timer.running(),
                        "size morph scheduling stops after settling",
                    );
                    check(
                        (ui.get_ui_scale() - 1.).abs() < 0.001,
                        "default UI size restored",
                    );
                    settled_frames.set(app.borrow().frames);
                }
                _ => {
                    let a = app.borrow();
                    check(
                        !a.timer.running() && !a.media_clock.running() && !a.deadline.running(),
                        "idle animation and presentation timers stay stopped",
                    );
                    check(
                        a.frames == settled_frames.get(),
                        "settled size schedules no further animation updates",
                    );
                    check(
                        std::fs::read(Settings::path()).ok() == *saved_preferences,
                        "user preferences are unchanged on disk",
                    );
                    record("size verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}
