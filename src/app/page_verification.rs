//! Finite, opt-in renderer/IPC regression. Uses an offline Codex fixture only.
use super::*;
use crate::{PluginManagerWindow, navigation::Page};
use slint::Model;

const CODEX: &str = "org.stillisland.codex-usage";

fn codex() -> Page {
    Page::Plugin {
        id: CODEX.into(),
        view: "main".into(),
    }
}
fn check(ok: bool, label: &str) {
    record(&format!(
        "pages {} {label}",
        if ok { "PASS" } else { "FAILED" }
    ));
}
fn selected(app: &Rc<RefCell<App>>, page: &Page) -> bool {
    app.borrow().settings.selected_page.as_ref() == Some(page)
}
fn capture(ui: &IslandWindow, name: &str) {
    match crate::verification::snapshot(ui, name) {
        Ok(path) => record(&format!("snapshot {}", path.display())),
        Err(error) => record(&format!("pages FAILED snapshot {error}")),
    }
}
fn send_wheel(app: &Rc<RefCell<App>>, point: (f64, f64)) -> slint::winit_030::EventResult {
    use slint::winit_030::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
    app.borrow_mut().pointer = Some(point);
    handle_overlay_input(
        app,
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0., -1.),
            phase: TouchPhase::Moved,
        },
    )
}
fn next_page(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    let result = send_wheel(app, (f64::from(ui.get_canvas_width()) / 2., 16.));
    check(
        matches!(result, slint::winit_030::EventResult::PreventDefault),
        "header wheel consumed by page navigation",
    );
}
fn protected_wheel(
    app: &Rc<RefCell<App>>,
    ui: &IslandWindow,
    hit: fn(&IslandWindow, f32, f32) -> bool,
    label: &str,
) {
    let point = (0..ui.get_canvas_height() as i32).step_by(2).find_map(|y| {
        (0..ui.get_canvas_width() as i32)
            .step_by(2)
            .find_map(|x| hit(ui, x as f32, y as f32).then_some((f64::from(x), f64::from(y))))
    });
    let before = app.borrow().settings.selected_page.clone();
    if let Some(point) = point {
        let _ = send_wheel(app, point);
        check(app.borrow().settings.selected_page == before, label);
    } else {
        check(false, &format!("{label}: no rendered hit region"));
    }
}
fn finite(app: &Rc<RefCell<App>>) -> bool {
    app.borrow()
        .springs
        .iter()
        .all(|spring| spring.position.is_finite() && spring.velocity.is_finite())
}

