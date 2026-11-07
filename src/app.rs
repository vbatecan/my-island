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
    ui: slint::Weak<IslandWindow>,
    state: IslandState,
    springs: [Spring; 5],
    timer: slint::Timer,
    last: Instant,
    settings: Settings,
    hover: bool,
    pending_content: Option<IslandState>,
    pending_compact: Option<IslandState>,
    pending_geometry: Option<[f64; 3]>,
    verification: bool,
    frames: u64,
    volume: Option<Rc<VolumeService>>,
    media: Option<Rc<MediaService>>,
    media_clock: slint::Timer,
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
        let g = state.geometry();
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
        if matches!(
            state,
            IslandState::Expanded
                | IslandState::Media
                | IslandState::Volume
                | IslandState::Notification
        ) {
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
            self.springs[3].snap(
                if matches!(
                    state,
                    IslandState::Expanded
                        | IslandState::Media
                        | IslandState::Volume
                        | IslandState::Notification
                ) {
                    1.0
                } else {
                    0.0
                },
            );
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
    fn apply_pending_presentation(&mut self) {
        if let Some(view) = self.pending_presentation.take() {
            if let Some(ui) = self.ui.upgrade() {
                apply_content_properties(&ui, &view, &self.settings);
                self.rendered_presentation = Some(view);
            }
        }
    }
    fn apply_content(&mut self) {
        self.apply_pending_presentation();
        if let Some(state) = self.pending_content.take() {
            if let Some(ui) = self.ui.upgrade() {
                ui.set_media_mode(state == IslandState::Media);
                ui.set_volume_mode(state == IslandState::Volume);
            }
        }
    }
    fn apply_compact(&mut self) {
        self.apply_pending_presentation();
        if let Some(state) = self.pending_compact.take() {
            if let Some(ui) = self.ui.upgrade() {
                ui.set_volume_mode(state == IslandState::Volume);
            }
        }
    }
    fn tick(&mut self) -> bool {
        if self.state == IslandState::Idle
            && self.springs[3].position.max(self.springs[4].position) < 0.01
        {
            self.apply_pending_presentation();
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
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
        let expanded = self.pending_content.is_none()
            && matches!(
                self.state,
                IslandState::Expanded
                    | IslandState::Media
                    | IslandState::Volume
                    | IslandState::Notification
            );
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
            ui.set_shape_width(self.springs[0].position as f32);
            ui.set_shape_height(self.springs[1].position as f32);
            ui.set_shape_radius(self.springs[2].position as f32);
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
                let image = self.glass.borrow_mut().image(
                    width,
                    height,
                    self.springs[2].position,
                    ui.window().scale_factor(),
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
                }
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
        if let Some(v) = arg.strip_prefix("--appearance=") {
            settings.appearance = Appearance::parse(v);
        }
        if let Some(v) = arg.strip_prefix("--opacity=") {
            if let Ok(v) = v.parse::<f32>() {
                if v.is_finite() {
                    settings.opacity = v.clamp(0.15, 0.95);
                }
            }
        }
        if let Some(v) = arg.strip_prefix("--bounce=") {
            settings.bounce = crate::settings::Bounce::parse(v);
        }
    }
    if std::env::args()
        .any(|a| a == "--verify" || a == "--verify-notifications" || a == "--verify-settings")
    {
        settings.auto_volume = false;
        settings.compact_now_playing = false;
        settings.notifications_enabled = false;
        settings.hover_expand = false;
    }
    if std::env::args().any(|a| a == "--benchmark") {
        settings.notifications_enabled = false;
    }
    let app = Rc::new(RefCell::new(App {
        ui: ui.as_weak(),
        state: IslandState::Idle,
        springs: [
            Spring::new(126.0),
            Spring::new(32.0),
            Spring::new(16.0),
            content_spring(),
            content_spring(),
        ],
        timer: slint::Timer::default(),
        last: Instant::now(),
        settings,
        hover: false,
        pending_content: None,
        pending_compact: None,
        pending_geometry: None,
        verification: std::env::args()
            .any(|a| a == "--verify" || a == "--verify-notifications" || a == "--verify-settings"),
        frames: 0,
        volume: None,
        media: None,
        media_clock: slint::Timer::default(),
        media_position: (0., 0., Instant::now()),
        glass: RefCell::new(Glass::default()),
        capture: None,
        incoming_frame: Arc::new(Mutex::new(None)),
        frame_scheduled: Arc::new(AtomicBool::new(false)),
        preferences: preferences.as_weak(),
        coordinator: PresentationCoordinator::default(),
        presentation: None,
        pending_presentation: None,
        rendered_presentation: None,
        epoch: Instant::now(),
        deadline: slint::Timer::default(),
        hover_timer: slint::Timer::default(),
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
    }));
    if app.borrow().verification {
        let metrics = app.borrow().metrics.clone();
        if let Err(e) = ui
            .window()
            .set_rendering_notifier(move |state, _| metrics.frame(state))
        {
            record(&format!("render metrics unavailable: {e:?}"));
        }
    }
    platform::set_animation_preference(app.borrow().settings.animations);
    app.borrow().render();
    {
        let app = app.clone();
        ui.on_toggle(move || user_toggle(&app));
    }
    {
        let app = app.clone();
        ui.on_collapse(move || transition(&app, IslandState::Idle));
    }
    {
        let app = app.clone();
        ui.on_open_media(move || {
            if let Some(media) = app.borrow().media.clone() {
                media.refresh();
            }
            transition(&app, IslandState::Media);
        });
    }
    {
        let app = app.clone();
        ui.on_open_volume(move || transition(&app, IslandState::Volume));
    }
    {
        let app = app.clone();
        ui.on_hover(move |hover| {
            let (idle, expand, delay) = {
                let mut a = app.borrow_mut();
                a.hover = hover;
                let now = a.now();
                a.coordinator.set_hover(hover, now);
                (
                    a.state == IslandState::Idle,
                    a.settings.hover_expand,
                    a.settings.hover_delay,
                )
            };
            if idle {
                let mut a = app.borrow_mut();
                let state = a.state;
                a.target(state);
                drop(a);
                wake(&app);
            }
            app.borrow().hover_timer.stop();
            if hover && expand {
                let weak = Rc::downgrade(&app);
                app.borrow().hover_timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_millis(delay as u64),
                    move || {
                        if let Some(app) = weak.upgrade() {
                            if app.borrow().hover
                                && !app.borrow().hidden
                                && !app.borrow().settings_visible
                                && !app.borrow().coordinator.panel_open()
                            {
                                transition(&app, IslandState::Expanded);
                            }
                        }
                    },
                );
            }
            sync_presentation(&app);
        });
    }
    // Services notify the Slint event loop; remote COM/WinRT work stays off the GUI thread.
    {
        let weak = ui.as_weak();
        match VolumeService::new(move |event| {
            let v = event.snapshot;
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_volume(v.percent);
                ui.set_volume_muted(v.muted);
                ui.set_volume_available(v.available);
                ui.invoke_volume_updated(event.origin == VolumeOrigin::External);
            });
        }) {
            Ok(service) => {
                if let Ok(v) = service.snapshot() {
                    ui.set_volume(v.percent);
                    ui.set_volume_muted(v.muted);
                    ui.set_volume_available(v.available);
                }
                app.borrow_mut().volume = Some(Rc::new(service));
            }
            Err(e) => ui.set_service_status(e.into()),
        }
    }
    {
        let weak = ui.as_weak();
        match MediaService::new(move |snapshot| {
            let _ = weak.upgrade_in_event_loop(move |ui| {
                apply_media(&ui, &snapshot);
                ui.invoke_media_updated();
            });
        }) {
            Ok(service) => {
                apply_media(&ui, &service.snapshot());
                app.borrow_mut().media = Some(Rc::new(service));
            }
            Err(e) => ui.set_service_status(e.into()),
        }
    }
    {
        let app = app.clone();
        ui.on_volume_change(move |v| {
            let service = app.borrow().volume.clone();
            if let Some(service) = service {
                if let Err(e) = service.set_volume(v) {
                    status(&app, &e);
                }
            }
        });
    }
    {
        let app = app.clone();
        ui.on_toggle_mute(move || {
            let service = app.borrow().volume.clone();
            if let Some(service) = service {
                if let Err(e) = service.toggle_mute() {
                    status(&app, &e);
                }
            }
        });
    }
    for command in 0..3 {
        let app = app.clone();
        let callback = move || {
            let service = app.borrow().media.clone();
            if let Some(service) = service {
                let result = match command {
                    0 => service.play_pause(),
                    1 => service.previous(),
                    _ => service.next(),
                };
                if let Err(e) = result {
                    status(&app, &e);
                }
            }
        };
        match command {
            0 => ui.on_play_pause(callback),
            1 => ui.on_previous(callback),
            2 => ui.on_next(callback),
            _ => {}
        }
    }
    {
        let app = app.clone();
        ui.on_seek(move |fraction| {
            let service = app.borrow().media.clone();
            let duration = app.borrow().media_position.1;
            if let Some(service) = service {
                if let Err(e) = service.seek(fraction as f64 * duration) {
                    status(&app, &e);
                }
            }
        });
    }
    {
        let app = app.clone();
        ui.on_media_updated(move || {
            let ui = app.borrow().ui.upgrade();
            if let Some(ui) = ui {
                app.borrow_mut().media_position = (
                    ui.get_position_seconds() as f64,
                    ui.get_duration_seconds() as f64,
                    Instant::now(),
                );
            }
            {
                let mut a = app.borrow_mut();
                let now = a.now();
                let media = a.settings.compact_now_playing
                    && a.ui
                        .upgrade()
                        .is_some_and(|ui| ui.get_media_available() && ui.get_playing());
                a.coordinator.set_media(media, now);
            }
            sync_media_clock(&app);
            sync_presentation(&app);
        });
    }
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
            Event::Reset => transition(&event_app, IslandState::Idle),
            Event::State(state) => transition(&event_app, state),
            Event::Exit => {
                let _ = slint::quit_event_loop();
            }

            Event::DisplayChanged => {
                configure_material(&event_app);
                refresh_preferences(&event_app);
                if !event_app.borrow().animations_enabled() {
                    let mut a = event_app.borrow_mut();
                    let state = a.state;
                    a.target(state);
                    a.timer.stop();
                }
            }
            Event::BackdropRefresh => refresh_eco_glass(&event_app),
            Event::OpenSettings => show_preferences(&event_app),
            Event::ClosePanel => {
                if !event_app.borrow().settings_visible {
                    transition(&event_app, IslandState::Idle);
                }
            }
            Event::HotkeyToggle => user_toggle(&event_app),
            Event::DesktopPolicyChanged(hidden) => {
                let old = {
                    let mut a = event_app.borrow_mut();
                    a.hidden = hidden;
                    let now = a.now();
                    let suppress = hidden || a.settings_visible;
                    a.coordinator.suppress(suppress, now);
                    if hidden {
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
            }
            Event::ToggleAnimations
            | Event::ToggleTopmost
            | Event::Offset(_)
            | Event::Appearance(_)
            | Event::Opacity(_)
            | Event::Bounce(_) => {
                let mut settings = event_app.borrow().settings.clone();
                match event {
                    Event::ToggleAnimations => settings.animations = !settings.animations,
                    Event::ToggleTopmost => settings.topmost = !settings.topmost,
                    Event::Offset(v) => settings.offset = v,
                    Event::Appearance(v) => settings.appearance = v,
                    Event::Opacity(v) => settings.opacity = v,
                    Event::Bounce(v) => settings.bounce = v,
                    _ => {}
                }
                apply_settings(&event_app, settings, true);
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
            init_app.borrow_mut().attached=true;
            let settings=init_app.borrow().settings.clone();apply_settings(&init_app,settings,false);
            configure_material(&init_app);
            // Deterministic opt-in test runs ignore live pointer input; normal use is unchanged.
            if std::env::args().any(|a| a == "--verify" || a == "--verify-notifications" || a=="--verify-settings" || a == "--benchmark" || a=="--verify-settings") {
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
            }
            init_app.borrow().render();
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
    wire_new_features(&app, &ui, &preferences);
    ui.show()?;
    app.borrow().render();
    if std::env::args().any(|a| a == "--verify-settings") {
        let weak = Rc::downgrade(&app);
        slint::Timer::single_shot(Duration::from_millis(700), move || {
            if let Some(app) = weak.upgrade() {
                show_preferences(&app);
            }
        });
        let weak = Rc::downgrade(&app);
        slint::Timer::single_shot(Duration::from_millis(1800), move || {
            if let Some(app) = weak.upgrade() {
                let ui = app.borrow().preferences.upgrade();
                if let Some(ui) = ui {
                    match crate::verification::snapshot_preferences(&ui) {
                        Ok(path) => record(&format!("settings snapshot {}", path.display())),
                        Err(e) => record(&format!("settings FAILED {e}")),
                    };
                    use slint::winit_030::WinitWindowAccessor;
                    ui.window().with_winit_window(|window|{if let Ok(handle)=window.window_handle(){if let RawWindowHandle::Win32(handle)=handle.as_raw(){let hwnd=windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _);unsafe{record(&format!("settings focused={} noactivate={}",windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow()==hwnd,windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd,windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE) as u32 & windows::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE.0 !=0));}}}});
                }
            }
        });
        let weak = Rc::downgrade(&app);
        slint::Timer::single_shot(Duration::from_millis(2600), move || {
            if let Some(app) = weak.upgrade() {
                close_preferences(&app, true);
            }
        });
    }
    let verification_timer = slint::Timer::default();
    if app.borrow().verification {
        record(&format!("startup {}", platform::diagnostic_snapshot()));
        let app = app.clone();
        let started = Instant::now();
        let mut stage = 0u32;
        verification_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let t = started.elapsed().as_millis();
                let next = if t < 3000 {
                    0
                } else if t < 6000 {
                    1
                } else if t < 9000 {
                    2
                } else if t < 12000 {
                    3
                } else if t < 14000 {
                    4 + ((t - 12000) / 100) as u32
                } else if t < 18000 {
                    24
                } else {
                    25
                };
                if next != stage {
                    stage = next;
                    if matches!(stage, 1..=4 | 25) {
                        if let Some(ui) = app.borrow().ui.upgrade() {
                            let name =
                                if std::env::args().any(|arg| arg == "--verify-notifications") {
                                    match stage {
                                        1 => "notification-before",
                                        2 => "notification-compact",
                                        3 => "notification-expanded",
                                        4 => "notification-protected-media",
                                        _ => "notification-settled",
                                    }
                                } else {
                                    match stage {
                                        1 => "idle",
                                        2 => "expanded",
                                        3 => "media",
                                        4 => "volume",
                                        _ => "settled",
                                    }
                                };
                            match crate::verification::snapshot(&ui, name) {
                                Ok(path) => record(&format!("snapshot {}", path.display())),
                                Err(e) => record(&format!("snapshot FAILED {e}")),
                            }
                        }
                    }
                    if std::env::args().any(|arg| arg == "--verify-notifications") {
                        verification_notification_step(&app, stage);
                        return;
                    }
                    match stage {
                        1 => transition(&app, IslandState::Expanded),
                        2 => transition(&app, IslandState::Media),
                        3 => transition(&app, IslandState::Volume),
                        4..=23 => transition(
                            &app,
                            if stage % 2 == 0 {
                                IslandState::Expanded
                            } else {
                                IslandState::Idle
                            },
                        ),
                        24 => transition(&app, IslandState::Idle),
                        _ => {
                            record(&format!(
                                "complete frames={} active={}",
                                app.borrow().frames,
                                app.borrow().timer.running()
                            ));
                            let _ = slint::quit_event_loop();
                        }
                    }
                }
            },
        );
    }
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

