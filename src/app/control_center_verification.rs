//! Exercise the owned Slint window and its normal carousel/control handlers.
use super::*;
use crate::{
    ControlAction, DeviceEntry, LiquidSurfaces, RadioControl, navigation::Page, shape::FormFactor,
};
use slint::{
    Model,
    platform::{Key, PointerEventButton, WindowEvent},
};

fn check(ok: bool, expectation: &str) {
    record(&format!(
        "control center {} {expectation}",
        if ok { "PASS" } else { "FAILED" }
    ));
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    if verification::has_arg("--verify-control-contrast") {
        contrast_snapshots(app);
        return;
    }
    let opening = Rc::new(std::cell::Cell::new(false));
    let initial_press = Rc::new(RefCell::new(None));
    let sample = Rc::new(slint::Timer::default());
    let sampled = opening.clone();
    let weak = Rc::downgrade(app);
    sample.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(20),
        move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            if ui.get_control_center_mode()
                && (ui.get_shape_height() - ui.get_control_center_height()).abs() > 0.25
                && ui.get_surface_glass_visible()
            {
                sampled.set(true);
            }
        },
    );
    for (step, delay) in [
        (-3, 100),
        (0, 900),
        (-1, 2450),
        (-2, 2580),
        (1, 3300),
        (9, 3900),
        (10, 4900),
        (11, 5250),
        (12, 6000),
        (13, 6800),
        (14, 7550),
        (20, 8150),
        (21, 8650),
        (2, 9000),
        (3, 9800),
        (4, 10650),
        (5, 11500),
        (6, 12300),
        (60, 12900),
        (7, 14050),
        (70, 14400),
        (71, 15100),
        (8, 15750),
    ] {
        let weak = Rc::downgrade(app);
        let opening = opening.clone();
        let initial_press = initial_press.clone();
        let sample = sample.clone();
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            if step == 1 {
                sample.stop();
            }
            run(&app, &ui, step, opening.get(), &initial_press);
        });
    }
}

fn contrast_snapshots(app: &Rc<RefCell<App>>) {
    let cases = [
        ("dark", false, false),
        ("light", true, false),
        ("pattern", false, true),
        ("narrow-dark", false, false),
        ("narrow-light", true, false),
        ("narrow-pattern", true, true),
    ];
    for (index, (name, light, patterned)) in cases.into_iter().enumerate() {
        let weak = Rc::downgrade(app);
        let delay = Duration::from_millis(700 + 2400 * index as u64);
        slint::Timer::single_shot(delay, move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            if index == 0 {
                run(&app, &ui, 0, false, &RefCell::new(None));
                ui.set_control_brightness(8);
                ui.set_volume(92);
            }
            if index == 3 {
                let mut settings = app.borrow().settings.clone();
                settings.text_scale = 1.4;
                app.borrow_mut().control_center.verification_width = Some(384.);
                apply_settings(&app, settings, false);
                app.borrow_mut().retarget_shape();
                wake(&app);
            }
            publish_backdrop(&app, light, patterned);
            slint::Timer::single_shot(Duration::from_millis(2050), move || {
                check(
                    ui.get_control_center_mode()
                        && ui.get_surface_glass_visible()
                        && ui.get_reveal() > 0.98
                        && ui.get_liquid_dark_backdrop() != light
                        && ui.get_control_center_wide() == (index < 3),
                    &format!("contrast fixture settles on {name}"),
                );
                capture(&ui, &format!("control-contrast-{name}"));
                if index == cases.len() - 1 {
                    let mut settings = app.borrow().settings.clone();
                    settings.appearance = Appearance::Solid;
                    apply_settings(&app, settings, false);
                    slint::Timer::single_shot(Duration::from_millis(450), move || {
                        check(
                            ui.get_control_center_mode() && !ui.get_surface_glass_visible(),
                            "solid Control Center keeps its controls without glass",
                        );
                        capture(&ui, "control-contrast-solid");
                        record("control contrast verification complete");
                        let _ = slint::quit_event_loop();
                    });
                }
            });
        });
    }
}

