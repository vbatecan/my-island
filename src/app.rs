mod artwork;
mod battery_verification;
mod contracted_verification;
mod interaction;
mod launcher;
mod layout_verification;
mod lyrics;
mod lyrics_verification;
mod material;
mod media;
mod media_polish_verification;
mod notification_bridge;
mod page_verification;
mod pages;
mod plugins;
mod settings_ui;
mod size_verification;
mod startup;
mod verification;
mod volume;

use interaction::handle_overlay_input;
use material::{configure_material, refresh_eco_glass};
use media::sync_media_clock;
use settings_ui::{
    apply_settings, close_preferences, refresh_preferences, settings_status, show_preferences,
};
use verification::record;

use crate::animation::choreography::{compact_space, expanded_space};
use crate::{IslandWindow, animation::Spring, platform, settings::Settings, state::IslandState};
use crate::{
    SettingsWindow,
    presentation::{Content, Panel, Presentation, PresentationCoordinator},
    services::notifications::{
        self, AccessStatus, NotificationEvent, NotificationRecord, NotificationService,
    },
    services::volume::VolumeOrigin,
};
use crate::{
    material::{Backdrop, Glass},
    services::{
        media::{MediaService, MediaSnapshot},
        volume::VolumeService,
    },
    settings::Appearance,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

struct App {
    launcher: launcher::Controller,
    plugin_host: Option<Arc<crate::plugins::PluginHost>>,
    ui: slint::Weak<IslandWindow>,
    state: IslandState,
    springs: [Spring; 5],
    timer: slint::Timer,
    last: Instant,
    settings: Settings,
    canvas_size: (f64, f64),
    startup: platform::startup::StartupStatus,
    startup_busy: bool,
    startup_pending: Option<bool>,
    hover: bool,
    pending_content: Option<IslandState>,
    pending_compact: Option<IslandState>,
    pending_geometry: Option<[f64; 3]>,
    verification: bool,
    frames: u64,
    volume: Option<Rc<VolumeService>>,
    media: Option<Rc<MediaService>>,
    lyrics: lyrics::Controller,
    media_clock: slint::Timer,
    media_clock_compact: bool,
    media_position: (f64, f64, Instant),
    glass: RefCell<Glass>,
    capture: Option<platform::capture::CaptureService>,
    incoming_frame: Arc<Mutex<Option<Backdrop>>>,
    frame_scheduled: Arc<AtomicBool>,
    preferences: slint::Weak<SettingsWindow>,
    coordinator: PresentationCoordinator,
    presentation: Option<Presentation>,
    pending_presentation: Option<Presentation>,
    rendered_presentation: Option<Presentation>,
    epoch: Instant,
    deadline: slint::Timer,
    hover_timer: slint::Timer,
    collapse_timer: slint::Timer,
    page_switch_timer: slint::Timer,
    page_wheel: pages::PageWheel,
    hover_intent: crate::hover::HoverIntent,
    pointer: Option<(f64, f64)>,
    pointer_pressed: bool,
    wheel: crate::gestures::WheelVolume,
    countdown: crate::countdown::Countdown,
    countdown_timer: slint::Timer,
    countdown_engaged: bool,
    hidden: bool,
    settings_visible: bool,
    settings_sync: bool,
    attached: bool,
    notifications: Option<Rc<NotificationService>>,
    notification_access: AccessStatus,
    notification_inbox: Arc<Mutex<VecDeque<NotificationEvent>>>,
    notification_scheduled: Arc<AtomicBool>,
    observed_apps: Vec<(String, String)>,
    metrics: Rc<crate::metrics::RenderMetrics>,
    battery: crate::services::battery::BatterySnapshot,
    battery_detector: crate::services::battery::BatteryDetector,
}
impl App {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn animations_enabled(&self) -> bool {
        self.settings.animations
            && !(self.settings.follow_reduced_motion && platform::prefers_reduced_motion())
    }

    fn target(&mut self, state: IslandState) {
        if self.verification {
            record(&format!("target {state:?}"));
        }
        self.state = state;
        if state == IslandState::Idle
            && self.springs[3].position.max(self.springs[4].position) < 0.01
        {
            self.apply_pending_presentation();
        }
        let g = self.geometry(state);
        self.update_media_layout();
        let collapsing = g.height < self.springs[1].position;
        for spring in &mut self.springs[..3] {
            spring.config.damping = self.settings.bounce.damping(collapsing);
        }

        let geometry = [
            g.width
                + if state == IslandState::Idle && self.hover {
                    4.0
                } else {
                    0.0
                },
            g.height,
            g.radius,
        ];
        // Give departing content a short head start. Keep the old geometry
        // target meanwhile; none of the springs lose their current momentum.
        if g.height < self.springs[1].position
            && self.springs[3].position.max(self.springs[4].position) > 0.2
            && self.animations_enabled()
        {
            self.pending_geometry = Some(geometry);
        } else {
            self.pending_geometry = None;
            for (spring, target) in self.springs[..3].iter_mut().zip(geometry) {
                spring.set_target(target);
            }
        }
        // Content leaves before geometry contracts. Existing content stays mounted during collapse.
        self.springs[3].set_target(0.0);
        self.springs[4].set_target(0.0);
        self.pending_compact = None;
        if state.is_expanded() {
            self.pending_content = Some(state);
        } else {
            self.pending_content = None;
            if matches!(state, IslandState::Compact) {
                self.pending_compact = Some(state);
            }
        }
        if !self.animations_enabled() {
            for spring in &mut self.springs[..3] {
                spring.snap(spring.target);
            }
            self.springs[3].snap(if state.is_expanded() { 1.0 } else { 0.0 });
            self.springs[4].snap(
                if matches!(state, IslandState::Compact | IslandState::Volume) {
                    1.0
                } else {
                    0.0
                },
            );
            self.apply_content();
            self.apply_compact();
            self.render();
        }
    }
    fn geometry(&self, state: IslandState) -> crate::state::Geometry {
        state.geometry_for_sizes(
            self.settings.form_factor,
            self.settings.notch_corner_radius,
            self.settings.notch_contracted_radius,
            platform::available_width() / self.settings.island_scale,
            &self.settings.sizes(),
        )
    }
    fn sync_canvas(&mut self, shrink: bool) -> bool {
        let requested = self.settings.canvas_size();
        let next = if shrink {
            requested
        } else {
            (
                self.canvas_size.0.max(requested.0),
                self.canvas_size.1.max(requested.1),
            )
        };
        let changed = next != self.canvas_size;
        self.canvas_size = next;
        changed
    }

    fn update_media_layout(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let media = self.geometry(IslandState::Media);
            ui.set_media_layout_width(media.width as f32);
            ui.set_media_layout_height(media.height as f32);
            let plugin = self.geometry(IslandState::Plugin);
            ui.set_plugin_layout_width((plugin.width - 48.).max(1.) as f32);
        }
    }

    /// Retarget only the contour when preferences or display geometry change.
    /// Content and all spring velocities survive the change.
    fn retarget_shape(&mut self) {
        let g = self.geometry(self.state);
        self.update_media_layout();
        let target = [
            g.width
                + if self.state == IslandState::Idle && self.hover {
                    4.
                } else {
                    0.
                },
            g.height,
            g.radius,
        ];
        if self.pending_geometry.is_some() {
            self.pending_geometry = Some(target);
        } else {
            let animate = self.animations_enabled();
            for (spring, target) in self.springs[..3].iter_mut().zip(target) {
                spring.set_target(target);
                if !animate {
                    spring.snap(target);
                }
            }
        }
    }
    fn apply_pending_presentation(&mut self) {
        if let Some(view) = self.pending_presentation.take()
            && let Some(ui) = self.ui.upgrade()
        {
            apply_content_properties(&ui, &view, &self.settings);
            self.rendered_presentation = Some(view);
        }
    }
    fn apply_content(&mut self) {
        self.apply_pending_presentation();
        if let Some(state) = self.pending_content.take()
            && let Some(ui) = self.ui.upgrade()
        {
            ui.set_media_mode(state == IslandState::Media);
            ui.set_volume_mode(state == IslandState::Volume);
        }
    }
    fn apply_compact(&mut self) {
        self.apply_pending_presentation();
        if let Some(state) = self.pending_compact.take()
            && let Some(ui) = self.ui.upgrade()
        {
            ui.set_volume_mode(state == IslandState::Volume);
        }
    }
    fn tick(&mut self) -> bool {
        if self.state == IslandState::Idle
            && self.springs[3].position.max(self.springs[4].position) < 0.01
        {
            self.apply_pending_presentation();
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64() * self.settings.animation_speed;
        self.last = now;
        if self.pending_content.is_some()
            && self.springs[3].position.max(self.springs[4].position) < 0.01
        {
            self.apply_content();
        }
        if self.pending_compact.is_some()
            && self.springs[3].position < 0.01
            && self.springs[4].position < 0.01
        {
            self.apply_compact();
        }
        if self.pending_geometry.is_some()
            && self.springs[3].position.max(self.springs[4].position) <= 0.2
        {
            let geometry = self.pending_geometry.take().unwrap();
            for (spring, target) in self.springs[..3].iter_mut().zip(geometry) {
                spring.set_target(target);
            }
        }
        let expanded = self.pending_content.is_none() && self.state.is_expanded();
        let compact = self.pending_compact.is_none()
            && matches!(self.state, IslandState::Compact | IslandState::Volume);
        // Space gates are continuous, so a reversal never pops content into a small capsule.
        self.springs[3].set_target(if expanded {
            expanded_space(self.springs[0].position, self.springs[1].position)
        } else {
            0.0
        });
        self.springs[4].set_target(if compact {
            compact_space(self.springs[0].position, self.springs[1].position)
        } else {
            0.0
        });
        let mut active = false;
        for spring in &mut self.springs {
            active |= spring.advance(dt);
        }
        active |= self.pending_presentation.is_some()
            || self.pending_geometry.is_some()
            || self.pending_content.is_some()
            || self.pending_compact.is_some();
        self.render();
        self.frames += 1;
        if !active && self.verification {
            record(&format!(
                "settled {:?} frames={} timer stopped; {}",
                self.state,
                self.frames,
                platform::diagnostic_snapshot()
            ));
        }
        active
    }
    fn render(&self) {
        if self.hidden {
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_ui_scale(self.settings.island_scale as f32);
            ui.set_text_scale(self.settings.text_scale as f32);
            ui.set_media_motion_enabled(self.animations_enabled());
            ui.set_canvas_width(self.canvas_size.0 as f32);
            ui.set_canvas_height(self.canvas_size.1 as f32);
            ui.set_animation_speed(self.settings.animation_speed as f32);
            ui.set_shape_width(self.springs[0].position as f32);
            ui.set_shape_height(self.springs[1].position as f32);
            ui.set_shape_radius(self.springs[2].position as f32);
            let contour = crate::shape::ShapeContour::new(
                self.springs[0].position,
                self.springs[1].position,
                self.springs[2].position,
                self.settings.form_factor,
            );
            ui.set_form_notch(self.settings.form_factor == crate::shape::FormFactor::Notch);
            ui.set_shoulder(contour.shoulder() as f32);
            ui.set_shape_path(contour.svg_commands().into());
            let width = self.springs[0].position;
            let height = self.springs[1].position;
            let reveal = self.springs[3].position.clamp(0.0, 1.0) * expanded_space(width, height);
            ui.set_reveal(reveal as f32);
            ui.set_compact_reveal(
                (self.springs[4].position.clamp(0.0, 1.0) * compact_space(width, height)) as f32,
            );
            ui.set_content_shift((8.0 * (1.0 - reveal)) as f32);
            ui.set_appearance(if self.settings.appearance == Appearance::LiveGlass {
                3
            } else {
                self.settings.appearance as i32
            });
            ui.set_tint_opacity(self.settings.opacity);
            ui.set_material_name(self.settings.appearance.name().into());
            if matches!(
                self.settings.appearance,
                Appearance::Glass | Appearance::LiveGlass
            ) {
                let image = self.glass.borrow_mut().image_in_canvas(
                    width,
                    height,
                    self.springs[2].position,
                    ui.window().scale_factor() * self.settings.island_scale as f32,
                    self.settings.form_factor,
                    self.canvas_size.0,
                );
                ui.set_glass_texture(image);
                ui.set_has_glass_texture(true);
            } else {
                ui.set_glass_texture(slint::Image::default());
                ui.set_has_glass_texture(false);
            }
            platform::update_shape(
                self.springs[0].position,
                self.springs[1].position,
                self.springs[2].position,
                self.settings.offset,
                self.settings.topmost,
                self.settings.form_factor,
                self.settings.island_scale,
                self.canvas_size.0,
                self.canvas_size.1,
            );
        }
    }
}
fn content_spring() -> Spring {
    let mut spring = Spring::new(0.0);
    // A critically damped, faster envelope lets text leave before contraction.
    spring.config.stiffness = 1600.0;
    spring.config.damping = 80.0;
    spring.config.position_threshold = 0.001;
    spring.config.velocity_threshold = 0.01;
    spring
}

fn wake(app: &Rc<RefCell<App>>) {
    if !app.borrow().animations_enabled() || app.borrow().hidden {
        return;
    }
    if app.borrow().timer.running() {
        return;
    }
    app.borrow_mut().last = Instant::now();
    app.borrow().metrics.active(true);
    let weak = Rc::downgrade(app);
    app.borrow().timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(8),
        move || {
            if let Some(app) = weak.upgrade() {
                let active = app.borrow_mut().tick();
                if !active {
                    app.borrow().timer.stop();
                    app.borrow().metrics.active(false);
                    if app.borrow_mut().sync_canvas(true) {
                        configure_material(&app);
                    }
                }
                interaction::refresh_pointer(&app);
            }
        },
    );
}

