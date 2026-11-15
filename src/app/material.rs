//! Live desktop capture and on-demand backdrop fallback.
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
                ui.set_gpu_material(false);
                ui.set_glass_texture(slint::Image::default());
                ui.set_has_glass_texture(false);
            }
            // SAFETY: the image is cleared and teardown owns the current context.
            unsafe { release_renderer(&mut renderer); }
            return;
        }
        let Some(ui) = ui_weak.upgrade() else { return };
        let Some(app) = weak.upgrade() else { return };
        if app.borrow().verification { app.borrow().metrics.frame(state.clone()); }
        match state {
            slint::RenderingState::RenderingSetup => {
                failed = false;
                sampled = None;
            }
            slint::RenderingState::BeforeRendering => {
                use crate::liquid::Optics;

                let a = app.borrow();
                let appearance = a.settings.appearance;
                let optics = match appearance {
                    Appearance::Acrylic
                        if !(a.verification && verification::has_arg("--cpu-acrylic")) =>
                    {
                        Some(Optics::Acrylic)
                    }
                    Appearance::Glass | Appearance::LiveGlass => Some(Optics::Glass),
                    Appearance::LiquidGlass => Some(Optics::Liquid),
                    _ => None,
                };
                let Some(optics) = optics else {
                    ui.set_gpu_material(false);
                    if renderer.is_some()
                        || matches!(appearance, Appearance::Solid | Appearance::Transparent)
                    {
                        ui.set_glass_texture(slint::Image::default());
                        ui.set_has_glass_texture(false);
                    }
                    // SAFETY: this notifier owns the context and detached the image.
                    unsafe { release_renderer(&mut renderer); }
                    sampled = None;
                    failed = false;
                    return;
                };
                if a.hidden || failed {
                    return;
                }
                let liquid = optics == Optics::Liquid;
                let has_backdrop = {
                    let glass = a.glass.borrow();
                    if liquid { glass.liquid_backdrop.is_some() } else { glass.backdrop.is_some() }
                };
                drop(a);
                if !has_backdrop {
                    return;
                }
                if renderer.is_none() {
                    // SAFETY: Slint guarantees this window's context is current.
                    match unsafe { crate::liquid::Renderer::for_graphics_api(api) } {
                        Ok(gpu) => renderer = Some(gpu),
                        Err(error) => {
                            record(&format!("Material GPU setup failed: {error}"));
                            failed = true;
                            if liquid {
                                status(&app, "Liquid Glass graphics unavailable. Update the graphics driver and try again.");
                            }
                            return;
                        }
                    }
                }
                let result = {
                    let a = app.borrow();
                    let Some(gpu) = renderer.as_mut() else { return };
                    let contour = crate::shape::ShapeContour::new(a.springs[0].position,a.springs[1].position,a.springs[2].position,a.settings.form_factor);
                    let mut glass = a.glass.borrow_mut();
                    let revision = if liquid { glass.liquid_revision } else { glass.backdrop_revision() };
                    let sample_key=(revision,a.settings.opacity.to_bits());
                    if liquid && sampled != Some(sample_key) {
                        let Glass { liquid_backdrop, palette, .. } = &mut *glass;
                        if let Some(frame) = liquid_backdrop {
                            let sx=frame.width as f32/a.canvas_size.0 as f32;
                            let sy=frame.height as f32/a.canvas_size.1 as f32;
                            let left=(a.canvas_size.0 as f32-contour.width as f32)*0.5;
                            palette.sample(frame,[(left+14.)*sx,8.*sy,(left+contour.width as f32-14.)*sx,(contour.height as f32-8.).max(9.)*sy],Instant::now(),a.settings.opacity);
                        }
                        sampled = Some(sample_key);
                    }
                    if liquid { ui.set_liquid_dark_backdrop(glass.palette.dark); }
                    let mut geometry = crate::liquid::Geometry::new(
                        [a.canvas_size.0 as f32, a.canvas_size.1 as f32],
                        contour,
                        ui.window().scale_factor() * a.settings.island_scale as f32,
                        if liquid { a.settings.opacity } else { 0. },
                        liquid && glass.palette.dark,
                    );
                    geometry.optics = optics;
                    geometry.sample_y = a.slide.position as f32;
                    match if liquid { glass.liquid_backdrop.as_ref() } else { glass.backdrop.as_ref() } {
                        Some(frame) => gpu.render(frame, revision, geometry, liquid && glass.palette.mixed),
                        None => Ok(None),
                    }
                };
                match result {
                    Ok(Some(image)) => {
                        ui.set_gpu_material(true);
                        ui.set_glass_texture(image); ui.set_has_glass_texture(true);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        ui.set_gpu_material(false);
                        ui.set_glass_texture(slint::Image::default());
                        ui.set_has_glass_texture(false);
                        // SAFETY: detach the borrowed image before freeing its GL storage.
                        unsafe { release_renderer(&mut renderer); }
                        record(&format!("Material GPU rendering failed: {error}"));
                        failed = true;
                        if liquid {
                            status(&app, "Liquid Glass graphics interrupted. Restart Still Island to retry.");
                        } else {
                            app.borrow().render();
                        }
                    }
                }
            }
            _ => {}
        }
    });
    if let Err(error) = result {
        record(&format!("Material renderer unavailable: {error:?}"));
    }
}