fn run(
    app: &Rc<RefCell<App>>,
    ui: &IslandWindow,
    step: i32,
    opening: bool,
    initial_press: &RefCell<Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>>,
) {
    if verification::has_arg("--verify-live-material") {
        if step == 0 {
            ui.invoke_toggle();
        } else if step == 1 {
            check(
                ui.get_has_glass_texture() && ui.get_surface_glass_visible(),
                "live Windows capture feeds the shell and control lenses",
            );
            record("control center verification complete");
            let _ = slint::quit_event_loop();
        }
        return;
    }
    let devices = verification::has_arg("--verify-control-center-devices");
    if devices {
        if step == 0 {
            ui.invoke_toggle();
        } else if step == 1 {
            record(&format!(
                "control center portable device reads: brightness={} checked={} Wi-Fi={} Bluetooth={} hotspot={}",
                ui.get_control_brightness(),
                ui.get_control_brightness_checked(),
                ui.get_control_wifi().known,
                ui.get_control_bluetooth().known,
                ui.get_control_hotspot_detail(),
            ));
            check(
                ui.get_control_brightness_checked(),
                "brightness provider replied",
            );
            check(
                !ui.get_control_hotspot_detail().is_empty(),
                "hotspot provider replied",
            );
            ui.invoke_control_action(ControlAction::WifiDevices);
            audit_native_devices(app, 4, Instant::now());
        }
        return;
    }
    match step {
        -3 => backdrop(app, false),
        0 => {
            let audio = app.borrow_mut().volume.take();
            drop(audio);
            let mut settings = app.borrow().settings.clone();
            settings.appearance = Appearance::LiquidGlass;
            settings.opacity = 0.15;
            settings.form_factor = FormFactor::Notch;
            settings.selected_page = Some(Page::Media);
            settings.animations = true;
            settings.animation_speed = 1.;
            settings.follow_reduced_motion = false;
            settings.collapse_on_leave = false;
            settings.text_scale = 1.;
            settings.notifications_enabled = true;
            settings.notifications_paused = false;
            app.borrow_mut().notification_access = AccessStatus::Allowed;
            apply_settings(app, settings, false);
            ui.invoke_hover(true);
            ui.invoke_toggle();
            update_battery(
                app,
                crate::services::battery::BatterySnapshot::from_raw(1, 8, 84),
            );
            ui.set_volume_available(true);
            ui.set_volume(42);
            ui.set_control_brightness(72);
            ui.set_control_brightness_checked(true);
            ui.set_control_wifi(RadioControl {
                known: true,
                on: true,
                switchable: true,
                busy: false,
            });
            ui.set_control_bluetooth(RadioControl {
                known: true,
                on: false,
                switchable: true,
                busy: false,
            });
            ui.set_control_hotspot(RadioControl {
                known: true,
                on: false,
                switchable: true,
                busy: false,
            });
            ui.set_control_hotspot_detail("Off".into());
        }
        -1 => {
            *initial_press.borrow_mut() = ui.get_surface_glass_texture().to_rgba8();
            press(ui, 17., 207., true);
        }
        -2 => {
            let after = ui.get_surface_glass_texture().to_rgba8();
            check(
                initial_press
                    .borrow()
                    .as_ref()
                    .zip(after.as_ref())
                    .is_some_and(|(a, b)| a.as_slice() != b.as_slice()),
                "the Liquid Rust scene changes during a held speaker-button press",
            );
            capture(ui, "control-center-pressed");
            press(ui, 17., 207., false);
        }
        1 => {
            check(
                opening,
                "glass controls render before the opening spring settles",
            );
            check(
                ui.get_control_center_mode()
                    && ui.get_page_index() == 0
                    && app.borrow().settings.selected_page == Some(Page::Media),
                "default expansion ignores the saved compact page",
            );
            check(
                ui.get_control_center_wide()
                    && ui.get_has_glass_texture()
                    && ui.get_liquid_dark_backdrop()
                    && (ui.get_shape_width() - 620.).abs() < 1.
                    && (ui.get_shape_height() - 264.).abs() < 1.,
                "wide notch fits its reclaimed space with dark-backdrop Liquid Rust",
            );
            check(
                refracted_controls(ui),
                "Liquid Rust paints the buttons, tiles, switches and faders",
            );
            check(
                rounded_faders(ui),
                "both slider fills have rounded end caps",
            );
            capture(ui, "control-center-dark");
            let width = control_width(ui);
            check(
                ui.get_next_page_label() == "Home",
                "the first next arrow names its destination",
            );
            move_pointer(ui, width - 15., 17.);
            capture(ui, "control-center-next-page");
            let column = (width - 16.) / 3.;
            move_pointer(ui, width - column / 4., 65.);
            capture(ui, "control-center-sleep");
            click(ui, 403., 207.);
            click(ui, 61., 167.);
            let [volume, brightness] = app.borrow().control_center.verified_levels;
            check(
                volume.is_some_and(|value| value > 75)
                    && brightness.is_some_and(|value| value < 25),
                "horizontal sliders send the correct level from pointer input",
            );
            let toggle = first_switch_x(ui);
            for (x, y, action) in [
                (toggle, 65., ControlAction::Wifi),
                (toggle, 117., ControlAction::Bluetooth),
                (width / 2., 117., ControlAction::Hotspot),
                (17., 207., ControlAction::Mute),
            ] {
                click(ui, x, y);
                check(
                    app.borrow().control_center.verified_action == Some(action),
                    &format!("pointer routes {action:?} directly"),
                );
            }
            click(ui, width / 2., 65.);
            check(
                app.borrow().settings.notifications_paused
                    && ui.get_control_previews_detail() == "Paused",
                "preview tile pauses Still Island notifications",
            );
            click(ui, width / 2., 65.);
            check(
                !app.borrow().settings.notifications_paused && ui.get_control_previews_on(),
                "preview tile resumes Still Island notifications",
            );
        }
        9 => {
            // Give the observer a full animation step after the overview snapshots.
            app.borrow_mut().settings.animation_speed = 0.5;
            click(ui, 105., 65.);
            check(
                app.borrow().control_center.verified_action == Some(ControlAction::WifiDevices)
                    && ui.get_control_menu() == 4,
                "Wi-Fi tile body opens its network list",
            );
            seed_devices(ui, "Network", 16);
            ui.invoke_hover(false);
            audit_device_morph(ui);
        }
        10 => {
            app.borrow_mut().settings.animation_speed = 1.;
            check(
                ui.get_control_menu() == 4
                    && ui.get_control_devices().row_count() == 16
                    && (ui.get_shape_height() - 404.).abs() < 1.
                    && surface_count(ui) > 1,
                "the Wi-Fi list stays open after pointer exit and renders Liquid Glass",
            );
            capture(ui, "control-center-wifi");
            click(ui, 105., 69.);
            check(
                app.borrow().control_center.verified_device.as_deref() == Some("Network-0"),
                "Wi-Fi row routes the selected network ID",
            );
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position: control_position(ui, 105., 120.),
                delta_x: 0.,
                delta_y: -210.,
            });
        }
        11 => {
            app.borrow_mut().control_center.verified_device = None;
            click(ui, 105., 79.);
            check(
                ui.get_page_index() == 0
                    && ui.get_control_menu() == 4
                    && app
                        .borrow()
                        .control_center
                        .verified_device
                        .as_deref()
                        .is_some_and(|id| id.starts_with("Network-") && id != "Network-0"),
                "scrolling the network list reaches later rows within Control Center",
            );
            click(ui, 17., 18.);
            check(
                ui.get_control_menu() == 0 && ui.get_control_devices().row_count() == 0,
                "Back returns to the compact controls and clears the Wi-Fi list",
            );
        }
        12 => {
            click(ui, 105., 117.);
            check(
                app.borrow().control_center.verified_action
                    == Some(ControlAction::BluetoothDevices)
                    && ui.get_control_menu() == 5,
                "Bluetooth tile body opens its device list while Bluetooth is off",
            );
            seed_devices(ui, "Headphones", 3);
        }
        13 => {
            check(
                ui.get_control_menu() == 5
                    && ui.get_control_devices().row_count() == 3
                    && surface_count(ui) >= 4,
                "Bluetooth device rows render with Liquid Glass",
            );
            capture(ui, "control-center-bluetooth");
            click(ui, 105., 69.);
            check(
                app.borrow().control_center.verified_device.as_deref() == Some("Headphones-0"),
                "Bluetooth row routes the selected device ID",
            );
            ui.invoke_focus_panel();
            let text: slint::SharedString = Key::Escape.into();
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
            check(
                ui.get_control_menu() == 0 && ui.get_control_center_mode(),
                "Escape returns from Bluetooth without dismissing Control Center",
            );
        }
        14 => {
            check(
                (ui.get_shape_height() - 264.).abs() < 1.,
                "returning from a device list restores the compact panel height",
            );
            click(ui, control_width(ui) - 28., 117.);
            check(ui.get_control_menu() == 1, "pointer opens the power menu");
        }
        20 => {
            check(
                surface_count(ui) == 5,
                &format!(
                    "the power buttons replace the previous surface geometry ({} surfaces)",
                    surface_count(ui)
                ),
            );
            click(ui, 60., 175.);
            check(
                ui.get_control_menu() == 2 && app.borrow().control_center.verified_power.is_none(),
                "restart enters confirmation without requesting a power action",
            );
        }
        21 => {
            check(
                surface_count(ui) == 3,
                &format!(
                    "confirmation buttons use Liquid Rust ({} surfaces)",
                    surface_count(ui)
                ),
            );
            capture(ui, "control-center-confirm");
            click(ui, 170., 153.);
            check(
                app.borrow().control_center.verified_power
                    == Some(platform::power::Action::Restart),
                "the confirmation follows the pointer handler to the fixture power backend",
            );
            ui.invoke_collapse();
            app.borrow_mut().control_center.verified_power = None;
            ui.invoke_control_action(ControlAction::Confirm);
            check(
                app.borrow().control_center.verified_power.is_none(),
                "dismissal discards the confirmation",
            );
        }
        2 => {
            ui.invoke_hover(true);
            ui.invoke_toggle();
        }
        3 => {
            check(
                ui.get_page_index() == 0 && ui.get_control_center_mode(),
                "reopening starts at slot one",
            );
            ui.invoke_step_page(1);
            check_slide(app, -1., "next page exits to the left");
        }
        4 => {
            check(ui.get_page_label() == "Home", "next page reaches Home");
            check(
                surface_count(ui) >= 4,
                "Home cards use the same Liquid Rust surface renderer",
            );
            capture(ui, "liquid-home");
            ui.invoke_step_page(-1);
            check_slide(app, 1., "previous page exits to the right");
        }
        5 => {
            check(
                ui.get_page_index() == 0,
                "previous page returns to Control Center",
            );
            ui.invoke_step_page(1);
            ui.invoke_step_page(1);
            ui.invoke_collapse();
        }
        6 => {
            check(
                !app.borrow().coordinator.panel_open() && app.borrow().pending_page.is_none(),
                "rapid switching cannot reopen a dismissed panel",
            );
            let mut settings = app.borrow().settings.clone();
            settings.form_factor = FormFactor::Island;
            settings.text_scale = 1.4;
            settings.follow_reduced_motion = true;
            {
                let mut a = app.borrow_mut();
                a.reduced_motion = true;
                a.control_center.verification_width = Some(384.);
            }
            apply_settings(app, settings, false);
            ui.invoke_hover(true);
            ui.invoke_toggle();
            ui.invoke_step_page(-1);
            check(
                ui.get_page_label() == "Timer" && ui.get_page_shift() == 0.,
                "reduced motion wraps backward without displacement",
            );
        }
        60 => {
            check(
                surface_count(ui) == 6,
                "the idle Timer refracts its secondary controls",
            );
            capture(ui, "liquid-timer");
            ui.invoke_open_volume();
            check(
                ui.get_page_index() == 0 && ui.get_control_center_mode(),
                "the volume shortcut resolves to Control Center",
            );
            backdrop(app, true);
            update_battery(app, crate::services::battery::BatterySnapshot::default());
            ui.set_volume_available(false);
            ui.set_control_brightness_checked(true);
            ui.set_control_brightness(-1);
            ui.set_control_wifi(RadioControl {
                known: true,
                on: true,
                switchable: false,
                busy: false,
            });
            ui.set_control_bluetooth(RadioControl::default());
            ui.set_control_hotspot_detail("No network".into());
        }
        7 => {
            check(
                !ui.get_control_center_wide()
                    && ui.get_has_glass_texture()
                    && !ui.get_liquid_dark_backdrop()
                    && ui.get_text_scale() == 1.4
                    && (ui.get_shape_height() - 370.).abs() < 1.,
                "narrow floating island fits its reclaimed space with enlarged text",
            );
            check(
                refracted_controls(ui),
                "lenses match the stacked control layout",
            );
            check(
                surface_count(ui) == 18,
                &format!(
                    "only the stacked control surfaces remain after switching from Timer ({} surfaces)",
                    surface_count(ui),
                ),
            );
            app.borrow_mut().control_center.verified_action = None;
            let toggle = first_switch_x(ui);
            click(ui, toggle, 65.);
            click(ui, toggle, 117.);
            click(ui, 260., 117.);
            check(
                app.borrow().control_center.verified_action.is_none()
                    && ui.get_control_center_mode(),
                "blocked and unavailable switches leave the panel open without dispatching actions",
            );
            capture(ui, "control-center-narrow");
            click(ui, 85., 65.);
            check(
                ui.get_control_menu() == 4
                    && ui.get_control_device_progress() == 1.
                    && (ui.get_shape_height() - 404.).abs() < 1.,
                "reduced motion opens the blocked Wi-Fi list without a height transition",
            );
            ui.invoke_control_action(ControlAction::Back);
        }
        70 => {
            launcher::toggle_for_local_verification(app);
            ui.set_launcher_text("18*4".into());
            ui.invoke_launcher_query(ui.get_launcher_text());
        }
        71 => {
            check(
                ui.get_app_launcher_mode() && ui.get_answer_visible() && surface_count(ui) >= 2,
                "the launcher search and answer surfaces use Liquid Rust",
            );
            capture(ui, "liquid-launcher");
        }
        8 => {
            let mut settings = app.borrow().settings.clone();
            settings.appearance = Appearance::Solid;
            apply_settings(app, settings, false);
            check(
                !ui.get_surface_glass_visible() && surface_count(ui) == 0,
                "Solid releases the Liquid Rust control texture",
            );
            record("control center verification complete");
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
}

