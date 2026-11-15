//! Deterministic material/layout snapshots after exercising actual live capture setup.
use super::*;

#[path = "live_material_verification.rs"]
mod live;

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    if verification::has_arg("--verify-live-material") {
        live::setup(app);
        return;
    }
    for (step, at) in [700, 1500, 2400, 3300, 4200, 5100, 6000, 6900, 7600, 8300]
        .into_iter()
        .enumerate()
    {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            if (1..=7).contains(&step)
                && let Err(e) =
                    crate::verification::snapshot(&ui, &format!("liquid-material-{step}"))
            {
                record(&format!("liquid FAILED {e}"));
            }
            if step == 7 {
                let mut settings = app.borrow().settings.clone();
                settings.appearance = Appearance::Solid;
                apply_settings(&app, settings, false);
                return;
            }
            if step == 8 {
                let a = app.borrow();
                let glass = a.glass.borrow();
                let cleared = a.capture.is_none()
                    && glass.backdrop.is_none()
                    && glass.liquid_backdrop.is_none()
                    && ui.get_glass_texture().size() == Default::default()
                    && !ui.get_has_glass_texture()
                    && !ui.get_gpu_material();
                let released = std::fs::read_to_string("verification.log").is_ok_and(|log| {
                    log.lines()
                        .any(|line| line.starts_with("liquid_gpu released "))
                });
                record(if cleared && released {
                    "liquid PASS Solid releases glass textures and backdrops"
                } else {
                    "liquid FAILED Solid retained glass resources"
                });
                drop(glass);
                drop(a);
            }
            if step == 9 {
                record(if ui.get_has_glass_texture() {
                    "liquid PASS optics recreated after returning from Solid"
                } else {
                    "liquid FAILED optics did not return after Solid"
                });
                record("liquid verification complete");
                let _ = slint::quit_event_loop();
                return;
            }
            if matches!(step, 1 | 2 | 4)
                && let Ok(buffer) = ui.window().take_snapshot()
            {
                let scale = buffer.width() as f32 / ui.get_canvas_width();
                let x = ((ui.get_canvas_width() + ui.get_shape_width()) * 0.5 - 14.) * scale;
                let y = ui.get_shape_height() * 0.55 * scale;
                let p = buffer.as_slice()[(y as u32 * buffer.width() + x as u32) as usize];
                let valid = if step == 1 {
                    (70..150).contains(&p.r)
                        && (70..150).contains(&p.g)
                        && (70..155).contains(&p.b)
                        && ui.get_liquid_dark_backdrop()
                } else if step == 2 {
                    p.r < 35 && p.g < 40 && p.b < 50
                } else {
                    (65..100).contains(&p.r) && p.g < 25 && (30..60).contains(&p.b)
                };
                record(&format!(
                    "liquid {} black tint and backdrop hue {p:?}",
                    if valid { "PASS" } else { "FAILED" }
                ));
            }
            let mut a = app.borrow_mut();
            if step == 0 {
                record(&format!(
                    "liquid capture worker scheduled={}",
                    a.capture.is_some()
                ));
                a.capture.take();
                a.capture_generation.fetch_add(1, Ordering::AcqRel);
            }
            a.settings.appearance = Appearance::LiquidGlass;
            a.settings.form_factor = if step == 3 {
                crate::shape::FormFactor::Island
            } else {
                crate::shape::FormFactor::Notch
            };
            a.settings.text_scale = if step == 2 { 1.4 } else { 1. };
            let width = a.canvas_size.0.ceil() as u32;
            let height = a.canvas_size.1.ceil() as u32;
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                for x in 0..width {
                    let color = match step {
                        0 => [245, 245, 250, 255],
                        1 => [20, 25, 35, 255],
                        3 => [180, 25, 95, 255],
                        _ if (x / 24 + y / 24) % 2 == 0 => [170, 210, 230, 255],
                        _ => [55, 90, 125, 255],
                    };
                    pixels.extend_from_slice(&color);
                }
            }
            a.glass.borrow_mut().set_liquid_backdrop(Backdrop {
                width,
                height,
                pixels,
            });
            a.retarget_shape();
            a.render();
            drop(a);
            if step == 2 {
                media::apply_media(
                    &ui,
                    &MediaSnapshot {
                        available: true,
                        can_play_pause: true,
                        title: "Liquid Glass".into(),
                        artist: "Material preview".into(),
                        duration_seconds: 180.,
                        position_seconds: 75.,
                        ..Default::default()
                    },
                    false,
                );
            }
            transition(
                &app,
                if step == 4 {
                    IslandState::Idle
                } else if step == 5 {
                    IslandState::Timer
                } else if step == 6 {
                    IslandState::Expanded
                } else if step == 2 {
                    IslandState::Media
                } else {
                    IslandState::Battery
                },
            );
        });
    }
}