fn morph_to(app: &Rc<RefCell<App>>, state: IslandState) {
    refresh_eco_glass(app);
    app.borrow_mut().target(state);
    wake(app);
    sync_media_clock(app);
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _instance = platform::initialize()?;
    use slint::winit_030::winit::{
        platform::windows::WindowAttributesExtWindows, window::WindowLevel,
    };
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        .with_winit_window_attributes_hook(|attributes| {
            attributes
                .with_skip_taskbar(true)
                .with_active(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
        })
        .select()?;
    let ui = IslandWindow::new()?;
    let preferences = SettingsWindow::new()?;
    let mut settings = Settings::load();
    for arg in std::env::args() {
        if let Some(v) = arg.strip_prefix("--form=") {
            settings.form_factor = crate::shape::FormFactor::parse(v);
        }
        if let Some(v) = arg.strip_prefix("--appearance=") {
            settings.appearance = Appearance::parse(v);
        }
        if let Some(v) = arg.strip_prefix("--opacity=")
            && let Ok(v) = v.parse::<f32>()
            && v.is_finite()
        {
            settings.opacity = v.clamp(0.15, 0.95);
        }
        if let Some(v) = arg.strip_prefix("--bounce=") {
            settings.bounce = crate::settings::Bounce::parse(v);
        }
    }
    settings.normalize();
    if verification::enabled() {
        settings.selected_page = None;
        settings.auto_volume = false;
        settings.compact_now_playing = false;
        settings.notifications_enabled = false;
        settings.hover_expand = false;
        // Verification drives the GUI directly; real desktop clicks, hotkeys,
        // and foreground changes must not interrupt its deterministic stages.
        settings.outside_click = false;
        settings.shortcut_enabled = false;
        settings.launcher_enabled = false;
        settings.fullscreen_hide = false;
    }
    if verification::has_arg("--verify-interactions") {
        settings.hover_expand = true;
        settings.hover_delay = 200;
        settings.collapse_delay = 150;
    }
    if verification::has_arg("--benchmark") {
        settings.notifications_enabled = false;
    }
    let initial_geometry = IslandState::Idle.geometry_for_sizes(
        settings.form_factor,
        settings.notch_corner_radius,
        settings.notch_contracted_radius,
        platform::available_width() / settings.island_scale,
        &settings.sizes(),
    );
    let canvas_size = settings.canvas_size();
    let mut coordinator = PresentationCoordinator::default();
    coordinator.set_selected_page(settings.selected_page.clone(), 0);
    let app = Rc::new(RefCell::new(App {
        launcher: launcher::Controller::default(),
        plugin_host: None,
        ui: ui.as_weak(),
        state: IslandState::Idle,
        springs: [
            Spring::new(initial_geometry.width),
            Spring::new(initial_geometry.height),
            Spring::new(initial_geometry.radius),
            content_spring(),
            content_spring(),
        ],
        timer: slint::Timer::default(),
        last: Instant::now(),
        settings,
        canvas_size,
        startup: platform::startup::StartupStatus {
            enabled: false,
            available: false,
            message: "Checking Windows startup registration…".into(),
        },
        startup_busy: false,
        startup_pending: None,
        hover: false,
        pending_content: None,
        pending_compact: None,
        pending_geometry: None,
        verification: verification::enabled(),
        frames: 0,
        volume: None,
        media: None,
        lyrics: lyrics::Controller::default(),
        media_clock: slint::Timer::default(),
        media_clock_compact: false,
        media_position: (0., 0., Instant::now()),
        glass: RefCell::new(Glass::default()),
        capture: None,
        incoming_frame: Arc::new(Mutex::new(None)),
        frame_scheduled: Arc::new(AtomicBool::new(false)),
        preferences: preferences.as_weak(),
        coordinator,
        presentation: None,
        pending_presentation: None,
        rendered_presentation: None,
        epoch: Instant::now(),
        deadline: slint::Timer::default(),
        hover_timer: slint::Timer::default(),
        collapse_timer: slint::Timer::default(),
        page_switch_timer: slint::Timer::default(),
        page_wheel: pages::PageWheel::default(),
        hover_intent: crate::hover::HoverIntent::default(),
        pointer: None,
        pointer_pressed: false,
        wheel: crate::gestures::WheelVolume::default(),
        countdown: crate::countdown::Countdown::default(),
        countdown_timer: slint::Timer::default(),
        countdown_engaged: false,
        hidden: false,
        settings_visible: false,
        settings_sync: false,
        attached: false,
        notifications: None,
        notification_access: AccessStatus::NotRequested,
        notification_inbox: Arc::new(Mutex::new(VecDeque::new())),
        notification_scheduled: Arc::new(AtomicBool::new(false)),
        observed_apps: vec![],
        metrics: Rc::new(crate::metrics::RenderMetrics::default()),
        battery: crate::services::battery::BatterySnapshot::default(),
        battery_detector: crate::services::battery::BatteryDetector::default(),
    }));
    verification::install_metrics(&app, &ui);
    app.borrow().render();
    {
        let app = app.clone();
        ui.on_toggle(move || user_toggle(&app));
    }
    {
        let app = app.clone();
        ui.on_collapse(move || transition(&app, IslandState::Idle));
    }
    // Services notify the GUI; all remote work stays on their sleeping workers.
    volume::wire(&app, &ui);
    media::wire(&app, &ui);
    lyrics::wire(&app, &ui);
    material::wire(&app, &ui);
    plugins::wire(&app, &ui, &preferences);
    launcher::wire(&app, &ui);
    {
        let app = app.clone();
        ui.on_open_battery(move || transition(&app, IslandState::Battery));
    }
    let event_app = app.clone();
    platform::set_event_handler(Box::new(move |event| {
        use platform::Event;
        match event {
            Event::Show => {
                if let Some(ui) = event_app.borrow().ui.upgrade() {
                    let _ = ui.show();
                }
                event_app.borrow().render();
            }
            Event::Battery(snapshot) => {
                if !event_app.borrow().verification {
                    update_battery(&event_app, snapshot);
                }
            }
            Event::Reset => transition(&event_app, IslandState::Idle),
            Event::State(state) => transition(&event_app, state),
            Event::Exit => {
                let _ = slint::quit_event_loop();
            }

            Event::DisplayChanged => {
                configure_material(&event_app);
                refresh_preferences(&event_app);
                event_app.borrow_mut().retarget_shape();
                wake(&event_app);
                lyrics::sync(&event_app, false);
                interaction::refresh_pointer(&event_app);
                if event_app.borrow().launcher.open {
                    launcher::refresh(&event_app);
                }
            }
            Event::BackdropRefresh => refresh_eco_glass(&event_app),
            Event::OpenSettings => show_preferences(&event_app),
            Event::ClosePanel => {
                if event_app.borrow().launcher.open {
                    launcher::dismiss(&event_app, false);
                    return;
                }
                if !event_app.borrow().settings_visible && !event_app.borrow().coordinator.pinned()
                {
                    transition(&event_app, IslandState::Idle);
                }
            }
            Event::HotkeyToggle => user_toggle(&event_app),
            Event::LauncherToggle => launcher::toggle(&event_app),
            Event::DesktopPolicyChanged(hidden) => {
                if hidden == event_app.borrow().hidden {
                    // External foreground/z-order changes can move a window over
                    // a stationary pointer. Reconcile without rebuilding capture
                    // or rescheduling presentation when visibility did not change.
                    interaction::refresh_pointer(&event_app);
                    return;
                }
                if hidden && event_app.borrow().launcher.open {
                    launcher::dismiss(&event_app, false);
                }
                let old = {
                    let mut a = event_app.borrow_mut();
                    a.hidden = hidden;
                    let now = a.now();
                    let suppress = hidden || a.settings_visible;
                    a.coordinator.suppress(suppress, now);
                    if hidden {
                        if let Some(ui) = a.ui.upgrade() {
                            ui.set_media_motion_enabled(false);
                        }
                        a.hover = false;
                        a.pointer = None;
                        a.pointer_pressed = false;
                        a.hover_intent.leave();
                        a.hover_timer.stop();
                        a.collapse_timer.stop();
                        a.coordinator.set_hover(false, now);
                        a.coordinator.close_hover(now);
                        a.timer.stop();
                        a.media_clock.stop();
                        a.capture.take()
                    } else {
                        None
                    }
                };
                drop(old);
                if !hidden {
                    configure_material(&event_app);
                }
                sync_presentation(&event_app);
                sync_media_clock(&event_app);
                interaction::refresh_pointer(&event_app);
            }
        }
    }));
    use slint::winit_030::WinitWindowAccessor;
    let init_error = Rc::new(RefCell::new(None::<String>));
    let init_failure = init_error.clone();
    let init_app = app.clone();
    slint::spawn_local(async move {
        let result = (async {
            let ui = init_app.borrow().ui.upgrade().ok_or("window destroyed")?;
            let native_window = ui.window().winit_window().await?;
            let raw = native_window.window_handle()?;
            let hwnd = match raw.as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get(),
                _ => return Err("Windows HWND required".into()),
            };
            platform::attach(hwnd)?;
            if let Ok(snapshot)=platform::power::snapshot(){update_battery(&init_app,snapshot);}
            init_app.borrow_mut().attached=true;
            let settings=init_app.borrow().settings.clone();apply_settings(&init_app,settings,false);
            configure_material(&init_app);
            // Deterministic opt-in test runs ignore live pointer input; normal use is unchanged.
            if verification::blocks_live_input() {
                use slint::winit_030::{EventResult, winit::event::WindowEvent};
                ui.window().on_winit_window_event(|_, event| match event {
                    WindowEvent::MouseInput { .. }
                    | WindowEvent::CursorMoved { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::CursorLeft { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::Touch { .. } => EventResult::PreventDefault,
                    _ => EventResult::Propagate,
                });
            } else {
                let weak = Rc::downgrade(&init_app);
                ui.window().on_winit_window_event(move |_, event| {
                    if let Some(app) = weak.upgrade() { handle_overlay_input(&app,event) }
                    else { slint::winit_030::EventResult::Propagate }
                });
            }
            init_app.borrow().render();
            interaction::refresh_pointer(&init_app);
            if let Some(state) =
                std::env::args().find_map(|a| a.strip_prefix("--demo-state=").map(str::to_owned))
            {
                let state = match state.as_str() {
                    "compact" => IslandState::Compact,
                    "expanded" => IslandState::Expanded,
                    "volume" => IslandState::Volume,
                    "media" => IslandState::Media,
                    _ => IslandState::Idle,
                };
                transition(&init_app, state);
            }
            if init_app.borrow().verification {
                record(&format!(
                    "native attached {}",
                    platform::diagnostic_snapshot()
                ));
                record(&format!("services volume_available={} actual_volume={} muted={} media_available={} appearance={}",ui.get_volume_available(),ui.get_volume(),ui.get_volume_muted(),ui.get_media_available(),ui.get_material_name()));
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        })
        .await;
        if let Err(e) = result {
            record(&format!("native initialization FAILED {e}"));
            *init_failure.borrow_mut() = Some(e.to_string());
            let _ = slint::quit_event_loop();
        }
    })?;
    interaction::wire(&app, &ui);
    settings_ui::wire(&app, &ui, &preferences);
    startup::refresh(&app);
    notification_bridge::wire(&app, &ui);
    ui.show()?;
    app.borrow().render();
    let _verification_timer = verification::setup(&app);
    slint::run_event_loop()?;
    if app.borrow().verification {
        record(&app.borrow().metrics.summary());
    }
    platform::set_event_handler(Box::new(|_| {}));
    if let Some(error) = init_error.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}

fn transition(app: &Rc<RefCell<App>>, state: IslandState) {
    if state == IslandState::AppLauncher {
        launcher::toggle(app);
        return;
    }
    if app.borrow().launcher.open {
        launcher::dismiss(app, true);
        if state == IslandState::Idle {
            return;
        }
    }
    {
        let mut a = app.borrow_mut();
        let now = a.now();
        match state {
            IslandState::Idle => {
                a.coordinator.close(now);
                // An explicit collapse consumes this hover visit. Require a
                // fresh entry before automatically opening again.
                a.hover_intent.leave();
            }
            IslandState::Expanded => a.coordinator.open(Panel::Launcher, now),
            IslandState::Media => a.coordinator.open(Panel::Media, now),
            IslandState::Volume => a.coordinator.open(Panel::Volume, now),
            IslandState::Compact => a.coordinator.volume(now),
            IslandState::Notification => a.coordinator.expand_notification(now),
            IslandState::Timer => a.coordinator.open(Panel::Timer, now),
            IslandState::Battery => a.coordinator.open(Panel::Battery, now),
            IslandState::Plugin => a.coordinator.open(Panel::Plugins, now),
            IslandState::AppLauncher => unreachable!("handled before borrowing app"),
        }
    }
    sync_presentation(app);
}

fn user_toggle(app: &Rc<RefCell<App>>) {
    if app.borrow().launcher.open {
        launcher::dismiss(app, true);
        return;
    }
    {
        let mut a = app.borrow_mut();
        let now = a.now();
        if a.coordinator.panel_open() {
            a.coordinator.close(now);
            a.hover_intent.leave();
        } else if matches!(a.coordinator.view(now).content, Content::Notification(..)) {
            a.coordinator.expand_notification(now);
        } else {
            if a.coordinator.selected_page().is_some() {
                a.coordinator.open_hover(now);
            } else {
                a.coordinator.open(Panel::Launcher, now);
            }
        }
    }
    sync_presentation(app);
}

fn apply_content_properties(ui: &IslandWindow, view: &Presentation, settings: &Settings) {
    ui.set_app_launcher_mode(matches!(view.content, Content::AppLauncher));
    plugins::apply_content_properties(ui, view);
    ui.set_battery_mode(matches!(view.content, Content::BatteryPanel));
    ui.set_compact_battery(matches!(view.content, Content::CompactBattery(_)));
    if let Content::CompactBattery(activity) = &view.content {
        apply_battery_properties(ui, activity.snapshot, activity.label());
    }
    ui.set_timer_mode(matches!(
        view.content,
        Content::Timer | Content::TimerFinished
    ));
    ui.set_compact_timer(matches!(view.content, Content::CompactTimer));
    ui.set_notification_compact(matches!(view.content, Content::Notification(..)));
    ui.set_notification_mode(matches!(view.content, Content::ExpandedNotification(..)));
    ui.set_compact_media(matches!(view.content, Content::CompactMedia));
    if let Content::Notification(record, _) | Content::ExpandedNotification(record) = &view.content
    {
        let compact = matches!(view.content, Content::Notification(..));
        ui.set_notification_app(record.app_name.as_str().into());
        let title = if compact {
            match settings.privacy {
                crate::settings::Privacy::Identity => String::new(),
                crate::settings::Privacy::Title => record.title.clone(),
                crate::settings::Privacy::Body => format!("{} · {}", record.title, record.body),
            }
        } else {
            record.title.clone()
        };
        ui.set_notification_title(title.into());
        ui.set_notification_body(record.body.as_str().into());
        ui.set_notification_count(if let Content::Notification(_, count) = &view.content {
            *count as i32
        } else {
            1
        });
        ui.set_can_open_source(notifications::can_open_source(&record.app_id));
        if let Some(image) = record
            .icon
            .as_ref()
            .and_then(|data| slint::Image::load_from_data(data, None).ok())
        {
            ui.set_notification_icon(image);
            ui.set_has_notification_icon(true);
        } else {
            ui.set_has_notification_icon(false);
        }
    }
}

fn sync_presentation(app: &Rc<RefCell<App>>) {
    let (changed, state, deadline, panel, ui, hidden) = {
        let mut a = app.borrow_mut();
        let now = a.now();
        let view = a.coordinator.view(now);
        let changed = a
            .presentation
            .as_ref()
            .is_none_or(|old| old.content != view.content);
        let state = view.state();
        if let Some(ui) = a.ui.upgrade() {
            plugins::sync_visibility(&ui, &view);
        }
        let panel = a.coordinator.panel_open() && !a.settings_visible && !a.hidden;
        let ui = a.ui.upgrade();
        if let Some(ui) = &ui {
            ui.set_pending_count(view.pending as i32);
            ui.set_volume_feedback(view.volume_chip);
            ui.set_pinned(a.coordinator.pinned());
            if matches!(view.content, Content::BatteryPanel) {
                apply_battery_properties(ui, a.battery, a.battery.label());
            }
        }
        if changed {
            a.pending_presentation = Some(view.clone());
        }
        a.presentation = Some(view);
        (
            changed,
            state,
            a.coordinator.next_deadline(now),
            panel,
            ui,
            a.hidden,
        )
    };
    if app.borrow().attached
        && let Err(e) = platform::set_panel_open(panel)
    {
        settings_status(app, &e);
    }
    pages::sync_hint(app);
    if changed && state == IslandState::Plugin {
        let selected = app.borrow().settings.selected_page.clone();
        if let (Some(crate::navigation::Page::Plugin { id, view }), Some(ui)) = (selected, &ui) {
            ui.invoke_plugin_page_ensure(id.into(), view.into());
        }
    }
    if !hidden {
        if changed || app.borrow().pending_presentation.is_some() || app.borrow().state != state {
            morph_to(app, state);
        } else if app
            .borrow()
            .springs
            .iter()
            .any(|spring| !spring.is_settled())
        {
            wake(app);
        }
    }
    app.borrow().deadline.stop();
    if let Some(end) = deadline {
        let delay = end.saturating_sub(app.borrow().now()).max(1);
        let weak = Rc::downgrade(app);
        app.borrow().deadline.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(delay),
            move || {
                if let Some(app) = weak.upgrade() {
                    sync_presentation(&app);
                }
            },
        );
    }
    if ui.is_some() {
        sync_media_clock(app);
    }
    lyrics::sync(app, false);
}

fn status(app: &Rc<RefCell<App>>, message: &str) {
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_service_status(message.into());
    }
}

fn apply_battery_properties(
    ui: &IslandWindow,
    snapshot: crate::services::battery::BatterySnapshot,
    label: &str,
) {
    ui.set_battery_present(snapshot.present);
    ui.set_battery_percent_label(
        snapshot
            .percent
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "Unavailable".into())
            .into(),
    );
    ui.set_battery_level(snapshot.percent.map_or(0., |p| p as f32 / 100.));
    ui.set_battery_power_label(label.into());
}

fn update_battery(app: &Rc<RefCell<App>>, snapshot: crate::services::battery::BatterySnapshot) {
    let mut a = app.borrow_mut();
    let activity = a.battery_detector.update(snapshot);
    a.battery = snapshot;
    if let Some(ui) = a.ui.upgrade() {
        apply_battery_properties(&ui, snapshot, snapshot.label());
    }
    let now = a.now();
    if a.settings.battery_previews
        && !a.verification
        && let Some(activity) = activity
    {
        a.coordinator.battery(activity, now);
    }
    if !snapshot.present {
        a.coordinator.clear_battery(now);
    }
    drop(a);
    sync_presentation(app);
}

fn time_label(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.) as u64
    } else {
        0
    };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