fn audit_native_devices(app: &Rc<RefCell<App>>, menu: i32, started: Instant) {
    let Some(ui) = app.borrow().ui.upgrade() else {
        return;
    };
    if ui.get_control_menu() == menu
        && ui.get_control_devices_busy()
        && started.elapsed() < Duration::from_secs(15)
    {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(125), move || {
            if let Some(app) = weak.upgrade() {
                audit_native_devices(&app, menu, started);
            }
        });
        return;
    }
    let rows = ui.get_control_devices().row_count();
    let label = if menu == 4 { "Wi-Fi" } else { "Bluetooth" };
    check(
        ui.get_control_menu() == menu
            && !ui.get_control_devices_busy()
            && (rows > 0 || !ui.get_control_device_detail().is_empty()),
        &format!("{label} discovery replied with {rows} entries"),
    );
    if menu == 4 {
        let signals: Vec<_> = ui
            .get_control_devices()
            .iter()
            .map(|row| row.signal)
            .collect();
        check(
            signals.iter().all(|signal| (0..=100).contains(signal))
                && signals.windows(2).all(|pair| pair[0] >= pair[1]),
            "Wi-Fi results are sorted by descending signal strength",
        );
        ui.invoke_control_action(ControlAction::Back);
        ui.invoke_control_action(ControlAction::BluetoothDevices);
        audit_native_devices(app, 5, Instant::now());
    } else {
        record("control center device verification complete");
        let _ = slint::quit_event_loop();
    }
}

