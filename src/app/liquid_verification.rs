//! Deterministic material/layout snapshots after exercising actual live capture setup.
use super::*;
pub(super) fn setup(app: &Rc<RefCell<App>>) {
    for (step, at) in [700, 1500, 2400, 3300, 4200, 5100, 6000, 6900]
        .into_iter()
        .enumerate()
    {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            if step > 0
                && let Err(e) =
                    crate::verification::snapshot(&ui, &format!("liquid-material-{step}"))
            {
                record(&format!("liquid FAILED {e}"));
            }
            if step == 7 {
                record("liquid verification complete");
                let _ = slint::quit_event_loop();
                return;
            }
            if matches!(step, 2 | 4)
                && let Ok(buffer) = ui.window().take_snapshot()
            {
                let scale = buffer.width() as f32 / ui.get_canvas_width();
                let x = ((ui.get_canvas_width() + ui.get_shape_width()) * 0.5 - 14.) * scale;
                let y = ui.get_shape_height() * 0.55 * scale;
                let p = buffer.as_slice()[(y as u32 * buffer.width() + x as u32) as usize];
                let valid = if step == 2 {
                    p.r < 35 && p.g < 40 && p.b < 50
                } else {
                    p.r > 150 && p.g < 45 && (70..110).contains(&p.b)
                };
                record(&format!(
                    "liquid {} backdrop hue retained {p:?}",
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
