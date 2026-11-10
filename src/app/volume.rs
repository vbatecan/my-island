//! System-volume UI binding; the service owns Core Audio work.
use super::*;

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    {
        let app = app.clone();
        ui.on_open_volume(move || {
            pages::open(&app, crate::navigation::Page::Volume);
        });
    }
    {
        let weak = ui.as_weak();
        match VolumeService::new(move |event| {
            let _ = weak.upgrade_in_event_loop(move |ui| {
                apply_snapshot(&ui, event.snapshot);
                ui.invoke_volume_updated(event.origin == VolumeOrigin::External);
            });
        }) {
            Ok(service) => {
                if let Ok(v) = service.snapshot() {
                    apply_snapshot(ui, v);
                }
                app.borrow_mut().volume = Some(Rc::new(service));
            }
            Err(e) => ui.set_service_status(e.into()),
        }
    }
    {
        let app = app.clone();
        ui.on_volume_change(move |v| {
            command(&app, |service| service.set_volume(v));
        });
    }
    {
        let app = app.clone();
        ui.on_toggle_mute(move || {
            command(&app, VolumeService::toggle_mute);
        });
    }
    {
        let app = app.clone();
        ui.on_volume_updated(move |external| {
            if external && app.borrow().settings.auto_volume {
                feedback(&app);
            } else {
                sync_presentation(&app);
            }
        });
    }
}

fn apply_snapshot(ui: &IslandWindow, value: crate::services::volume::VolumeSnapshot) {
    ui.set_volume(value.percent);
    ui.set_volume_muted(value.muted);
    ui.set_volume_available(value.available);
}

fn command(app: &Rc<RefCell<App>>, action: impl FnOnce(&VolumeService) -> Result<(), String>) {
    let service = app.borrow().volume.clone();
    if let Some(service) = service
        && let Err(error) = action(&service)
    {
        status(app, &error);
    }
}

pub(super) fn feedback(app: &Rc<RefCell<App>>) {
    let mut a = app.borrow_mut();
    let now = a.now();
    a.coordinator.volume(now);
    if a.visibility.covered && !a.visibility.locked {
        a.visibility.until = Some(now + 3000);
        a.visibility.preview = Some(Content::Volume);
    }
    drop(a);
    sync_presentation(app);
}