fn seed_devices(ui: &IslandWindow, prefix: &str, count: usize) {
    let rows = (0..count)
        .map(|index| DeviceEntry {
            id: format!("{prefix}-{index}").into(),
            label: format!("{prefix} {}", index + 1).into(),
            detail: match (prefix, index) {
                (_, 1) => "Connected",
                ("Network", 2) => "Open network",
                ("Network", 3) => "Setup required",
                ("Network", _) => "",
                _ => "Paired",
            }
            .into(),
            connected: index == 1,
            enabled: index != 1 && (prefix != "Network" || index != 3),
            signal: if prefix == "Network" {
                (100 - 7 * index as i32).max(0)
            } else {
                -1
            },
        })
        .collect::<Vec<_>>();
    ui.set_control_devices(slint::ModelRc::new(slint::VecModel::from(rows)));
    ui.set_control_devices_busy(false);
    ui.set_control_device_detail("".into());
}

fn audit_device_morph(ui: &IslandWindow) {
    let window = ui.as_weak();
    let intermediate = Rc::new(std::cell::Cell::new(false));
    for delay in [8, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512] {
        let window = window.clone();
        let intermediate = intermediate.clone();
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
            if let Some(ui) = window.upgrade()
                && ui.get_control_menu() == 4
                && (264.25..403.75).contains(&ui.get_shape_height())
                && (0.0..1.0).contains(&ui.get_control_device_progress())
            {
                intermediate.set(true);
            }
            if delay == 512 {
                check(
                    intermediate.get(),
                    "the device list morphs through an intermediate height",
                );
            }
        });
    }
}