pub(super) fn setup(app: &Rc<RefCell<App>>, ui: &IslandWindow, window: &PluginManagerWindow) {
    let argument =
        |prefix: &str| std::env::args().find_map(|arg| arg.strip_prefix(prefix).map(str::to_owned));
    let (Some(package), Some(executable)) = (
        argument("--verify-plugin-package="),
        argument("--verify-codex-executable="),
    ) else {
        check(false, "offline package/executable arguments required");
        let _ = slint::quit_event_loop();
        return;
    };
    let frames = Rc::new(std::cell::Cell::new(0u64));
    let rendered = Rc::new(RefCell::new(String::new()));
    for (step, delay) in [
        500, 1300, 1900, 2500, 3100, 4400, 5500, 6500, 7500, 8400, 9500, 10200, 11300, 12200,
        13200, 14000, 14900,
    ]
    .into_iter()
    .enumerate()
    {
        let weak = Rc::downgrade(app);
        let ui = ui.as_weak();
        let window = window.as_weak();
        let package = package.clone();
        let executable = executable.clone();
        let frames = frames.clone();
        let rendered = rendered.clone();
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
            let (Some(app), Some(ui), Some(window)) =
                (weak.upgrade(), ui.upgrade(), window.upgrade())
            else {
                return;
            };
            match step {
                0 => {
                    {
                        let mut a = app.borrow_mut();
                        a.media = None;
                        a.volume = None;
                        a.media_clock.stop();
                    }
                    check(
                        app.borrow().media.is_none() && app.borrow().volume.is_none(),
                        "no real player or system-volume commands",
                    );
                    let mut settings = app.borrow().settings.clone();
                    settings.appearance = Appearance::Solid;
                    settings.animations = true;
                    settings.follow_reduced_motion = false;
                    settings.animation_speed = 1.;
                    settings.text_scale = 1.;
                    settings.island_scale = 1.;
                    settings.expanded_width = 360.;
                    settings.expanded_height = 160.;
                    settings.media_width = 640.;
                    settings.online_lyrics = false;
                    settings.hover_expand = true;
                    settings.hover_delay = 150;
                    settings.collapse_delay = 200;
                    settings.selected_page = None;
                    apply_settings(&app, settings, false);
                    let Some(host) = app.borrow().plugin_host.clone() else {
                        check(false, "isolated plugin host available");
                        return;
                    };
                    check(
                        host.list().is_empty(),
                        "verification starts with isolated empty plugin directory",
                    );
                    let result = host
                        .import(std::path::Path::new(&package))
                        .and_then(|manifest| {
                            if manifest.id != CODEX {
                                return Err("Wrong verification package".into());
                            }
                            host.configure(CODEX, serde_json::json!({"executable":executable}))?;
                            host.set_enabled(CODEX, true)
                        });
                    check(
                        result.is_ok(),
                        "offline Codex package imported configured and explicitly enabled",
                    );
                    if let Err(error) = result {
                        record(&format!("pages FAILED fixture import: {error}"));
                    }
                    let _ = window.hide();
                    media::apply_media(
                        &ui,
                        &MediaSnapshot {
                            title: "A quiet synthetic track".into(),
                            artist: "Original fixture".into(),
                            available: true,
                            duration_seconds: 180.,
                            ..MediaSnapshot::default()
                        },
                        false,
                    );
                    ui.set_volume_available(true);
                    ui.set_volume(46);
                    ui.invoke_hover(true);
                    pages::open(&app, Page::Home);
                }
                1 => {
                    check(
                        selected(&app, &Page::Home) && ui.get_page_count() == 5,
                        "Home plus four named destinations in deterministic page list",
                    );
                    next_page(&app, &ui);
                }
                2 => {
                    check(
                        selected(&app, &Page::Media) && ui.get_media_mode(),
                        "wheel Home to Media",
                    );
                    ui.set_lyrics_available(true);
                    ui.set_lyrics_synced(false);
                    ui.set_lyrics_plain(
                        "Original fixture verse\nA second fixture line\nA third fixture line"
                            .into(),
                    );
                    protected_wheel(
                        &app,
                        &ui,
                        |ui, x, y| ui.invoke_lyrics_scroll_hit(x, y),
                        "wheel over plain lyrics retains Media",
                    );
                    ui.set_lyrics_available(false);
                    ui.set_player_picker_open(true);
                    next_page_picker_guard(&app, &ui);
                    ui.set_player_picker_open(false);
                    next_page(&app, &ui);
                }
                3 => {
                    check(
                        selected(&app, &Page::Timer) && ui.get_timer_mode(),
                        "wheel Media to Timer",
                    );
                    next_page(&app, &ui);
                }
                4 => {
                    check(
                        selected(&app, &Page::Volume) && ui.get_volume_mode(),
                        "wheel Timer to Volume",
                    );
                    protected_wheel(
                        &app,
                        &ui,
                        |ui, x, y| ui.invoke_volume_scroll_hit(x, y),
                        "wheel over volume slider retains Volume",
                    );
                    check(
                        ui.get_volume() == 46 && app.borrow().volume.is_none(),
                        "volume gesture fixture cannot mutate real system volume",
                    );
                    next_page(&app, &ui);
                }
                5 => {
                    check(
                        selected(&app, &codex()) && ui.get_plugin_mode(),
                        "wheel Volume to Codex",
                    );
                    let nodes = (0..ui.get_plugin_nodes().row_count())
                        .filter_map(|index| ui.get_plugin_nodes().row_data(index))
                        .collect::<Vec<_>>();
                    check(
                        nodes.iter().any(|n| n.text.contains("25.5"))
                            && nodes.iter().any(|n| n.text.contains("63")),
                        "actual plugin IPC renders both independent quota windows",
                    );
                    check(
                        nodes.iter().any(|n| n.font_size >= 24.)
                            && nodes
                                .iter()
                                .any(|n| n.kind == "progress" && n.value > 0. && n.value < 1.),
                        "semantic metric typography and normalized progress rendered",
                    );
                    check(
                        (ui.get_shape_width() - 480.).abs() < 1.
                            && (ui.get_shape_height() - 240.).abs() < 1.,
                        "Codex default panel is 480 by 240 without font stretching",
                    );
                    capture(&ui, "pages-codex-default");
                    let original = ui.get_plugin_view_height();
                    ui.set_plugin_view_height(800.);
                    protected_wheel(
                        &app,
                        &ui,
                        |ui, x, y| ui.invoke_plugin_scroll_hit(x, y),
                        "wheel over overflowing plugin content retains Codex",
                    );
                    ui.set_plugin_view_height(original);
                    ui.invoke_hover(false);
                }
                6 => {
                    check(
                        !app.borrow().coordinator.panel_open()
                            && matches!(app.borrow().presentation.as_ref().map(|p|&p.content),Some(Content::CompactPluginPage {plugin_id,..}) if plugin_id==CODEX),
                        "hover leave contracts into remembered Codex page",
                    );
                    check(
                        ui.get_compact_plugin()
                            && ui.get_plugin_title().contains("63")
                            && ui.get_plugin_body().contains("7-day")
                            && ui.get_plugin_has_progress()
                            && (ui.get_plugin_progress() - 0.63).abs() < 0.001,
                        "sticky compact badge uses most constrained server-reported window",
                    );
                    capture(&ui, "pages-codex-compact");
                    frames.set(app.borrow().frames);
                    *rendered.borrow_mut() = app.borrow().metrics.summary();
                }
                7 => {
                    check(
                        app.borrow().frames == frames.get()
                            && !app.borrow().timer.running()
                            && !app.borrow().page_switch_timer.running(),
                        "compact badge settles without recurring morph frames",
                    );
                    check(
                        app.borrow().metrics.summary() == *rendered.borrow(),
                        "compact badge schedules no further renderer frames after settling",
                    );
                    // Synthetic playback exercises the same media-updated
                    // binding as WinRT, without touching an actual player.
                    let mut settings = app.borrow().settings.clone();
                    settings.compact_now_playing = true;
                    apply_settings(&app, settings, false);
                    ui.set_media_available(true);
                    ui.set_playing(true);
                    ui.invoke_media_updated();
                    check(
                        matches!(
                            app.borrow().presentation.as_ref().map(|p| &p.content),
                            Some(Content::CompactMedia)
                        ) && selected(&app, &codex()),
                        "playing media overrides compact Codex without changing remembered page",
                    );
                    ui.set_playing(false);
                    ui.invoke_media_updated();
                    check(
                        matches!(
                            app.borrow().presentation.as_ref().map(|p| &p.content),
                            Some(Content::CompactPluginPage { .. })
                        ) && selected(&app, &codex()),
                        "paused media immediately yields to remembered Codex page",
                    );
                    ui.set_media_available(false);
                    ui.invoke_media_updated();
                    ui.invoke_hover(true);
                }
                8 => {
                    check(
                        selected(&app, &codex())
                            && app.borrow().coordinator.panel_open()
                            && ui.get_plugin_mode(),
                        "hover reentry reopens remembered Codex page",
                    );
                    ui.invoke_toggle_pin();
                    ui.invoke_hover(false);
                    let mut settings = app.borrow().settings.clone();
                    settings.text_scale = 1.4;
                    apply_settings(&app, settings, false);
                }
                9 => {
                    check(
                        app.borrow().coordinator.pinned() && app.borrow().coordinator.panel_open(),
                        "pin keeps selected page expanded after pointer leaves",
                    );
                    check(
                        (ui.get_shape_width() - 480.).abs() < 1.
                            && (ui.get_text_scale() - 1.4).abs() < 0.001,
                        "140 percent text rerasterizes within independent panel geometry",
                    );
                    capture(&ui, "pages-codex-text140");
                    ui.invoke_toggle_pin();
                }
                10 => {
                    check(
                        !app.borrow().coordinator.panel_open() && selected(&app, &codex()),
                        "unpin outside resumes collapse and retains selection",
                    );
                    ui.invoke_hover(true);
                }
                11 => {
                    app.borrow_mut().settings.collapse_delay = 0;
                    pages::choose(&app, Page::Media);
                    pages::choose(&app, codex());
                    pages::choose(&app, Page::Timer);
                    ui.invoke_hover(false);
                }
                12 => {
                    check(
                        !app.borrow().coordinator.panel_open()
                            && selected(&app, &Page::Timer)
                            && !app.borrow().page_switch_timer.running(),
                        "rapid interrupted page switches cannot reopen after hover leave",
                    );
                    check(
                        finite(&app),
                        "interrupted geometry preserves finite spring position and velocity",
                    );
                    ui.invoke_hover(true);
                    pages::choose(&app, Page::Media);
                    pages::open(&app, codex());
                }
                13 => {
                    check(
                        selected(&app, &codex()) && ui.get_plugin_mode(),
                        "explicit reopen cancels an older pending page switch",
                    );
                    let mut settings = app.borrow().settings.clone();
                    settings.animations = false;
                    settings.text_scale = 1.;
                    apply_settings(&app, settings, false);
                    pages::choose(&app, Page::Home);
                    pages::choose(&app, Page::Media);
                    pages::choose(&app, codex());
                    check(
                        selected(&app, &codex())
                            && !app.borrow().page_switch_timer.running()
                            && finite(&app),
                        "animations disabled commits page changes synchronously without invalid geometry",
                    );
                }
                14 => {
                    let host = app.borrow().plugin_host.clone().unwrap();
                    check(
                        host.set_enabled(CODEX, false).is_ok(),
                        "disable only the isolated fixture plugin",
                    );
                    pages::reconcile(&app);
                    check(
                        selected(&app, &Page::Home) && ui.get_page_count() == 4,
                        "disabled selected plugin falls back to Home and leaves carousel",
                    );
                }
                15 => {
                    ui.invoke_hover(false);
                    {
                        let mut a = app.borrow_mut();
                        let now = a.now();
                        a.coordinator.close_hover(now);
                    }
                    sync_presentation(&app);
                    check(finite(&app), "all final geometry remains finite");
                    frames.set(app.borrow().frames);
                }
                _ => {
                    let a = app.borrow();
                    check(
                        !a.coordinator.panel_open()
                            && !a.timer.running()
                            && !a.page_switch_timer.running()
                            && !a.media_clock.running()
                            && !a.deadline.running()
                            && a.frames == frames.get(),
                        "final idle stops animation media and page timers",
                    );
                    drop(a);
                    record("pages verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}

fn next_page_picker_guard(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    let before = app.borrow().settings.selected_page.clone();
    let result = send_wheel(app, (f64::from(ui.get_canvas_width()) / 2., 16.));
    check(
        matches!(result, slint::winit_030::EventResult::Propagate)
            && app.borrow().settings.selected_page == before,
        "open player picker protects its wheel interactions",
    );
}
