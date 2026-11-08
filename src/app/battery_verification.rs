//! Explicit synthetic battery GUI regression. Never changes Windows power state.
use super::*;
use crate::services::battery::{BatteryActivity, BatteryEventKind, BatterySnapshot};

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    // Leave time for the native window to attach before injecting a preview.
    for (step, delay) in [700, 1500, 2000, 2700, 3300, 4700, 6200]
        .into_iter()
        .enumerate()
    {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            let check = |condition: bool, label: &str| {
                record(&format!(
                    "battery {} {label}",
                    if condition { "PASS" } else { "FAILED" }
                ));
            };
            let capture = |name: &str| match crate::verification::snapshot(&ui, name) {
                Ok(path) => record(&format!("snapshot {}", path.display())),
                Err(error) => record(&format!("snapshot FAILED {error}")),
            };
            match step {
                0 => {
                    let snapshot = BatterySnapshot::from_raw(0, 2, 19);
                    update_battery(&app, snapshot);
                    let mut a = app.borrow_mut();
                    let now = a.now();
                    a.coordinator.battery(
                        BatteryActivity {
                            snapshot,
                            kind: BatteryEventKind::Low,
                        },
                        now,
                    );
                    drop(a);
                    sync_presentation(&app);
                }
                1 => {
                    check(
                        matches!(
                            app.borrow().presentation.as_ref().map(|p| &p.content),
                            Some(Content::CompactBattery(_))
                        ),
                        "low battery stays compact",
                    );
                    capture("battery-compact");
                }
                2 => transition(&app, IslandState::Battery),
                3 => {
                    check(app.borrow().state == IslandState::Battery, "details expand");
                    capture("battery-details");
                }
                4 => transition(&app, IslandState::Idle),
                5 => {
                    check(
                        app.borrow().state == IslandState::Idle,
                        "preview dismisses and restores idle",
                    );
                    capture("battery-settled");
                }
                _ => {
                    check(
                        app.borrow().state == IslandState::Idle,
                        "idle remains stable",
                    );
                    check(
                        !app.borrow().timer.running() && !app.borrow().deadline.running(),
                        "animation and activity scheduling stop",
                    );
                    record("battery verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}