fn rounded_faders(ui: &IslandWindow) -> bool {
    let Ok(frame) = ui.window().take_snapshot() else {
        return false;
    };
    let scale = frame.width() as f32 / ui.get_canvas_width();
    let sample = |x: f32, y: f32| -> Option<i32> {
        let x = ((ui.get_control_origin_x() + x) * scale).round() as usize;
        let y = ((ui.get_control_origin_y() + y) * scale).round() as usize;
        let pixel = frame.as_slice().get(y * frame.width() as usize + x)?;
        Some((i32::from(pixel.r) + i32::from(pixel.g) + i32::from(pixel.b)) / 3)
    };
    let rail = control_width(ui) - 56.;
    [(0.72, 150.), (0.42, 190.)]
        .into_iter()
        .all(|(level, top)| {
            // Sample inside the center of the end cap and outside its rounded corners.
            let x = 33. + (rail - 33.) * level - 4.;
            sample(x, top + 17.)
                .zip(sample(x, top + 2.))
                .zip(sample(x, top + 32.))
                .is_some_and(|((center, upper), lower)| center > upper + 15 && center > lower + 15)
        })
}

fn refracted_controls(ui: &IslandWindow) -> bool {
    let shell = ui.get_glass_texture().to_rgba8();
    let composite = ui.get_surface_glass_texture().to_rgba8();
    let (Some(shell), Some(composite)) = (shell, composite) else {
        return false;
    };
    if surface_count(ui) < 18
        || shell.width() != composite.width()
        || shell.height() != composite.height()
    {
        return false;
    }
    let silhouette_width = ui.get_shape_width() + 2. * ui.get_shoulder();
    let left = (ui.get_canvas_width() - silhouette_width) / 2.;
    let scale_x = shell.width() as f32 / silhouette_width;
    let scale_y = shell.height() as f32 / ui.get_shape_height();
    let width = shell.width() as usize;
    let differs = |index| {
        let a: &slint::Rgba8Pixel = &shell.as_slice()[index];
        let b: &slint::Rgba8Pixel = &composite.as_slice()[index];
        a.r.abs_diff(b.r)
            .max(a.g.abs_diff(b.g))
            .max(a.b.abs_diff(b.b))
            >= 2
    };
    // Check the navigation button above the larger tiles, and both fader tracks.
    // Compare the GPU textures before Slint draws any labels and tints.
    !differs(width * 4 + width / 2)
        && [
            0.,
            if ui.get_control_center_wide() {
                150.
            } else {
                256.
            },
            if ui.get_control_center_wide() {
                190.
            } else {
                296.
            },
        ]
        .into_iter()
        .all(|top| {
            let x = ui.get_control_origin_x() - left;
            let y = ui.get_control_origin_y() + top;
            let x0 = (x * scale_x).max(0.) as usize;
            let x1 = ((x + 31.) * scale_x).min(width as f32) as usize;
            let y0 = (y * scale_y).max(0.) as usize;
            let y1 = ((y + 31.) * scale_y).min(shell.height() as f32) as usize;
            (y0..y1)
                .flat_map(|row| (x0..x1).map(move |col| row * width + col))
                .filter(|index| differs(*index))
                .take(12)
                .count()
                == 12
        })
}