fn record(message: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("verification.log")
    {
        let _ = writeln!(file, "{message}");
    }
}
fn status(app: &Rc<RefCell<App>>, message: &str) {
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_service_status(message.into());
    }
}

fn apply_media(ui: &IslandWindow, snapshot: &MediaSnapshot) {
    ui.set_media_available(snapshot.available);
    ui.set_playing(snapshot.playing);
    ui.set_track_title(snapshot.title.as_str().into());
    ui.set_artist_name(snapshot.artist.as_str().into());
    ui.set_media_source(snapshot.source.as_str().into());
    ui.set_can_play_pause(snapshot.can_play_pause);
    ui.set_can_previous(snapshot.can_previous);
    ui.set_can_next(snapshot.can_next);
    ui.set_can_seek(snapshot.can_seek);
    ui.set_position_seconds(snapshot.position_seconds as f32);
    ui.set_duration_seconds(snapshot.duration_seconds as f32);
    ui.set_progress(if snapshot.duration_seconds > 0. {
        (snapshot.position_seconds / snapshot.duration_seconds).clamp(0., 1.) as f32
    } else {
        0.
    });
    ui.set_elapsed_label(time_label(snapshot.position_seconds).into());
    ui.set_duration_label(time_label(snapshot.duration_seconds).into());
    ui.set_service_status(snapshot.error.as_deref().unwrap_or("").into());
    if let Some(bytes) = &snapshot.artwork {
        match slint::Image::load_from_data(bytes, None) {
            Ok(image) => {
                ui.set_artwork(image);
                ui.set_has_artwork(true);
            }
            Err(_) => ui.set_has_artwork(false),
        }
    } else {
        ui.set_has_artwork(false);
    }
}
fn time_label(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.) as u64
    } else {
        0
    };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn sync_media_clock(app: &Rc<RefCell<App>>) {
    let active = {
        let a = app.borrow();
        !a.hidden
            && a.state == IslandState::Media
            && a.ui
                .upgrade()
                .is_some_and(|ui| ui.get_media_available() && ui.get_playing())
    };
    if !active {
        app.borrow().media_clock.stop();
        return;
    }
    if app.borrow().media_clock.running() {
        return;
    }
    let weak = Rc::downgrade(app);
    app.borrow().media_clock.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            if let Some(app) = weak.upgrade() {
                let a = app.borrow();
                if let Some(ui) = a.ui.upgrade() {
                    let (position, duration, received) = a.media_position;
                    let position =
                        (position + received.elapsed().as_secs_f64()).min(duration.max(0.));
                    ui.set_progress(if duration > 0. {
                        (position / duration).clamp(0., 1.) as f32
                    } else {
                        0.
                    });
                    ui.set_elapsed_label(time_label(position).into());
                }
            }
        },
    );
}
fn configure_material(app: &Rc<RefCell<App>>) {
    if app.borrow().hidden {
        return;
    }
    app.borrow().render();
    let old = app.borrow_mut().capture.take();
    drop(old);
    let _ = platform::exclude_from_capture(false);
    let settings = app.borrow().settings.clone();
    platform::set_animation_preference(settings.animations);
    if let Err(e) = platform::set_appearance(settings.appearance, settings.opacity, settings.bounce)
    {
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
                    if !scheduled.swap(true, Ordering::AcqRel) {
                        if ui
                            .upgrade_in_event_loop(|ui| ui.invoke_backdrop_updated())
                            .is_err()
                        {
                            scheduled.store(false, Ordering::Release);
                        }
                    }
                },
            )
        })();
        match setup {
            Ok(capture) => app.borrow_mut().capture = Some(capture),
            Err(e) => {
                let _ = platform::exclude_from_capture(false);
                app.borrow_mut().settings.appearance = Appearance::Acrylic;
                let _ = platform::set_appearance(
                    Appearance::Acrylic,
                    settings.opacity,
                    settings.bounce,
                );
                status(app, &format!("Glass unavailable; acrylic active: {e}"));
            }
        }
    } else {
        app.borrow().glass.borrow_mut().clear();
    }
    app.borrow().render();
}

