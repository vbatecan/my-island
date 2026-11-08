//! Network-free renderer regression for finite media motion and readable lyrics.
use super::*;

fn check(ok: bool, label: &str) {
    record(&format!(
        "media polish {} {label}",
        if ok { "PASS" } else { "FAILED" }
    ));
}

fn capture(ui: &IslandWindow, name: &str) {
    match crate::verification::snapshot(ui, name) {
        Ok(path) => record(&format!("snapshot {}", path.display())),
        Err(error) => record(&format!("media polish FAILED snapshot {error}")),
    }
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    let settled = Rc::new(RefCell::new(String::new()));
    for (step, delay) in [
        700, 1500, 1560, 1850, 2100, 2500, 2700, 3300, 3900, 4500, 5100, 5800, 6300, 7100,
    ]
    .into_iter()
    .enumerate()
    {
        let weak = Rc::downgrade(app);
        let settled = settled.clone();
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            match step {
                0 => {
                    check(
                        app.borrow().media.is_none(),
                        "fixture sends no live player commands",
                    );
                    let mut settings = app.borrow().settings.clone();
                    settings.media_width = 480.;
                    settings.media_height = 216.;
                    settings.text_scale = 1.4;
                    settings.island_scale = 1.;
                    settings.online_lyrics = false;
                    settings.follow_reduced_motion = false;
                    settings.animations = true;
                    settings.animation_speed = 1.;
                    apply_settings(&app, settings, false);
                    transition(&app, IslandState::Media);
                    media::apply_media(&ui, &MediaSnapshot {
                        title: "A beautifully long track title that wraps gracefully onto another line".into(),
                        artist: "An artist with a longer name".into(),
                        source: "Original verification fixture".into(),
                        available: true,
                        can_play_pause: true,
                        duration_seconds: 180.,
                        ..MediaSnapshot::default()
                    }, false);
                    ui.invoke_media_updated();
                    ui.set_lyrics_available(true);
                    ui.set_lyrics_synced(true);
                    ui.set_lyrics_current(
                        "We leave a little room for the evening light to find us".into(),
                    );
                    let rows = [
                        "A quiet spark",
                        "We leave a little room for the evening light to find us",
                        "",
                        "The next verse begins",
                        "A final line",
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, text)| crate::LyricRow {
                        text: text.into(),
                        line_index: i as i32 - 1,
                    })
                    .collect::<Vec<_>>();
                    ui.set_lyrics_rows(slint::ModelRc::from(Rc::new(slint::VecModel::from(rows))));
                    ui.invoke_move_lyrics(0., false);
                }
                1 => {
                    check(
                        (ui.invoke_lyric_font_size(0.) - 25.2).abs() < 0.01,
                        "font scale changes rasterized lyric size independently of geometry",
                    );
                    check(
                        (ui.get_shape_width() - 480.).abs() < 1.,
                        "smallest two-column media width remains independent of text size",
                    );
                    check(!ui.get_can_seek(), "unsupported seeking remains disabled");
                    capture(&ui, "media-polish-large-text-narrow");
                    ui.invoke_media_change();
                }
                2 => {
                    check(
                        ui.get_media_change_active(),
                        "track change has a finite intermediate presentation",
                    );
                    ui.invoke_media_change();
                    capture(&ui, "media-polish-track-moving");
                }
                3 => {
                    check(
                        !ui.get_media_change_active(),
                        "rapid metadata updates do not restart the active pulse",
                    );
                    ui.set_media_motion_enabled(false);
                    ui.invoke_media_change();
                    check(
                        !ui.get_media_change_active(),
                        "disabled motion cannot start a media pulse",
                    );
                    ui.set_media_motion_enabled(true);
                    ui.set_lyrics_current("".into());
                    ui.invoke_move_lyrics(1., true);
                }
                4 => check(
                    ui.get_lyrics_gap_active(),
                    "blank timed line presents an instrumental break",
                ),
                5 => capture(&ui, "media-polish-instrumental-gap"),
                6 => {
                    ui.set_lyrics_current("The next verse begins".into());
                    ui.invoke_move_lyrics(2., false);
                    check(
                        !ui.get_lyrics_motion_active()
                            && (ui.get_lyrics_visual_cursor() - 2.).abs() < 0.001,
                        "seek snaps to new lyric without traversing old lines",
                    );
                    check(
                        !ui.get_lyrics_gap_active(),
                        "seeking to vocals clears instrumental hint",
                    );
                    ui.set_media_motion_enabled(false);
                    ui.set_playing(true);
                    ui.set_playing(false);
                    check(
                        !ui.get_media_change_active(),
                        "hidden or reduced motion changes remain stationary",
                    );
                }
                7 => {
                    transition(&app, IslandState::Volume);
                    ui.set_volume_available(true);
                    ui.set_volume(72);
                }
                8 => {
                    capture(&ui, "media-polish-volume-large-text");
                    transition(&app, IslandState::Battery);
                    ui.set_battery_percent_label("82%".into());
                    ui.set_battery_power_label("Connected to power".into());
                    ui.set_battery_level(0.82);
                }
                9 => {
                    capture(&ui, "media-polish-battery-large-text");
                    transition(&app, IslandState::Timer);
                }
                10 => {
                    capture(&ui, "media-polish-timer-large-text");
                    {
                        let mut a = app.borrow_mut();
                        let now = a.now();
                        a.coordinator.close(now);
                        a.coordinator.set_media(true, now);
                    }
                    sync_presentation(&app);
                }
                11 => {
                    // Pending synthetic content is applied during the morph;
                    // select this fixture after it has finished mounting.
                    ui.set_compact_media(true);
                    capture(&ui, "media-polish-compact-large-text");
                }
                12 => *settled.borrow_mut() = app.borrow().metrics.summary(),
                _ => {
                    check(
                        app.borrow().metrics.summary() == *settled.borrow(),
                        "no recurring renderer frames after media and lyric motion settle",
                    );
                    check(
                        !app.borrow().timer.running() && !app.borrow().media_clock.running(),
                        "no morph or playback clock remains active while paused",
                    );
                    record("media polish verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}