fn surface_count(ui: &IslandWindow) -> i32 {
    ui.global::<LiquidSurfaces>().get_rendered_count()
}

fn check_slide(app: &Rc<RefCell<App>>, sign: f32, label: &'static str) {
    let ui = app.borrow().ui.clone();
    let moved = Rc::new(std::cell::Cell::new(false));
    let glass = Rc::new(std::cell::Cell::new(false));
    for delay in [24, 48, 72] {
        let ui = ui.clone();
        let moved = moved.clone();
        let glass = glass.clone();
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
            if let Some(ui) = ui.upgrade()
                && ui.get_page_shift() * sign > 0.
            {
                moved.set(true);
                if ui.get_surface_glass_visible() {
                    glass.set(true);
                }
            }
            if delay == 72 {
                check(moved.get(), label);
                check(
                    glass.get(),
                    "glass surfaces render during the Liquid Rust page slide",
                );
            }
        });
    }
}

fn click(ui: &IslandWindow, x: f32, y: f32) {
    press(ui, x, y, true);
    press(ui, x, y, false);
}

fn move_pointer(ui: &IslandWindow, x: f32, y: f32) {
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: control_position(ui, x, y),
    });
}

fn press(ui: &IslandWindow, x: f32, y: f32, down: bool) {
    let position = control_position(ui, x, y);
    move_pointer(ui, x, y);
    ui.window().dispatch_event(if down {
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        }
    } else {
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        }
    });
}

