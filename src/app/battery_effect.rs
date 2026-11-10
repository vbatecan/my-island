//! Finite power-event feedback, independent of preview dwell and power state.
use super::*;
#[derive(Default)]
pub(super) struct Effect {
    pub generation: u64,
    pub active: Option<(bool, u64)>,
}
impl Effect {
    pub fn start(&mut self, connected: bool, now: u64) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.active = Some((connected, now));
        self.end().unwrap()
    }
    pub fn end(&self) -> Option<u64> {
        self.active
            .map(|(connected, start)| start.saturating_add(if connected { 2000 } else { 900 }))
    }
    pub fn cancel(&mut self) {
        self.active = None;
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn frame(&self, now: u64) -> (f32, f32, bool) {
        let Some((connected, start)) = self.active else {
            return (0., 0., false);
        };
        let elapsed = now.saturating_sub(start) as f32;
        let (fade_in, fade_out, end) = if connected {
            (150., 1400., 2000.)
        } else {
            (100., 400., 900.)
        };
        let intensity = (elapsed / fade_in)
            .min((end - elapsed) / (end - fade_out))
            .clamp(0., 1.);
        (intensity, elapsed / 4000., connected)
    }
}
pub(super) fn cancel(app: &Rc<RefCell<App>>) {
    let mut a = app.borrow_mut();
    a.battery_effect.cancel();
    a.battery_effect_frames.stop();
    a.battery_effect_expiry.stop();
    if let Some(ui) = a.ui.upgrade() {
        ui.set_charge_intensity(0.);
    }
}
pub(super) fn sync(app: &Rc<RefCell<App>>) {
    let a = app.borrow();
    if a.battery_effect.active.is_none() {
        return;
    }
    if a.hidden
        || a.visibility.hiding
        || a.visibility.locked
        || a.settings_visible
        || !a.battery.present
        || a.battery_effect.end().is_some_and(|end| a.now() >= end)
    {
        drop(a);
        cancel(app);
        return;
    }
    if !a.animations_enabled() {
        a.battery_effect_frames.stop();
    }
    let (intensity, phase, connected) = a.battery_effect.frame(a.now());
    if let Some(ui) = a.ui.upgrade() {
        ui.set_charge_intensity(if a.animations_enabled() {
            intensity
        } else {
            1.
        });
        ui.set_charge_phase(phase * a.settings.animation_speed as f32);
        ui.set_charge_connecting(connected);
    }
}
pub(super) fn start(app: &Rc<RefCell<App>>, connected: bool) {
    cancel(app);
    let mut a = app.borrow_mut();
    if a.hidden
        || a.visibility.hiding
        || a.visibility.locked
        || a.settings_visible
        || !a.battery.present
    {
        return;
    }
    let now = a.now();
    let end = a.battery_effect.start(connected, now);
    let generation = a.battery_effect.generation;
    let animated = a.animations_enabled();
    drop(a);
    sync(app);
    let weak = Rc::downgrade(app);
    app.borrow().battery_effect_expiry.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(end - now),
        move || {
            if let Some(app) = weak.upgrade() {
                let current = app.borrow().battery_effect.generation == generation;
                if current {
                    cancel(&app);
                }
            }
        },
    );
    if animated {
        let weak = Rc::downgrade(app);
        app.borrow().battery_effect_frames.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(34),
            move || {
                if let Some(app) = weak.upgrade() {
                    sync(&app);
                }
            },
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expires_while_power_stays_connected() {
        let mut e = Effect::default();
        e.start(true, 100);
        assert_eq!(e.end(), Some(2100));
        assert_eq!(e.frame(1600).0, 500. / 600.);
        assert_eq!(e.frame(2100).0, 0.);
        assert_eq!(e.frame(10000).0, 0.);
    }
    #[test]
    fn reconnect_replaces_exit_without_a_cooldown() {
        let mut e = Effect::default();
        e.start(true, 0);
        let old = e.generation;
        e.start(false, 100);
        assert_eq!(e.end(), Some(1000));
        assert_ne!(old, e.generation);
        e.start(true, 150);
        assert_eq!(e.end(), Some(2150));
        assert!(e.frame(400).2);
        e.cancel();
        assert_eq!(e.end(), None);
        assert_eq!(e.frame(500).0, 0.);
    }
}
