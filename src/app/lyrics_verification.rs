//! Opt-in, network-free lyrics GUI regression with original synthetic content.
use super::*;
use crate::services::lyrics::{LyricsContent, LyricsEvent, LyricsLine, LyricsStatus, TrackKey};

const FIRST: &str = "A quiet spark against the sky";
const SECOND: &str = "The city turns its windows bright";
const THIRD: &str = "We carry little pools of light";

struct Fixture {
    track: TrackKey,
    generation: u64,
    paused_position: f64,
    settled_frames: u64,
    settled_metrics: String,
}

fn check(condition: bool, label: &str) {
    record(&format!(
        "lyrics {} {label}",
        if condition { "PASS" } else { "FAILED" }
    ));
}

fn capture(ui: &IslandWindow, name: &str) {
    match crate::verification::snapshot(ui, name) {
        Ok(path) => record(&format!("snapshot {}", path.display())),
        Err(error) => record(&format!("snapshot FAILED {error}")),
    }
}

fn synced() -> Arc<LyricsContent> {
    Arc::new(LyricsContent::Synced(
        [(0., FIRST), (2., SECOND), (4., THIRD)]
            .into_iter()
            .map(|(timestamp_seconds, text)| LyricsLine {
                timestamp_seconds,
                text: text.into(),
            })
            .collect(),
    ))
}

fn deliver(app: &Rc<RefCell<App>>, fixture: &Fixture, status: LyricsStatus) {
    lyrics::verification_receive(
        app,
        LyricsEvent {
            generation: fixture.generation,
            track: fixture.track.clone(),
            status,
        },
    );
}

fn begin(app: &Rc<RefCell<App>>, ui: &IslandWindow, fixture: &mut Fixture) {
    fixture.generation = lyrics::verification_begin(app, fixture.track.clone());
    ui.set_lyrics_loading(true);
    ui.set_lyrics_available(false);
    ui.set_lyrics_synced(false);
    ui.set_lyrics_previous("".into());
    ui.set_lyrics_current("".into());
    ui.set_lyrics_next("".into());
    ui.set_lyrics_plain("".into());
    ui.set_lyrics_status("Finding lyrics on LRCLIB…".into());
}

