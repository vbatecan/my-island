//! Backdrop lifecycle and event delivery for Eco and Live glass.
use super::*;

type SnapshotReply = (u64, Result<Backdrop, String>);

/// UI-owned scheduling state. There is at most one capture task; bursts collapse
/// into one follow-up sample. The worker retains no App/Slint/GDI handles.
#[derive(Default)]
pub(super) struct Snapshots {
    generation: u64,
    busy: bool,
    again: bool,
    inbox: Arc<Mutex<Option<SnapshotReply>>>,
}
impl Snapshots {
    fn begin(&mut self) -> Option<u64> {
        if self.busy {
            self.again = true;
            return None;
        }
        self.busy = true;
        Some(self.generation)
    }
    fn complete(&mut self, generation: u64) -> (bool, bool) {
        self.busy = false;
        (
            generation == self.generation,
            std::mem::take(&mut self.again),
        )
    }
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.again = false;
        // Do not clear busy: the old generation still owns the sole worker slot.
        // Its completion will release it, then service a newer queued request.
    }
}

fn uses_snapshot(appearance: Appearance) -> bool {
    matches!(appearance, Appearance::Glass | Appearance::Acrylic)
}

fn receive_snapshot(app: &Rc<RefCell<App>>) {
    let reply = {
        let a = app.borrow();
        a.snapshots
            .inbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    };
    let Some((generation, result)) = reply else {
        return;
    };
    let (valid, again) = {
        let mut a = app.borrow_mut();
        let (current, again) = a.snapshots.complete(generation);
        (
            current && a.attached && !a.hidden && uses_snapshot(a.settings.appearance),
            again,
        )
    };
    if valid {
        match result {
            Ok(frame) => {
                let changed = app.borrow().glass.borrow_mut().set_backdrop(frame);
                if changed {
                    app.borrow().render();
                }
            }
            Err(error) => status(app, &error),
        }
    }
    if again {
        refresh_eco_glass(app);
    }
}

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    {
        let app = app.clone();
        ui.on_backdrop_updated(move || {
            receive_snapshot(&app);
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
    app.borrow_mut().snapshots.invalidate();
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
    if matches!(settings.appearance, Appearance::Glass | Appearance::Acrylic) {
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
                if platform::exclude_from_capture(true).is_ok() {
                    platform::settle_exclusion();
                    refresh_eco_glass(app);
                }
                status(app, &format!("Glass unavailable; acrylic active: {e}"));
            }
        }
    } else {
        app.borrow().glass.borrow_mut().clear();
    }
    app.borrow().render();
}

pub(super) fn refresh_eco_glass(app: &Rc<RefCell<App>>) {
    let generation = {
        let mut a = app.borrow_mut();
        if !a.attached || a.hidden || !uses_snapshot(a.settings.appearance) {
            return;
        }
        let Some(generation) = a.snapshots.begin() else {
            return;
        };
        generation
    };
    // Only bounds resolution touches the UI-thread HWND. The potentially slow
    // desktop BitBlt/readback executes entirely on the short-lived task below.
    let bounds = match platform::capture_bounds() {
        Ok(bounds) => bounds,
        Err(error) => {
            app.borrow_mut().snapshots.busy = false;
            status(app, &error);
            return;
        }
    };
    let (inbox, ui) = {
        let a = app.borrow();
        (a.snapshots.inbox.clone(), a.ui.clone())
    };
    if let Err(error) = std::thread::Builder::new()
        .name("island-backdrop".into())
        .spawn(move || {
            let result = platform::capture_snapshot_at(bounds).map(|frame| Backdrop {
                width: frame.width,
                height: frame.height,
                pixels: frame.pixels,
            });
            *inbox.lock().unwrap_or_else(|e| e.into_inner()) = Some((generation, result));
            let _ = ui.upgrade_in_event_loop(|ui| ui.invoke_backdrop_updated());
        })
    {
        app.borrow_mut().snapshots.busy = false;
        record(&format!("Backdrop worker could not start: {error}"));
        status(app, "Desktop background could not be sampled.");
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn event_bursts_keep_one_task_and_one_followup() {
        let mut snapshots = Snapshots::default();
        let generation = snapshots.begin().unwrap();
        for _ in 0..100 {
            assert!(snapshots.begin().is_none());
        }
        assert_eq!(snapshots.complete(generation), (true, true));
        assert!(!snapshots.busy && !snapshots.again);
        let next = snapshots.begin().unwrap();
        assert_eq!(snapshots.complete(next), (true, false));
    }

    #[test]
    fn material_reconfigure_rejects_old_pixels_without_spawning_second_task() {
        let mut snapshots = Snapshots::default();
        let old = snapshots.begin().unwrap();
        snapshots.invalidate();
        assert!(snapshots.busy);
        assert!(snapshots.begin().is_none());
        assert_eq!(snapshots.complete(old), (false, true));
        let current = snapshots.begin().unwrap();
        assert_ne!(old, current);
        assert_eq!(snapshots.complete(current), (true, false));
    }
}
