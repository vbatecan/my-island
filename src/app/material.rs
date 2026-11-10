//! Backdrop lifecycle and event delivery for Eco and Live glass.
use super::*;

/// The sole main-window rendering notifier owns both optics and instrumentation.
pub(super) fn install_renderer(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    let weak = Rc::downgrade(app);
    let ui_weak = ui.as_weak();
    let mut renderer: Option<crate::liquid::Renderer> = None;
    let mut sampled = None;
    let mut failed = false;
    let result = ui.window().set_rendering_notifier(move |state, api| {
        if matches!(state, slint::RenderingState::RenderingTeardown) {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_glass_texture(slint::Image::default());
                ui.set_has_glass_texture(false);
            }
            if let Some(gpu) = renderer.take() {
                record(&format!("liquid_gpu submissions={} mean_submit_ms={:.3} (CPU submission, not GPU completion or presentation)",gpu.submissions,gpu.submission_ms/gpu.submissions.max(1) as f64));
                // SAFETY: the property is released or its window has been destroyed;
                // teardown still owns the current context even when App is gone.
                unsafe { gpu.destroy(); }
            }
            return;
        }
        let Some(ui) = ui_weak.upgrade() else { return };
        let Some(app) = weak.upgrade() else { return };
        if app.borrow().verification { app.borrow().metrics.frame(state.clone()); }
        match state {
            slint::RenderingState::RenderingSetup => {
                failed = false;
                sampled = None;
                if let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api {
                    // SAFETY: Slint guarantees this window's context is current.
                    let result = unsafe {
                        crate::liquid::Renderer::new(glow::Context::from_loader_function_cstr(get_proc_address))
                    };
                    match result {
                        Ok(gpu) => renderer = Some(gpu),
                        Err(error) => { record(&format!("Liquid GPU setup failed: {error}")); failed = true; }
                    }
                } else { failed = true; }
            }
            slint::RenderingState::BeforeRendering => {
                if failed {
                    if app.borrow().settings.appearance == Appearance::LiquidGlass {
                        status(&app,"Liquid Glass graphics unavailable. Update the graphics driver and restart Still Island.");
                    }
                    return;
                }
                let result = {
                    let a = app.borrow();
                    if a.hidden || a.settings.appearance != Appearance::LiquidGlass {
                        if let Some(gpu) = renderer.as_mut() { gpu.invalidate(); }
                        return;
                    }
                    let Some(gpu) = renderer.as_mut() else { return };
                    let contour = crate::shape::ShapeContour::new(a.springs[0].position,a.springs[1].position,a.springs[2].position,a.settings.form_factor);
                    let mut glass = a.glass.borrow_mut();
                    let revision = glass.liquid_revision;
                    if sampled != Some(revision) {
                        let Glass { liquid_backdrop, palette, .. } = &mut *glass;
                        if let Some(frame) = liquid_backdrop {
                            let sx=frame.width as f32/a.canvas_size.0 as f32;
                            let sy=frame.height as f32/a.canvas_size.1 as f32;
                            let left=(a.canvas_size.0 as f32-contour.width as f32)*0.5;
                            palette.sample(frame,[(left+14.)*sx,8.*sy,(left+contour.width as f32-14.)*sx,(contour.height as f32-8.).max(9.)*sy],Instant::now());
                        }
                        sampled = Some(revision);
                    }
                    ui.set_liquid_dark_backdrop(glass.palette.dark);
                    let geometry=crate::liquid::Geometry::new([a.canvas_size.0 as f32,a.canvas_size.1 as f32],contour,
                        ui.window().scale_factor()*a.settings.island_scale as f32,a.settings.opacity,glass.palette.dark);
                    match glass.liquid_backdrop.as_ref() {
                        Some(frame) => gpu.render(frame,revision,geometry,contour,glass.palette.mixed),
                        None => Ok(None),
                    }
                };
                match result {
                    Ok(Some(image)) => { ui.set_glass_texture(image); ui.set_has_glass_texture(true); }
                    Ok(None) => {}
                    Err(error) => {
                        record(&format!("Liquid GPU rendering failed: {error}")); failed=true;
                        status(&app,"Liquid Glass graphics interrupted. Restart Still Island to retry.");
                    }
                }
            }
            slint::RenderingState::RenderingTeardown => {
                ui.set_glass_texture(slint::Image::default()); ui.set_has_glass_texture(false);
                if let Some(gpu) = renderer.take() {
                    record(&format!("liquid_gpu submissions={} mean_submit_ms={:.3} (CPU submission, not GPU completion or presentation)",gpu.submissions,gpu.submission_ms/gpu.submissions.max(1) as f64));
                    // SAFETY: images were released above; context is still current.
                    unsafe { gpu.destroy(); }
                }
            }
            _ => {}
        }
    });
    if let Err(error) = result {
        record(&format!("Material renderer unavailable: {error:?}"));
    }
}