fn refresh_eco_glass(app: &Rc<RefCell<App>>) {
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

fn transition(app: &Rc<RefCell<App>>, state: IslandState) {
    {
        let mut a = app.borrow_mut();
        let now = a.now();
        match state {
            IslandState::Idle => a.coordinator.close(now),
            IslandState::Expanded => a.coordinator.open(Panel::Launcher, now),
            IslandState::Media => a.coordinator.open(Panel::Media, now),
            IslandState::Volume => a.coordinator.open(Panel::Volume, now),
            IslandState::Compact => a.coordinator.volume(now),
            IslandState::Notification => a.coordinator.expand_notification(now),
        }
    }
    sync_presentation(app);
}
fn user_toggle(app: &Rc<RefCell<App>>) {
    {
        let mut a = app.borrow_mut();
        let now = a.now();
        if a.coordinator.panel_open() {
            a.coordinator.close(now);
        } else if matches!(a.coordinator.view(now).content, Content::Notification(..)) {
            a.coordinator.expand_notification(now);
        } else {
            a.coordinator.open(Panel::Launcher, now);
        }
    }
    sync_presentation(app);
}
fn apply_content_properties(ui: &IslandWindow, view: &Presentation, settings: &Settings) {
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
        let panel = a.coordinator.panel_open() && !a.settings_visible && !a.hidden;
        let ui = a.ui.upgrade();
        if let Some(ui) = &ui {
            ui.set_pending_count(view.pending as i32);
            ui.set_volume_feedback(view.volume_chip);
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
    if app.borrow().attached {
        if let Err(e) = platform::set_panel_open(panel) {
            settings_status(app, &e);
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
}
fn access_label(access: &AccessStatus) -> String {
    match access{AccessStatus::UnavailablePortable=>"Portable build: install the MSIX package to enable Windows notifications.".into(),AccessStatus::NotRequested=>"Access has not been granted. Choose Enable access to request it.".into(),AccessStatus::Denied=>"Windows denied access. Allow notification access in Windows Settings, then choose Retry.".into(),AccessStatus::Allowed=>"Windows notification access is allowed. Previews never clear Notification Center.".into(),AccessStatus::Error(e)=>format!("Notification access unavailable: {e}")}
}
fn settings_status(app: &Rc<RefCell<App>>, message: &str) {
    if let Some(ui) = app.borrow().preferences.upgrade() {
        ui.set_status_text(message.into());
    }
}
fn refresh_preferences(app: &Rc<RefCell<App>>) {
    let (ui, s, apps, access) = {
        let mut a = app.borrow_mut();
        if a.settings_sync {
            return;
        }
        a.settings_sync = true;
        (
            a.preferences.upgrade(),
            a.settings.clone(),
            a.observed_apps.clone(),
            a.notification_access.clone(),
        )
    };
    if let Some(ui) = ui {
        crate::preferences::populate(
            &ui,
            &s,
            &platform::monitors(),
            &apps,
            &access_label(&access),
        );
        ui.set_notification_access_requestable(!matches!(
            access,
            AccessStatus::UnavailablePortable
        ));
    }
    app.borrow_mut().settings_sync = false;
}
fn apply_settings(app: &Rc<RefCell<App>>, settings: Settings, persist: bool) {
    let (old, attached, service) = {
        let mut a = app.borrow_mut();
        let old = a.settings.clone();
        a.settings = settings.clone();
        let now = a.now();
        a.coordinator.set_dwell(settings.notification_duration, now);
        a.coordinator.retain_apps(&settings.blocked_apps, now);
        if !settings.notifications_enabled || settings.notifications_paused {
            a.coordinator.clear_notifications(now);
        }
        let media = settings.compact_now_playing
            && a.ui
                .upgrade()
                .is_some_and(|ui| ui.get_media_available() && ui.get_playing());
        a.coordinator.set_media(media, now);
        (old, a.attached, a.notifications.clone())
    };
    if let Some(service) = service {
        service.set_enabled(settings.notifications_enabled && !settings.notifications_paused);
    }
    if attached {
        if let Err(e) = platform::configure_desktop(
            &settings.monitor_id,
            settings.fullscreen_hide,
            settings.outside_click,
            settings.shortcut_enabled,
        ) {
            settings_status(app, &e);
            if e.contains("Ctrl+Alt+Space") {
                app.borrow_mut().settings.shortcut_enabled = false;
                let _ = platform::configure_desktop(
                    &settings.monitor_id,
                    settings.fullscreen_hide,
                    settings.outside_click,
                    false,
                );
            }
        }
        platform::set_animation_preference(app.borrow().animations_enabled());
        if old.appearance != settings.appearance
            || old.offset != settings.offset
            || old.monitor_id != settings.monitor_id
        {
            configure_material(app);
        } else {
            let _ =
                platform::set_appearance(settings.appearance, settings.opacity, settings.bounce);
        }
    }
    {
        let mut a = app.borrow_mut();
        if !a.animations_enabled() {
            let state = a.state;
            a.target(state);
            a.timer.stop();
        }
        a.render();
    }
    if old.privacy != settings.privacy {
        let a = app.borrow();
        if let (Some(ui), Some(view)) = (a.ui.upgrade(), a.rendered_presentation.as_ref()) {
            apply_content_properties(&ui, view, &a.settings);
        }
    }
    if persist {
        if let Err(e) = app.borrow().settings.save() {
            settings_status(app, &format!("Preferences could not be saved: {e}"));
        }
    }
    sync_presentation(app);
    refresh_preferences(app);
}
fn show_preferences(app: &Rc<RefCell<App>>) {
    refresh_preferences(app);
    let ui = app.borrow().preferences.upgrade();
    if let Some(ui) = ui {
        {
            let mut a = app.borrow_mut();
            a.settings_visible = true;
            let now = a.now();
            a.coordinator.suppress(true, now);
        }
        sync_presentation(app);
        if let Err(e) = ui.show() {
            settings_status(app, &e.to_string());
            return;
        }
        let weak = ui.as_weak();
        let _ = slint::spawn_local(async move {
            if let Some(ui) = weak.upgrade() {
                use slint::winit_030::{
                    WinitWindowAccessor,
                    winit::{platform::windows::WindowExtWindows, window::WindowLevel},
                };
                if let Ok(window) = ui.window().winit_window().await {
                    window.set_skip_taskbar(false);
                    window.set_window_level(WindowLevel::Normal);
                    window.focus_window();
                }
            }
        });
    }
}
fn close_preferences(app: &Rc<RefCell<App>>, hide: bool) {
    let ui = app.borrow().preferences.upgrade();
    if hide {
        if let Some(ui) = ui {
            let _ = ui.hide();
        }
    }
    {
        let mut a = app.borrow_mut();
        a.settings_visible = false;
        let now = a.now();
        let hidden = a.hidden;
        a.coordinator.suppress(hidden, now);
    }
    sync_presentation(app);
}
fn wire_new_features(app: &Rc<RefCell<App>>, ui: &IslandWindow, prefs: &SettingsWindow) {
    {
        let app = app.clone();
        ui.on_volume_updated(move |external| {
            if external && app.borrow().settings.auto_volume {
                let mut a = app.borrow_mut();
                let now = a.now();
                a.coordinator.volume(now);
            }
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_pointer_interaction(move |pressed| {
            let mut a = app.borrow_mut();
            let now = a.now();
            let pause = pressed || a.hover;
            a.coordinator.set_hover(pause, now);
            drop(a);
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_expand_notification(move || {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.expand_notification(now);
            drop(a);
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_hide_notification(move || {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.hide_notification(now);
            drop(a);
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_open_source(move || {
            let record =
                app.borrow()
                    .presentation
                    .as_ref()
                    .and_then(|view| match &view.content {
                        Content::Notification(record, _)
                        | Content::ExpandedNotification(record) => Some(record.clone()),
                        _ => None,
                    });
            if let Some(record) = record {
                if let Err(e) = notifications::open_source_app(&record.app_id) {
                    status(&app, &e);
                } else {
                    transition(&app, IslandState::Idle);
                }
            }
        });
    }
    {
        let app = app.clone();
        ui.on_settings_requested(move || show_preferences(&app));
    }
    {
        let app = app.clone();
        ui.on_escape(move || transition(&app, IslandState::Idle));
    }
    {
        let app = app.clone();
        prefs.on_changed(move || {
            if app.borrow().settings_sync {
                return;
            }
            let ui = app.borrow().preferences.upgrade();
            if let Some(ui) = ui {
                let settings =
                    crate::preferences::read(&ui, &app.borrow().settings, &platform::monitors());
                apply_settings(&app, settings, true);
            }
        });
    }
    {
        let app = app.clone();
        prefs.on_reset_defaults(move || apply_settings(&app, Settings::default(), true));
    }
    {
        let app = app.clone();
        prefs.on_app_filter_changed(move |id, enabled| {
            let mut s = app.borrow().settings.clone();
            s.blocked_apps.retain(|old| old != id.as_str());
            if !enabled {
                s.blocked_apps.push(id.to_string());
            }
            apply_settings(&app, s, true);
        });
    }
    {
        let app = app.clone();
        prefs.on_close_settings(move || close_preferences(&app, true));
    }
    {
        let app = app.clone();
        prefs.window().on_close_requested(move || {
            close_preferences(&app, false);
            slint::CloseRequestResponse::HideWindow
        });
    }
    {
        let app = app.clone();
        prefs.on_preview_motion(move || {
            transition(&app, IslandState::Expanded);
            let weak = Rc::downgrade(&app);
            slint::Timer::single_shot(Duration::from_millis(900), move || {
                if let Some(app) = weak.upgrade() {
                    transition(&app, IslandState::Idle);
                }
            });
        });
    }
    {
        let app = app.clone();
        prefs.on_retry_notifications(move || {
            if let Some(service) = app.borrow().notifications.clone() {
                service.refresh_access();
            }
            refresh_preferences(&app);
        });
    }
    {
        let app = app.clone();
        prefs.on_enable_notifications(move || {
            let app = app.clone();
            let _ = slint::spawn_local(async move {
                match NotificationService::request_access().await {
                    Ok(access) => {
                        app.borrow_mut().notification_access = access.clone();
                        if access == AccessStatus::Allowed {
                            let mut s = app.borrow().settings.clone();
                            s.notifications_enabled = true;
                            apply_settings(&app, s, true);
                        }
                        if let Some(service) = app.borrow().notifications.clone() {
                            service.refresh_access();
                        }
                        refresh_preferences(&app);
                    }
                    Err(e) => settings_status(&app, &e),
                }
            });
        });
    }
    {
        let app = app.clone();
        ui.on_notifications_updated(move || {
            let events = {
                let a = app.borrow();
                let mut queue = a
                    .notification_inbox
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                a.notification_scheduled.store(false, Ordering::Release);
                queue.drain(..).collect::<Vec<_>>()
            };
            for event in events {
                let mut a = app.borrow_mut();
                let now = a.now();
                match event {
                    NotificationEvent::Access(access) => {
                        a.notification_access = access.clone();
                        if access != AccessStatus::Allowed {
                            a.coordinator.clear_notifications(now);
                        }
                    }
                    NotificationEvent::Removed(id) => a.coordinator.remove(id, now),
                    NotificationEvent::Added(record) | NotificationEvent::Updated(record) => {
                        if !a.observed_apps.iter().any(|(id, _)| id == &record.app_id) {
                            a.observed_apps
                                .push((record.app_id.clone(), record.app_name.clone()));
                        }
                        if a.settings.notifications_enabled
                            && !a.settings.notifications_paused
                            && !a.settings.blocked_apps.contains(&record.app_id)
                            && platform::notification_allowed()
                        {
                            a.coordinator.push(record, now);
                        }
                    }
                }
            }
            sync_presentation(&app);
            refresh_preferences(&app);
        });
    }
    let (incoming, scheduled) = {
        let a = app.borrow();
        (
            a.notification_inbox.clone(),
            a.notification_scheduled.clone(),
        )
    };
    let weak = ui.as_weak();
    match NotificationService::new(move |event| {
        let mut queue = incoming.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= 128 {
            queue.pop_front();
        }
        queue.push_back(event);
        drop(queue);
        if !scheduled.swap(true, Ordering::AcqRel) {
            if weak
                .upgrade_in_event_loop(|ui| ui.invoke_notifications_updated())
                .is_err()
            {
                scheduled.store(false, Ordering::Release);
            }
        }
    }) {
        Ok(service) => {
            let mut a = app.borrow_mut();
            a.notification_access = service.access_status();
            service
                .set_enabled(a.settings.notifications_enabled && !a.settings.notifications_paused);
            a.notifications = Some(Rc::new(service));
        }
        Err(e) => app.borrow_mut().notification_access = AccessStatus::Error(e),
    }
    refresh_preferences(app);
}

fn verification_notification_step(app: &Rc<RefCell<App>>, stage: u32) {
    let fixture = |id| NotificationRecord {
        id,
        app_id: "StillIsland.SilentFixture".into(),
        app_name: "Integration fixture".into(),
        title: format!("Notification {id}"),
        body: "A synthetic preview. Hiding this never removes a Windows notification.".into(),
        created_ticks: 0,
        icon: None,
    };
    match stage {
        1 => {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.push(fixture(1), now);
        }
        2 => {
            let ui = app.borrow().ui.upgrade();
            if let Some(ui) = ui {
                ui.invoke_expand_notification();
            }
        }
        3 => {
            transition(app, IslandState::Media);
            let mut a = app.borrow_mut();
            let now = a.now();
            for id in 2..32 {
                a.coordinator.push(fixture(id), now);
            }
            record("notification fixture queued while user Media panel remains pinned");
        }
        4 => {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.close(now);
            a.coordinator.volume(now);
        }
        24 => {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.clear_notifications(now);
            a.coordinator.close(now);
        }
        25 => {
            record(&format!(
                "complete frames={} active={}",
                app.borrow().frames,
                app.borrow().timer.running()
            ));
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
    sync_presentation(app);
}
