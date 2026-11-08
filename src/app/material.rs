//! Backdrop lifecycle and event delivery for Eco and Live glass.
use super::*;

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    {
        let app = app.clone();
        ui.on_backdrop_updated(move || {
            let frame = {
                let a = app.borrow();
                let mut frame = a.incoming_frame.lock().unwrap_or_else(|e| e.into_inner());
                a.frame_scheduled.store(false, Ordering::Release);
                frame.take()
            };
            if let Some(frame) = frame {
                let a = app.borrow();
                if a.settings.appearance == Appearance::LiveGlass {
                    let changed = a.glass.borrow_mut().set_backdrop(frame);
                    if changed {
                        a.render();
                    }
                }
            }
        });
    }
}

pub(super) fn configure_material(app: &Rc<RefCell<App>>) {
    if app.borrow().hidden {
        return;
    }
    app.borrow().render();
    let old = app.borrow_mut().capture.take();
    drop(old);
    let _ = platform::exclude_from_capture(false);
    let settings = app.borrow().settings.clone();
    if let Err(e) = platform::set_appearance(settings.appearance) {
        status(app, &e);
        if matches!(settings.appearance, Appearance::Acrylic) {
            app.borrow_mut().settings.appearance = Appearance::Transparent;
        }
    }
    if settings.appearance == Appearance::Glass {
        if let Err(e) = platform::exclude_from_capture(true) {
            status(app, &e);
        } else {
            platform::settle_exclusion();
            refresh_eco_glass(app);
        }
    } else if settings.appearance == Appearance::LiveGlass {
        let setup = (|| -> Result<platform::capture::CaptureService, String> {
            platform::exclude_from_capture(true)?;
            let bounds = platform::capture_bounds()?;
            let a = app.borrow();
            let ui = a.ui.clone();
            let incoming = a.incoming_frame.clone();
            let scheduled = a.frame_scheduled.clone();
            drop(a);
            scheduled.store(false, Ordering::Release);
            platform::capture::CaptureService::start(
                bounds,
                platform::selected_monitor_handle(),
                move |frame| {
                    *incoming.lock().unwrap_or_else(|e| e.into_inner()) = Some(Backdrop {
                        width: frame.width,
                        height: frame.height,
                        pixels: frame.pixels,
                    });
                    if !scheduled.swap(true, Ordering::AcqRel)
                        && ui
                            .upgrade_in_event_loop(|ui| ui.invoke_backdrop_updated())
                            .is_err()
                    {
                        scheduled.store(false, Ordering::Release);
                    }
                },
            )
        })();
        match setup {
            Ok(capture) => app.borrow_mut().capture = Some(capture),
            Err(e) => {
                let _ = platform::exclude_from_capture(false);
                app.borrow_mut().settings.appearance = Appearance::Acrylic;
                let _ = platform::set_appearance(Appearance::Acrylic);
                status(app, &format!("Glass unavailable; acrylic active: {e}"));
            }
        }
    } else {
        app.borrow().glass.borrow_mut().clear();
    }
    app.borrow().render();
}

pub(super) fn refresh_eco_glass(app: &Rc<RefCell<App>>) {
    if !app.borrow().attached
        || app.borrow().hidden
        || app.borrow().settings.appearance != Appearance::Glass
    {
        return;
    }
    match platform::capture_snapshot() {
        Ok(frame) => {
            let a = app.borrow();
            let changed = a.glass.borrow_mut().set_backdrop(Backdrop {
                width: frame.width,
                height: frame.height,
                pixels: frame.pixels,
            });
            if changed {
                a.render();
            }
        }
        Err(e) => status(app, &e),
    }
}