fn control_position(ui: &IslandWindow, x: f32, y: f32) -> slint::LogicalPosition {
    let scale = ui.get_ui_scale();
    slint::LogicalPosition::new(
        (ui.get_control_origin_x() + x) * scale,
        (ui.get_control_origin_y() + y) * scale,
    )
}

fn control_width(ui: &IslandWindow) -> f32 {
    let inset = if ui.get_control_center_wide() {
        64.
    } else {
        48.
    };
    ui.get_shape_width() - inset
}

fn first_switch_x(ui: &IslandWindow) -> f32 {
    let columns = if ui.get_control_center_wide() { 3. } else { 2. };
    (control_width(ui) - (columns - 1.) * 8.) / columns - 23.
}

fn backdrop(app: &Rc<RefCell<App>>, light: bool) {
    publish_backdrop(app, light, true);
}

fn publish_backdrop(app: &Rc<RefCell<App>>, light: bool, patterned: bool) {
    let a = app.borrow();
    let width = a.canvas_size.0.ceil() as u32;
    let height = a.canvas_size.1.ceil() as u32;
    let base = if light {
        [245u8, 249, 253]
    } else {
        [24u8, 35, 52]
    };
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let stripe = (patterned && (x + y * 2) % 280 < 110) as u8 * 10;
            pixels.extend([base[0] - stripe, base[1] - stripe, base[2] - stripe, 255]);
        }
    }
    a.glass.borrow_mut().set_liquid_backdrop(Backdrop {
        width,
        height,
        pixels,
    });
    a.render();
}

fn capture(ui: &IslandWindow, name: &str) {
    if let Err(error) = crate::verification::snapshot(ui, name) {
        check(false, &format!("snapshot {name}: {error}"));
    }
}