/// Call with the renderer's GL context current and its borrowed image detached.
unsafe fn release_renderer(renderer: &mut Option<crate::liquid::Renderer>) {
    if let Some(gpu) = renderer.take() {
        let submissions = gpu.submissions;
        let mean_ms = gpu.submission_ms / submissions.max(1) as f64;
        unsafe { gpu.destroy() };
        record(&format!(
            "liquid_gpu released submissions={submissions} mean_submit_ms={mean_ms:.3} (CPU submission, not GPU completion or presentation)"
        ));
    }
}

pub(super) enum MaterialFrame {
    Prepared(crate::material::PreparedBackdrop),
    Liquid(Backdrop),
}

pub(super) type SnapshotReply = (u64, Result<MaterialFrame, String>);

/// UI-owned scheduling state. There is at most one capture task; bursts collapse
/// into one follow-up sample. The worker retains no App/Slint/GDI handles.
#[derive(Default)]
pub(super) struct Snapshots {
    generation: u64,
    excluded: bool,
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
        self.excluded = false;
        self.again = false;
        // Do not clear busy: the old generation still owns the sole worker slot.
        // Its completion will release it, then service a newer queued request.
    }
}

fn uses_live_capture(appearance: Appearance) -> bool {
    matches!(
        appearance,
        Appearance::Acrylic | Appearance::Glass | Appearance::LiveGlass | Appearance::LiquidGlass
    )
}

fn prepare_frame(
    appearance: Appearance,
    frame: platform::capture::BackdropFrame,
) -> Result<MaterialFrame, String> {
    let backdrop = Backdrop {
        width: frame.width,
        height: frame.height,
        pixels: frame.pixels,
    };
    if appearance == Appearance::LiquidGlass {
        Ok(MaterialFrame::Liquid(backdrop))
    } else {
        Glass::prepare_backdrop(backdrop)
            .map(MaterialFrame::Prepared)
            .ok_or_else(|| "Desktop returned an invalid backdrop.".into())
    }
}

fn publish_frame(app: &App, frame: MaterialFrame) {
    let changed = {
        let mut glass = app.glass.borrow_mut();
        match frame {
            MaterialFrame::Prepared(frame) => glass.set_prepared_backdrop(frame),
            MaterialFrame::Liquid(frame) => glass.set_liquid_backdrop(frame),
        }
    };
    if changed && let Some(ui) = app.ui.upgrade() {
        if app.settings.appearance == Appearance::LiquidGlass || ui.get_gpu_material() {
            ui.window().request_redraw();
        } else {
            app.render();
        }
    }
}

fn snapshot_allowed(app: &App) -> bool {
    app.attached
        && !app.hidden
        && uses_live_capture(app.settings.appearance)
        && app.snapshots.excluded
        && app.capture.is_none()
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
        (current && snapshot_allowed(&a), again)
    };
    if valid {
        match result {
            Ok(frame) => publish_frame(&app.borrow(), frame),
            Err(error) => {
                record(&format!("Backdrop snapshot failed: {error}"));
                status(app, "Desktop background could not be sampled.");
            }
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
                        let a = app.borrow();
                        generation == a.capture_generation.load(Ordering::Acquire)
                            && !a.hidden
                            && a.capture.is_some()
                    };
                    if valid {
                        app.borrow_mut().capture.take();
                        platform::set_snapshot_backdrop(true);
                        record(&format!("Live backdrop failed: {error}"));
                        status(
                            &app,
                            "Live backdrop unavailable. Using an on-demand background.",
                        );
                        refresh_eco_glass(&app);
                    }
                    return;
                }
                let Ok(frame) = frame else { return };
                let a = app.borrow();
                if generation == a.capture_generation.load(Ordering::Acquire)
                    && !a.hidden
                    && a.capture.is_some()
                    && uses_live_capture(a.settings.appearance)
                {
                    publish_frame(&a, frame);
                }
            }
        });
    }
}

pub(super) fn resume_material(app: &Rc<RefCell<App>>) {
    let needs_capture = {
        let a = app.borrow();
        a.attached && !a.hidden && uses_live_capture(a.settings.appearance) && a.capture.is_none()
    };
    if needs_capture {
        configure_material(app);
    }
}

