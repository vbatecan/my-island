//! Real battery handler regression; no injected input or manual animation triggers.
use super::*;
fn check(ok: bool, message: &str) {
    record(&format!(
        "illustration {} {message}",
        if ok { "PASS" } else { "FAILED" }
    ));
}
fn capture(ui: &IslandWindow, name: &str) {
    if let Err(e) = crate::verification::snapshot(ui, name) {
        record(&format!("illustration FAILED {e}"));
    }
}
pub(super) fn setup(app: &Rc<RefCell<App>>) {
    for (step, at) in [
        700, 1000, 2300, 2900, 3100, 3400, 4200, 4500, 4600, 4900, 5300,
    ]
    .into_iter()
    .enumerate()
    {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            let unplugged = crate::services::battery::BatterySnapshot::from_raw(0, 1, 100);
            let plugged = crate::services::battery::BatterySnapshot::from_raw(1, 1, 100);
            match step {
                0 => {
                    transition(&app, IslandState::Battery);
                    update_battery(&app, unplugged);
                    update_battery(&app, plugged);
                }
                1 => {
                    check(
                        ui.get_battery_power_connected() && ui.get_charge_intensity() > 0.,
                        "full battery connection starts effect",
                    );
                    capture(&ui, "illustration-charge-active");
                    let mut a = app.borrow_mut();
                    let now = a.now();
                    a.coordinator.set_hover(true, now);
                    a.coordinator.toggle_pin(now);
                }
                2 => {
                    update_battery(&app, plugged);
                    capture(&ui, "illustration-charge-fading");
                }
                3 => {
                    check(
                        ui.get_charge_intensity() == 0.
                            && !app.borrow().battery_effect_frames.running(),
                        "plugged-in effect expires despite hover pin and duplicate snapshot",
                    );
                    capture(&ui, "illustration-charge-expired");
                }
                4 => update_battery(&app, unplugged),
                5 => {
                    check(
                        !ui.get_charge_connecting() && ui.get_charge_intensity() > 0.,
                        "unplug exit sweep starts",
                    );
                    capture(&ui, "illustration-unplug-active");
                }
                6 => {
                    check(
                        ui.get_charge_intensity() == 0.
                            && !app.borrow().battery_effect_frames.running(),
                        "unplug sweep expires",
                    );
                }
                7 => update_battery(&app, plugged),
                8 => {
                    app.borrow_mut().visibility.locked = true;
                    sync_presentation(&app);
                    check(ui.get_charge_intensity() == 0., "locking cancels effect");
                }
                9 => {
                    app.borrow_mut().visibility.locked = false;
                    sync_presentation(&app);
                    check(
                        ui.get_charge_intensity() == 0.,
                        "unlock does not replay effect",
                    );
                }
                _ => {
                    record("illustration verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}