fn cursor(ui: &IslandWindow, position: f64, playing: bool) {
    ui.set_playing(playing);
    ui.set_position_seconds(position as f32);
    // Exercise the normal snapshot callback and shared playback clock. The media
    // service is absent in this verification mode, so no player receives commands.
    ui.invoke_media_updated();
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    // Observe an actual in-flight Slint animation, then interrupt it twice.
    // The seven row identities remain fixed while the numeric cursor retargets.
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(2820), move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let Some(ui) = app.borrow().ui.upgrade() else {
            return;
        };
        let before = ui.get_lyrics_visual_cursor();
        check(
            ui.get_lyrics_motion_active() && before > 0. && before < 1.,
            "line transition has continuous intermediate movement",
        );
        capture(&ui, "lyrics-moving");
        check(
            ui.invoke_lyric_font_size(0.) < 18. && ui.invoke_lyric_font_size(1.) > 13.,
            "finished line shrinks while next line grows during upward movement",
        );
        ui.invoke_move_lyrics(2., true);
        check(
            (ui.get_lyrics_visual_cursor() - before).abs() < 0.025,
            "interrupted lyric movement preserves rendered position",
        );
        ui.invoke_move_lyrics(1., true);
    });
    let fixture = Rc::new(RefCell::new(Fixture {
        track: TrackKey::new(
            "Windowlight",
            "Still Island Studio",
            "Original fixture",
            180.,
        )
        .expect("valid static track fixture"),
        generation: 0,
        paused_position: 0.,
        settled_frames: 0,
        settled_metrics: String::new(),
    }));
    // Ignore the redraw requested by the explicit idle screenshot, then observe
    // both Slint's renderer and the Rust morph scheduler remaining quiet.
    let weak = Rc::downgrade(app);
    let render_fixture = fixture.clone();
    slint::Timer::single_shot(Duration::from_millis(11100), move || {
        if let Some(app) = weak.upgrade() {
            render_fixture.borrow_mut().settled_metrics = app.borrow().metrics.summary();
        }
    });
    for (step, delay) in [
        700, 1800, 3200, 3400, 4400, 4700, 5000, 6200, 7000, 8000, 8500, 9100, 9500, 10800, 11800,
    ]
    .into_iter()
    .enumerate()
    {
        let weak = Rc::downgrade(app);
        let fixture = fixture.clone();
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
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
                    {
                        let mut a = app.borrow_mut();
                        a.settings.form_factor = crate::shape::FormFactor::Notch;
                        a.settings.island_scale = 1.;
                        a.settings.notch_corner_radius = 8.;
                        a.settings.online_lyrics = true;
                        a.settings.appearance = Appearance::Solid;
                        a.settings.animations = true;
                        a.settings.animation_speed = 1.;
                        a.settings.follow_reduced_motion = false;
                        a.settings.topmost = true;
                    }
                    configure_material(&app);
                    ui.set_lyrics_enabled(true);
                    ui.set_media_available(true);
                    ui.set_track_title("Windowlight".into());
                    ui.set_artist_name("Still Island Studio".into());
                    ui.set_media_source("Original verification fixture".into());
                    ui.set_player_label("Fixture player".into());
                    ui.set_can_play_pause(true);
                    ui.set_can_previous(true);
                    ui.set_can_next(true);
                    ui.set_can_seek(true);
                    ui.set_has_artwork(false);
                    ui.set_duration_seconds(180.);
                    ui.set_duration_label("3:00".into());
                    transition(&app, IslandState::Media);
                    cursor(&ui, 0., true);
                    let mut fixture = fixture.borrow_mut();
                    begin(&app, &ui, &mut fixture);
                    deliver(&app, &fixture, LyricsStatus::Found(synced()));
                    check(
                        ui.get_lyrics_current() == FIRST,
                        "initial synchronized line",
                    );
                    check(
                        ui.invoke_lyric_font_size(0.) == 18.
                            && ui.invoke_lyric_font_size(-1.) == 13.
                            && ui.invoke_lyric_font_size(1.) == 13.,
                        "current lyric is larger than previous and next lines",
                    );
                }
                1 => {
                    let expected = app.borrow().geometry(IslandState::Media);
                    check(
                        (f64::from(ui.get_shape_width()) - expected.width).abs() < 0.5
                            && (ui.get_shape_height() - 216.).abs() < 0.5
                            && (ui.get_shape_radius() - 8.).abs() < 0.25,
                        "wide media geometry and default eight pixel corners",
                    );
                    if platform::available_width() >= 800. {
                        check(
                            (ui.get_shape_width() - 640.).abs() < 0.5,
                            "preferred 640 pixel media width",
                        );
                    }
                    check(
                        ui.get_lyrics_available() && ui.get_lyrics_synced(),
                        "synced lyrics displayed",
                    );
                    check(
                        ui.get_lyrics_next() == SECOND,
                        "next synchronized line prepared",
                    );
                    capture(&ui, "lyrics-wide");
                    audit_native_bounds(&ui);
                }
                2 => {
                    check(
                        !ui.get_lyrics_motion_active()
                            && (ui.get_lyrics_visual_cursor() - 1.).abs() < 0.001,
                        "lyric movement settles at the next line",
                    );
                    check(
                        ui.get_lyrics_current() == SECOND,
                        "shared playback clock advances lyrics",
                    );
                    check(
                        ui.get_lyrics_previous() == FIRST,
                        "previous synchronized line retained",
                    );
                    check(
                        app.borrow().media_clock.running(),
                        "playing uses existing media clock",
                    );
                }
                3 => {
                    let position = lyrics::position(&app.borrow(), &ui);
                    fixture.borrow_mut().paused_position = position;
                    cursor(&ui, position, false);
                }
                4 => {
                    check(
                        !app.borrow().media_clock.running(),
                        "pause stops playback clock",
                    );
                    check(
                        (lyrics::position(&app.borrow(), &ui) - fixture.borrow().paused_position)
                            .abs()
                            < 0.001,
                        "paused playback cursor remains stationary",
                    );
                    check(
                        ui.get_lyrics_current() == SECOND,
                        "pause keeps current synchronized line",
                    );
                }
                5 => {
                    cursor(&ui, 0.2, false);
                    check(
                        !ui.get_lyrics_motion_active()
                            && ui.get_lyrics_visual_cursor().abs() < 0.001,
                        "backward seek rebases lyrics without scrolling across old lines",
                    );
                    check(
                        ui.get_lyrics_current() == FIRST,
                        "backward seek restores earlier line",
                    );
                    check(
                        ui.get_lyrics_previous().is_empty(),
                        "backward seek clears stale previous line",
                    );
                    ui.invoke_move_lyrics(1., true);
                    let mut settings = app.borrow().settings.clone();
                    settings.animations = false;
                    apply_settings(&app, settings.clone(), false);
                    check(
                        !ui.get_lyrics_motion_active()
                            && (ui.get_lyrics_visual_cursor() - 1.).abs() < 0.001,
                        "disabling animation stops lyric movement immediately",
                    );
                    ui.invoke_move_lyrics(0., false);
                    settings.animations = true;
                    apply_settings(&app, settings, false);
                }
                6 => {
                    app.borrow_mut().settings.notch_corner_radius = 16.;
                    app.borrow_mut().retarget_shape();
                    wake(&app);
                }
                7 => {
                    check(
                        (ui.get_shape_radius() - 16.).abs() < 0.25,
                        "custom sixteen pixel corners applied",
                    );
                    check(
                        ui.get_lyrics_current() == FIRST,
                        "corner morph preserves lyrics content",
                    );
                    capture(&ui, "lyrics-custom");
                    audit_native_bounds(&ui);
                }
                8 => {
                    let mut fixture = fixture.borrow_mut();
                    begin(&app, &ui, &mut fixture);
                    deliver(
                        &app,
                        &fixture,
                        LyricsStatus::Found(Arc::new(LyricsContent::Plain(vec![
                            "The evening gathers at the door".into(),
                            "A gentle rhythm on the floor".into(),
                            "We leave a little room for more".into(),
                        ]))),
                    );
                }
                9 => {
                    check(
                        ui.get_lyrics_available() && !ui.get_lyrics_synced(),
                        "plain lyrics fallback displayed",
                    );
                    check(
                        ui.get_lyrics_plain().contains("The evening gathers"),
                        "plain text delivered intact",
                    );
                    check(
                        ui.get_lyrics_current().is_empty(),
                        "plain fallback clears timed content",
                    );
                    if ui.get_shape_width() >= 480. {
                        let left = (crate::state::OVERLAY_WIDTH as f32 - ui.get_shape_width()) / 2.;
                        check(
                            ui.invoke_lyrics_scroll_hit(left + 36., 100.),
                            "plain lyrics viewport receives wheel scrolling",
                        );
                        check(
                            !ui.invoke_lyrics_scroll_hit(left + ui.get_shape_width() - 36., 100.),
                            "media controls retain configurable volume gesture",
                        );
                    }
                    capture(&ui, "lyrics-plain");
                    // A narrow panel may cross the two-column breakpoint while
                    // overshooting. Its destination must keep the layout stable.
                    let width = app.borrow().springs[0].position;
                    app.borrow_mut().springs[0].snap(500.);
                    app.borrow().render();
                    ui.set_media_layout_width(470.);
                    check(
                        !ui.invoke_lyrics_scroll_hit(186., 100.),
                        "narrow destination keeps one column during overshoot",
                    );
                    capture(&ui, "lyrics-narrow-overshoot");
                    app.borrow_mut().springs[0].snap(width);
                    app.borrow().update_media_layout();
                    app.borrow().render();
                }
                10 => {
                    let old_track = fixture.borrow().track.clone();
                    let old_generation = fixture.borrow().generation;
                    {
                        let mut fixture = fixture.borrow_mut();
                        fixture.track = TrackKey::new(
                            "Afterlight",
                            "Still Island Studio",
                            "Original fixture",
                            180.,
                        )
                        .expect("valid replacement track");
                        begin(&app, &ui, &mut fixture);
                    }
                    lyrics::verification_receive(
                        &app,
                        LyricsEvent {
                            generation: old_generation,
                            track: fixture.borrow().track.clone(),
                            status: LyricsStatus::Found(synced()),
                        },
                    );
                    check(
                        ui.get_lyrics_loading() && !ui.get_lyrics_available(),
                        "stale generation rejected",
                    );
                    lyrics::verification_receive(
                        &app,
                        LyricsEvent {
                            generation: fixture.borrow().generation,
                            track: old_track,
                            status: LyricsStatus::Found(synced()),
                        },
                    );
                    check(
                        ui.get_lyrics_loading() && ui.get_lyrics_current().is_empty(),
                        "stale track signature rejected",
                    );
                    deliver(&app, &fixture.borrow(), LyricsStatus::NotFound);
                }
                11 => {
                    check(
                        !ui.get_lyrics_loading() && !ui.get_lyrics_available(),
                        "no result exits loading state",
                    );
                    check(
                        ui.get_lyrics_status().contains("No matching lyrics"),
                        "no result is clearly explained",
                    );
                    capture(&ui, "lyrics-no-result");
                }
                12 => {
                    cursor(&ui, 0.2, false);
                    transition(&app, IslandState::Idle);
                }
                13 => {
                    check(
                        app.borrow().state == IslandState::Idle,
                        "close restores idle capsule",
                    );
                    check(
                        !app.borrow().timer.running(),
                        "morph timer stops after settling",
                    );
                    fixture.borrow_mut().settled_frames = app.borrow().frames;
                    capture(&ui, "lyrics-settled");
                }
                _ => {
                    let a = app.borrow();
                    check(
                        !a.timer.running() && !a.media_clock.running() && !a.deadline.running(),
                        "animation, playback and presentation scheduling remain stopped",
                    );
                    check(
                        a.frames == fixture.borrow().settled_frames,
                        "no additional animation updates at idle",
                    );
                    check(
                        !ui.get_lyrics_motion_active(),
                        "lyric animation scheduling stops at idle",
                    );
                    check(
                        a.metrics.summary() == fixture.borrow().settled_metrics,
                        "Slint schedules no further renderer frames after lyric motion settles",
                    );
                    record(&format!(
                        "lyrics verification complete frames={} active={}",
                        a.frames,
                        a.timer.running()
                    ));
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}

