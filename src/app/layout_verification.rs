//! Opt-in layout, typography, speed binding, and native tab regression.
use super::*;

fn check(ok: bool, label: &str) {
    record(&format!(
        "layout {} {label}",
        if ok { "PASS" } else { "FAILED" }
    ));
}

fn capture(ui: &IslandWindow, name: &str) {
    match crate::verification::snapshot(ui, name) {
        Ok(path) => record(&format!("snapshot {}", path.display())),
        Err(error) => record(&format!("snapshot FAILED {error}")),
    }
    lyrics_verification::audit_native_bounds(ui);
}

fn dimensions(ui: &IslandWindow, width: f32, height: f32, label: &str) {
    check(
        (ui.get_shape_width() - width).abs() < 0.5 && (ui.get_shape_height() - height).abs() < 0.5,
        label,
    );
    check(
        (ui.get_lyrics_font_pixels() - 18.).abs() < 0.01,
        "layout dimensions preserve eighteen pixel text",
    );
}

fn settings_tab(app: &Rc<RefCell<App>>, index: i32) {
    let Some(settings) = app.borrow().preferences.upgrade() else {
        check(false, "settings window exists");
        return;
    };
    settings.set_active_tab(index);
    refresh_preferences(app);
    if index == 0 {
        settings.invoke_apply_layout_preset(1);
        check(
            app.borrow().settings.expanded_width == 480.,
            "Roomy preset applies actual dimensions",
        );
        let before = app.borrow().settings.sizes();
        settings.set_text_scale_percent(125);
        settings.invoke_changed();
        let after = app.borrow().settings.sizes();
        check(
            before.expanded_width == after.expanded_width
                && before.expanded_height == after.expanded_height
                && before.contracted_width == after.contracted_width
                && before.media_width == after.media_width,
            "text size changes without geometry changing",
        );
        check(
            (app.borrow().ui.upgrade().unwrap().get_text_scale() - 1.25).abs() < 0.001,
            "independent text scale reaches island",
        );
        for state in 0..3 {
            settings.set_preview_state(state);
            settings.invoke_preview_changed();
            check(
                !settings.get_preview_path().is_empty() && settings.get_preview_height() > 0.,
                &format!("live preview state {state} has actual contour"),
            );
        }
        settings.set_preview_state(1);
        settings.invoke_preview_changed();
        settings.set_advanced_appearance(true);
        settings.set_appearance_index(1);
        settings.set_tint(0.35);
        settings.invoke_changed();
        check(
            settings.get_advanced_appearance()
                && (app.borrow().settings.opacity - 0.35).abs() < 0.001,
            "advanced group and tint update live",
        );
        let encoded = app.borrow().settings.encode();
        check(
            Settings::parse(&encoded).encode() == encoded,
            "preset and custom typography round trip",
        );
        settings.set_fullscreen_always_hide_text("C:\\invalid\\game.exe".into());
        let previous = app.borrow().settings.encode();
        settings.invoke_changed();
        check(
            app.borrow().settings.encode() == previous
                && settings.get_fullscreen_always_hide_text().starts_with("C:"),
            "invalid app rule is rejected and edit retained",
        );
        settings.set_fullscreen_always_hide_text("".into());
        settings.invoke_changed();
        settings.invoke_preview_motion();
        check(
            settings.get_preview_state() == 0,
            "motion preview starts locally",
        );
        // A manual choice cancels outstanding motion callbacks.
        settings.set_preview_state(1);
        settings.invoke_preview_changed();
    }
    check(
        settings.get_active_tab() == index,
        &format!("native tab {index} remains selected through model refresh"),
    );
    match crate::verification::snapshot_preferences(&settings) {
        Ok(path) => {
            let copy = path.with_file_name(format!("layout-settings-tab{index}.ppm"));
            match std::fs::copy(&path, &copy) {
                Ok(_) => record(&format!("settings snapshot {}", copy.display())),
                Err(error) => record(&format!("snapshot FAILED {error}")),
            }
        }
        Err(error) => record(&format!("snapshot FAILED {error}")),
    }
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    let saved = Rc::new(std::fs::read(Settings::path()).ok());
    let mut settings = app.borrow().settings.clone();
    settings.form_factor = crate::shape::FormFactor::Notch;
    settings.appearance = Appearance::Solid;
    settings.island_scale = 1.;
    settings.text_scale = 1.;
    settings.fullscreen_always_hide.clear();
    settings.fullscreen_always_show.clear();
    settings.animation_speed = 1.;
    settings.animations = false;
    settings.compact_now_playing = false;
    settings.auto_volume = false;
    settings.battery_previews = false;
    settings.notifications_enabled = false;
    settings.hover_expand = false;
    settings.outside_click = false;
    settings.shortcut_enabled = false;
    settings.fullscreen_hide = false;
    settings.topmost = true;
    settings.contracted_width = 220.;
    settings.contracted_height = 52.;
    settings.expanded_width = 600.;
    settings.expanded_height = 240.;
    settings.media_width = 900.;
    settings.media_height = 300.;
    apply_settings(app, settings, false);
    for (step, at) in [
        500, 1200, 2100, 3000, 3900, 4700, 5500, 6500, 7300, 8100, 8900, 9800, 11200,
    ]
    .into_iter()
    .enumerate()
    {
        let weak = Rc::downgrade(app);
        let saved = saved.clone();
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
                        "synthetic fixture excludes live media commands",
                    );
                    ui.set_media_available(true);
                    ui.set_playing(false);
                    ui.set_track_title("Windowlight".into());
                    ui.set_artist_name("Still Island Studio".into());
                    ui.set_player_label("Fixture player".into());
                    ui.set_media_source("Original layout fixture".into());
                    ui.set_can_play_pause(true);
                    ui.set_can_seek(true);
                    ui.set_can_previous(true);
                    ui.set_can_next(true);
                    ui.set_duration_seconds(180.);
                    ui.set_duration_label("3:00".into());
                    ui.set_has_artwork(false);
                    ui.set_lyrics_available(true);
                    ui.set_lyrics_synced(false);
                    ui.set_lyrics_plain("The evening gathers at the door\nA gentle rhythm on the floor\nWe leave a little room for more".into());
                    transition(&app, IslandState::Idle);
                }
                1 => {
                    dimensions(&ui, 220., 52., "custom contracted dimensions apply");
                    capture(&ui, "layout-contracted");
                    transition(&app, IslandState::Expanded);
                }
                2 => {
                    dimensions(&ui, 600., 240., "custom expanded dimensions apply");
                    capture(&ui, "layout-expanded");
                    transition(&app, IslandState::Media);
                }
                3 => {
                    let expected = app.borrow().geometry(IslandState::Media);
                    dimensions(
                        &ui,
                        expected.width as f32,
                        300.,
                        "custom media dimensions apply with monitor fit",
                    );
                    if platform::available_width() >= 800. {
                        check(
                            expected.width > 640.,
                            "media width can exceed the former fixed width",
                        );
                    }
                    capture(&ui, "layout-media");
                    let mut settings = app.borrow().settings.clone();
                    settings.island_scale = 1.5;
                    apply_settings(&app, settings, false);
                }
                4 => {
                    check(
                        (ui.get_lyrics_font_pixels() - 27.).abs() < 0.01,
                        "zoom explicitly rasterizes twenty-seven pixel text",
                    );
                    capture(&ui, "layout-media-150");
                    let mut settings = app.borrow().settings.clone();
                    settings.island_scale = 1.;
                    settings.animation_speed = 0.5;
                    apply_settings(&app, settings, false);
                }
                5 => {
                    check(
                        (ui.get_animation_speed() - 0.5).abs() < 0.01,
                        "fifty percent motion setting reaches the UI",
                    );
                    let mut settings = app.borrow().settings.clone();
                    settings.animation_speed = 2.;
                    apply_settings(&app, settings, false);
                }
                6 => {
                    check(
                        (ui.get_animation_speed() - 2.).abs() < 0.01,
                        "two hundred percent motion setting reaches the UI",
                    );
                    show_preferences(&app);
                }
                7..=10 => settings_tab(&app, (step - 7) as i32),
                11 => {
                    close_preferences(&app, true);
                    transition(&app, IslandState::Idle);
                }
                _ => {
                    check(
                        !app.borrow().timer.running() && !app.borrow().media_clock.running(),
                        "layout settles without background animation scheduling",
                    );
                    check(
                        std::fs::read(Settings::path()).ok() == *saved,
                        "saved user preferences remain unchanged",
                    );
                    record("layout verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}