pub(super) fn configure_material(app: &Rc<RefCell<App>>) {
    platform::set_snapshot_backdrop(false);
    app.borrow()
        .capture_generation
        .fetch_add(1, Ordering::AcqRel);
    app.borrow_mut().snapshots.invalidate();
    {
        let a = app.borrow();
        if matches!(
            a.settings.appearance,
            Appearance::Solid | Appearance::Transparent
        ) {
            a.glass.borrow_mut().clear();
            if let Some(ui) = a.ui.upgrade() {
                ui.set_gpu_material(false);
                ui.set_glass_texture(slint::Image::default());
                ui.set_has_glass_texture(false);
                ui.window().request_redraw();
            }
        }
    }
    let old = app.borrow_mut().capture.take();
    drop(old);
    {
        let a = app.borrow();
        a.incoming_frame
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        a.frame_scheduled.store(false, Ordering::Release);
    }
    if !app.borrow().attached {
        return;
    }
    app.borrow().render();
    let appearance = app.borrow().settings.appearance;
    if let Err(error) = platform::set_appearance(appearance) {
        record(&format!("Native backdrop configuration failed: {error}"));
    }
    if !uses_live_capture(appearance) {
        if let Err(error) = platform::exclude_from_capture(false) {
            record(&format!("Could not restore normal capture: {error}"));
        }
        return;
    }
    if app.borrow().hidden {
        return;
    }
    if let Err(error) = platform::exclude_from_capture(true) {
        app.borrow().glass.borrow_mut().clear();
        if let Some(ui) = app.borrow().ui.upgrade() {
            ui.set_gpu_material(false);
            ui.set_glass_texture(slint::Image::default());
            ui.set_has_glass_texture(false);
            ui.window().request_redraw();
        }
        record(&format!(
            "Could not exclude the overlay from its backdrop: {error}"
        ));
        status(
            app,
            "Desktop background unavailable. Windows could not exclude Still Island from capture.",
        );
        return;
    }
    app.borrow_mut().snapshots.excluded = true;
    {
        let setup = (|| -> Result<platform::capture::CaptureService, String> {
            let bounds = platform::settled_capture_bounds()?;
            let a = app.borrow();
            let ui = a.ui.clone();
            let incoming = a.incoming_frame.clone();
            let scheduled = a.frame_scheduled.clone();
            let error_ui = ui.clone();
            let error_incoming = incoming.clone();
            let error_scheduled = scheduled.clone();
            let frame_generation = a.capture_generation.clone();
            let error_generation = frame_generation.clone();
            let generation = a.capture_generation.load(Ordering::Acquire);
            drop(a);
            scheduled.store(false, Ordering::Release);
            platform::capture::CaptureService::start(
                bounds,
                platform::selected_monitor_handle(),
                if appearance == Appearance::Glass {
                    30
                } else {
                    60
                },
                move |frame| {
                    if generation != frame_generation.load(Ordering::Acquire) {
                        return;
                    }
                    let frame = prepare_frame(appearance, frame);
                    let mut slot = incoming.lock().unwrap_or_else(|e| e.into_inner());
                    if generation != frame_generation.load(Ordering::Acquire) {
                        return;
                    }
                    *slot = Some((generation, frame));
                    drop(slot);
                    if !scheduled.swap(true, Ordering::AcqRel)
                        && ui
                            .upgrade_in_event_loop(|ui| ui.invoke_backdrop_updated())
                            .is_err()
                    {
                        scheduled.store(false, Ordering::Release);
                    }
                },
                move |error| {
                    let mut slot = error_incoming.lock().unwrap_or_else(|e| e.into_inner());
                    if generation != error_generation.load(Ordering::Acquire) {
                        return;
                    }
                    *slot = Some((generation, Err(error)));
                    drop(slot);
                    if !error_scheduled.swap(true, Ordering::AcqRel) {
                        let _ = error_ui.upgrade_in_event_loop(|ui| ui.invoke_backdrop_updated());
                    }
                },
            )
        })();
        match setup {
            Ok(capture) => app.borrow_mut().capture = Some(capture),
            Err(error) => {
                platform::set_snapshot_backdrop(true);
                record(&format!("Live backdrop capture failed: {error}"));
                status(
                    app,
                    "Live backdrop unavailable. Using an on-demand background.",
                );
                refresh_eco_glass(app);
            }
        }
    }
}

pub(super) fn refresh_eco_glass(app: &Rc<RefCell<App>>) {
    if verification::has_arg("--verify-liquid") {
        return;
    }
    let (generation, appearance) = {
        let mut a = app.borrow_mut();
        if !snapshot_allowed(&a) {
            return;
        }
        let Some(generation) = a.snapshots.begin() else {
            return;
        };
        (generation, a.settings.appearance)
    };
    // Only bounds resolution touches the UI-thread HWND. The potentially slow
    // desktop BitBlt/readback executes entirely on the short-lived task below.
    let bounds = match platform::settled_capture_bounds() {
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
            platform::settle_exclusion();
            let result = platform::capture_snapshot_at(bounds)
                .and_then(|frame| prepare_frame(appearance, frame));
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