/// Read back the actual Win32 region and desktop hit target near both widened
/// sides. This verifies cross-process exclusion beyond the renderer's alpha test.
pub(super) fn audit_native_bounds(ui: &IslandWindow) {
    use slint::winit_030::WinitWindowAccessor;
    use windows::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    };
    record(&format!("lyrics {}", platform::diagnostic_snapshot()));
    ui.window().with_winit_window(|window| {
        let Ok(handle) = window.window_handle() else {
            check(false, "native window handle available");
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            check(false, "native Windows handle available");
            return;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        // SAFETY: the UI-owned HWND stays live throughout this synchronous read.
        // Every stack output is initialized; the copied HRGN is deleted once.
        unsafe {
            let scale =
                f64::from(GetDpiForWindow(hwnd).max(96)) / 96. * f64::from(ui.get_ui_scale());
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                check(false, "native bounds readable");
                return;
            }
            let region = CreateRectRgn(0, 0, 0, 0);
            if region.0.is_null() {
                check(false, "native region copy allocated");
                return;
            }
            let _ = GetWindowRgn(hwnd, region);
            let center = f64::from(ui.get_canvas_width()) * scale / 2.;
            let half = f64::from(ui.get_shape_width()) * scale / 2.;
            let y = (f64::from(ui.get_shape_height()) * scale / 2.).round() as i32;
            for (label, x, expected) in [
                ("left body accepts clicks", center - half + 3., true),
                ("right body accepts clicks", center + half - 3., true),
                (
                    "left transparent side passes clicks",
                    center - half - 3.,
                    false,
                ),
                (
                    "right transparent side passes clicks",
                    center + half + 3.,
                    false,
                ),
            ] {
                let x = x.round() as i32;
                check(
                    PtInRegion(region, x, y).as_bool() == expected,
                    &format!("region {label}"),
                );
                check(
                    (WindowFromPoint(POINT {
                        x: rect.left + x,
                        y: rect.top + y,
                    }) == hwnd)
                        == expected,
                    &format!("desktop {label}"),
                );
            }
            let _ = DeleteObject(HGDIOBJ(region.0));
        }
    });
}