pub(super) type SnapshotReply = (u64, Result<Backdrop, String>);

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
    matches!(
        appearance,
        Appearance::Glass | Appearance::Acrylic | Appearance::LiquidGlass
    )
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
                let a = app.borrow();
                let accept = if a.settings.appearance == Appearance::LiquidGlass {
                    Glass::set_liquid_backdrop
                } else {
                    Glass::set_backdrop
                };
                let changed = accept(&mut a.glass.borrow_mut(), frame);
                drop(a);
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
            if let Some((generation, frame)) = frame {
                if let Err(error) = &frame {
                    let valid = {
                        let a=app.borrow();
                        generation==a.capture_generation.load(Ordering::Acquire) && !a.hidden
                    };
                    if valid {
                        app.borrow_mut().capture.take();
                        record(&format!("Live backdrop failed: {error}"));
                        status(&app,"Live backdrop unavailable; using a static backdrop. Reselect the material to retry.");
                        refresh_eco_glass(&app);
                    }
                    return;
                }
                let Ok(frame)=frame else {return};
                let a = app.borrow();
                if generation == a.capture_generation.load(Ordering::Acquire)
                    && !a.hidden
                    && a.capture.is_some()
                    && matches!(
                        a.settings.appearance,
                        Appearance::LiveGlass | Appearance::LiquidGlass
                    )
                {
                    let accept = if a.settings.appearance == Appearance::LiquidGlass {
                        Glass::set_liquid_backdrop
                    } else {
                        Glass::set_backdrop
                    };
                    let changed = accept(&mut a.glass.borrow_mut(), frame);
                    if changed {
                        a.render();
                    }
                }
            }
        });
    }
}

pub(super) fn configure_material(app: &Rc<RefCell<App>>) {
    app.borrow()
        .capture_generation
        .fetch_add(1, Ordering::AcqRel);
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
    } else if matches!(
        settings.appearance,
        Appearance::LiveGlass | Appearance::LiquidGlass
    ) {
        let setup = (|| -> Result<platform::capture::CaptureService, String> {
            platform::exclude_from_capture(true)?;
            let bounds = platform::capture_bounds()?;
            let a = app.borrow();
            let ui = a.ui.clone();
            let incoming = a.incoming_frame.clone();
            let scheduled = a.frame_scheduled.clone();
            let error_ui = ui.clone();
            let error_incoming = incoming.clone();
            let error_scheduled = scheduled.clone();
            let generation = a.capture_generation.load(Ordering::Acquire);
            drop(a);
            scheduled.store(false, Ordering::Release);
            platform::capture::CaptureService::start(
                bounds,
                platform::selected_monitor_handle(),
                move |frame| {
                    *incoming.lock().unwrap_or_else(|e| e.into_inner()) = Some((
                        generation,
                        Ok(Backdrop {
                            width: frame.width,
                            height: frame.height,
                            pixels: frame.pixels,
                        }),
                    ));
                    if !scheduled.swap(true, Ordering::AcqRel)
                        && ui
                            .upgrade_in_event_loop(|ui| ui.invoke_backdrop_updated())
                            .is_err()
                    {
                        scheduled.store(false, Ordering::Release);
                    }
                },
                move |error| {
                    *error_incoming.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((generation, Err(error)));
                    if !error_scheduled.swap(true, Ordering::AcqRel) {
                        let _ = error_ui.upgrade_in_event_loop(|ui| ui.invoke_backdrop_updated());
                    }
                },
            )
        })();
        match setup {
            Ok(capture) => app.borrow_mut().capture = Some(capture),
            Err(e) => {
                let _ = platform::exclude_from_capture(false);
                if settings.appearance != Appearance::LiquidGlass {
                    app.borrow_mut().settings.appearance = Appearance::Acrylic;
                    let _ = platform::set_appearance(Appearance::Acrylic);
                }
                if platform::exclude_from_capture(true).is_ok() {
                    platform::settle_exclusion();
                    refresh_eco_glass(app);
                }
                record(&format!("Live backdrop capture failed: {e}"));
                status(
                    app,
                    "Live backdrop unavailable; using a static backdrop. Reselect the material to retry.",
                );
            }
        }
    } else {
        app.borrow().glass.borrow_mut().clear();
    }
    app.borrow().render();
}

pub(super) fn refresh_eco_glass(app: &Rc<RefCell<App>>) {
    if verification::has_arg("--verify-liquid") {
        return;
    }
    let generation = {
        let mut a = app.borrow_mut();
        if !a.attached
            || a.hidden
            || !uses_snapshot(a.settings.appearance)
            || (a.settings.appearance == Appearance::LiquidGlass && a.capture.is_some())
        {
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
