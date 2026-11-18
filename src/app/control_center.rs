//! Control Center owns the lifetime of on-demand device reads and confirmed actions.
use super::*;
use crate::{
    ControlAction, DeviceEntry, RadioControl,
    platform::{
        brightness,
        control_devices::{self, Kind},
        hotspot,
        power::Action as PowerAction,
    },
    services::volume::VolumeAction,
};
use slint::Model;
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Menu {
    #[default]
    Controls,
    Power,
    ConfirmRestart,
    ConfirmShutDown,
    WifiDevices,
    BluetoothDevices,
}

impl Menu {
    fn confirmation(self) -> Option<PowerAction> {
        match self {
            Self::ConfirmRestart => Some(PowerAction::Restart),
            Self::ConfirmShutDown => Some(PowerAction::ShutDown),
            _ => None,
        }
    }
    fn device_kind(self) -> Option<Kind> {
        match self {
            Self::WifiDevices => Some(Kind::Wifi),
            Self::BluetoothDevices => Some(Kind::Bluetooth),
            _ => None,
        }
    }
}

#[derive(Default)]
struct DeviceRequests {
    view: Option<Arc<AtomicBool>>,
    pending: [bool; 2],
    status_delay: slint::Timer,
}

impl DeviceRequests {
    fn clear(&mut self, ui: &IslandWindow) {
        invalidate(&mut self.view);
        self.status_delay.stop();
        ui.set_control_devices(Default::default());
        ui.set_control_devices_busy(false);
        ui.set_control_device_detail("".into());
    }
}

#[derive(Default)]
struct LevelRequests {
    queued: Option<i32>,
    writing: Option<i32>,
    busy: bool,
}

impl LevelRequests {
    fn target(&self, reported: i32) -> i32 {
        self.queued.or(self.writing).unwrap_or(reported)
    }

    fn clear(&mut self) {
        self.queued = None;
        self.writing = None;
    }
}

#[derive(Default)]
pub(super) struct Controller {
    session: Option<Arc<AtomicBool>>,
    display: Option<Arc<AtomicBool>>,
    refresh: slint::Timer,
    brightness: Option<brightness::Service>,
    brightness_requests: LevelRequests,
    volume_requests: LevelRequests,
    brightness_wheel: crate::gestures::WheelVolume,
    muting: bool,
    radios: [Option<Radio>; 2],
    radio_refreshing: bool,
    radio_busy: Option<usize>,
    radio_access: Option<RadioAccessStatus>,
    hotspot_request: Option<Rc<()>>,
    hotspot_busy: bool,
    previews_busy: bool,
    menu: Menu,
    devices: DeviceRequests,
    power_busy: bool,
    pub(super) verified_power: Option<PowerAction>,
    pub(super) verified_levels: [Option<i32>; 2],
    pub(super) verified_action: Option<ControlAction>,
    pub(super) verified_device: Option<String>,
    pub(super) verification_width: Option<f64>,
}

impl Controller {
    pub(super) fn has_device_view(&self) -> bool {
        self.menu.device_kind().is_some()
    }
}

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    let weak = Rc::downgrade(app);
    ui.on_control_action(move |action| {
        if let Some(app) = weak.upgrade() {
            activate(&app, action);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_control_device_action(move |id| {
        if let Some(app) = weak.upgrade() {
            request_devices(&app, Some(id.as_str().to_owned()));
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_control_brightness_change(move |value| {
        if let Some(app) = weak.upgrade() {
            set_brightness(&app, value);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_control_volume(move |value| {
        if let Some(app) = weak.upgrade() {
            set_volume(&app, value);
        }
    });
}

fn current(session: &Arc<AtomicBool>) -> bool {
    session.load(Ordering::Acquire)
}

fn invalidate(slot: &mut Option<Arc<AtomicBool>>) {
    if let Some(session) = slot.take() {
        session.store(false, Ordering::Release);
    }
}

pub(super) fn sync(app: &Rc<RefCell<App>>, visible: bool) {
    let (ui, started, verification) = {
        let mut a = app.borrow_mut();
        let Some(ui) = a.ui.upgrade() else { return };
        let c = &mut a.control_center;
        match (visible, c.session.is_some()) {
            (true, false) => {
                c.session = Some(Arc::new(AtomicBool::new(true)));
                c.display = Some(Arc::new(AtomicBool::new(true)));
                (ui, true, a.verification)
            }
            (true, true) => (ui, false, a.verification),
            (false, true) => {
                invalidate(&mut c.session);
                invalidate(&mut c.display);
                c.refresh.stop();
                c.menu = Menu::Controls;
                c.devices.clear(&ui);
                c.brightness_requests.clear();
                c.volume_requests.clear();
                c.brightness_wheel.reset();
                c.brightness = None;
                c.radios = Default::default();
                ui.set_control_menu(0);
                ui.set_control_device_progress(0.);
                ui.set_control_error("".into());
                ui.set_control_brightness(-1);
                ui.set_control_brightness_checked(false);
                ui.set_control_wifi(RadioControl::default());
                ui.set_control_bluetooth(RadioControl::default());
                ui.set_control_hotspot(RadioControl::default());
                ui.set_control_hotspot_detail("".into());
                return;
            }
            (false, false) => return,
        }
    };
    let a = app.borrow();
    let on = a.settings.notifications_enabled
        && !a.settings.notifications_paused
        && a.notification_access == AccessStatus::Allowed;
    let available = matches!(
        a.notification_access,
        AccessStatus::Allowed | AccessStatus::NotRequested
    ) && !a.control_center.previews_busy;
    let detail = match a.notification_access {
        AccessStatus::Allowed if on => "On",
        AccessStatus::Allowed if a.settings.notifications_enabled => "Paused",
        AccessStatus::Allowed => "Off",
        AccessStatus::NotRequested => "Allow access",
        AccessStatus::Denied => "Access denied",
        AccessStatus::UnavailablePortable => "Needs MSIX",
        AccessStatus::Error(_) => "Unavailable",
    };
    ui.set_control_previews_available(available);
    ui.set_control_previews_on(on);
    ui.set_control_previews_detail(detail.into());
    drop(a);
    if started && (!verification || verification::has_arg("--verify-control-center-devices")) {
        refresh(app);
        let weak = Rc::downgrade(app);
        app.borrow().control_center.refresh.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(3),
            move || {
                if let Some(app) = weak.upgrade() {
                    refresh(&app);
                }
            },
        );
    }
}

fn refresh(app: &Rc<RefCell<App>>) {
    brightness_request(app);
    refresh_radios(app);
    request_hotspot(app, None);
}

pub(super) fn refresh_display(app: &Rc<RefCell<App>>) {
    {
        let mut a = app.borrow_mut();
        if a.verification || a.control_center.display.is_none() {
            return;
        }
        invalidate(&mut a.control_center.display);
        a.control_center.display = Some(Arc::new(AtomicBool::new(true)));
        a.control_center.brightness_requests.clear();
        a.control_center.brightness_wheel.reset();
        if let Some(ui) = a.ui.upgrade() {
            ui.set_control_brightness_checked(false);
            ui.set_control_brightness(-1);
        }
    }
    brightness_request(app);
}

fn brightness_request(app: &Rc<RefCell<App>>) {
    let (service, display, value, monitor) = {
        let mut a = app.borrow_mut();
        let Some(display) = a.control_center.display.clone() else {
            return;
        };
        let c = &mut a.control_center;
        if c.brightness_requests.busy {
            return;
        }
        if c.brightness.is_none() {
            match brightness::Service::start() {
                Ok(service) => c.brightness = Some(service),
                Err(error) => {
                    record(&format!("Brightness service: {error}"));
                    if let Some(ui) = a.ui.upgrade() {
                        ui.set_control_brightness_checked(true);
                    }
                    return;
                }
            }
        }
        c.brightness_requests.busy = true;
        c.brightness_requests.writing = c.brightness_requests.queued.take();
        (
            c.brightness.clone().unwrap(),
            display,
            c.brightness_requests.writing,
            platform::selected_monitor_handle(),
        )
    };
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        let result = service.request(monitor, value, display.clone()).await;
        let Some(app) = weak.upgrade() else { return };
        let follow_up = {
            let mut a = app.borrow_mut();
            let c = &mut a.control_center;
            c.brightness_requests.busy = false;
            c.brightness_requests.writing = None;
            c.display.is_some() && (!current(&display) || c.brightness_requests.queued.is_some())
        };
        if current(&display) {
            if let Some(ui) = app.borrow().ui.upgrade() {
                ui.set_control_brightness_checked(true);
                if let Ok(level) = &result {
                    ui.set_control_brightness(level.unwrap_or(-1));
                } else if value.is_none() {
                    ui.set_control_brightness(-1);
                }
            }
            if let Err(error) = result {
                if value.is_some() {
                    show_error(&app, "Brightness couldn’t be changed.");
                }
                record(&format!("Brightness service: {error}"));
            }
        }
        if follow_up {
            brightness_request(&app);
        }
    });
}

fn set_brightness(app: &Rc<RefCell<App>>, value: i32) {
    {
        let mut a = app.borrow_mut();
        let Some(ui) = a.ui.upgrade() else { return };
        let reported = ui.get_control_brightness();
        let value = value.clamp(0, 100);
        if a.control_center.session.is_none()
            || reported < 0
            || a.pending_page.is_some()
            || a.control_center.brightness_requests.target(reported) == value
        {
            return;
        }
        if a.verification {
            a.control_center.verified_levels[1] = Some(value);
            return;
        }
        a.control_center.brightness_requests.queued = Some(value);
    }
    show_error(app, "");
    brightness_request(app);
}

fn set_volume(app: &Rc<RefCell<App>>, value: i32) {
    {
        let mut a = app.borrow_mut();
        let Some(ui) = a.ui.upgrade() else { return };
        let value = value.clamp(0, 100);
        if a.control_center.session.is_none()
            || !ui.get_volume_available()
            || a.pending_page.is_some()
            || a.control_center.volume_requests.target(ui.get_volume()) == value
        {
            return;
        }
        if a.verification {
            a.control_center.verified_levels[0] = Some(value);
            return;
        }
        if a.volume.is_none() {
            return;
        }
        a.control_center.volume_requests.queued = Some(value);
    }
    show_error(app, "");
    pump_volume(app);
}

fn pump_volume(app: &Rc<RefCell<App>>) {
    let (service, session, value) = {
        let mut a = app.borrow_mut();
        let Some(session) = a.control_center.session.clone() else {
            return;
        };
        let Some(service) = a.volume.clone() else {
            return;
        };
        let c = &mut a.control_center;
        if c.volume_requests.busy {
            return;
        }
        let Some(value) = c.volume_requests.queued.take() else {
            return;
        };
        c.volume_requests.busy = true;
        c.volume_requests.writing = Some(value);
        (service, session, value)
    };
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        let result = if current(&session) {
            service.apply_async(VolumeAction::Set(value)).await
        } else {
            Ok(())
        };
        let Some(app) = weak.upgrade() else { return };
        {
            let mut a = app.borrow_mut();
            a.control_center.volume_requests.busy = false;
            a.control_center.volume_requests.writing = None;
        }
        if current(&session)
            && let Err(error) = result
        {
            record(&error);
            show_error(&app, "Volume couldn’t be changed.");
        }
        pump_volume(&app);
    });
}

fn mute(app: &Rc<RefCell<App>>, session: Arc<AtomicBool>) {
    let service = {
        let mut a = app.borrow_mut();
        if a.control_center.muting || a.verification {
            return;
        }
        let Some(service) = a.volume.clone() else {
            return;
        };
        a.control_center.muting = true;
        service
    };
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        let result = if current(&session) {
            service.apply_async(VolumeAction::ToggleMute).await
        } else {
            Ok(())
        };
        let Some(app) = weak.upgrade() else { return };
        app.borrow_mut().control_center.muting = false;
        if current(&session)
            && let Err(error) = result
        {
            record(&error);
            show_error(&app, "Mute couldn’t be changed.");
        }
    });
}

pub(super) fn wheel(app: &Rc<RefCell<App>>, fader: i32, delta: f64) {
    let (change, value) = {
        let mut a = app.borrow_mut();
        let Some(ui) = a.ui.upgrade() else { return };
        let step = a.settings.volume_step;
        a.page_wheel.reset();
        if fader == 1 {
            if !a.settings.scroll_volume {
                return;
            }
            let value = a.control_center.volume_requests.target(ui.get_volume());
            (a.wheel.advance(delta, step), value)
        } else {
            let c = &mut a.control_center;
            let value = c.brightness_requests.target(ui.get_control_brightness());
            (c.brightness_wheel.advance(delta, 5), value)
        }
    };
    if change == 0 || value < 0 {
        return;
    }
    if fader == 1 {
        set_volume(app, value + change);
    } else {
        set_brightness(app, value + change);
    }
}

fn refresh_radios(app: &Rc<RefCell<App>>) {
    let session = {
        let mut a = app.borrow_mut();
        let c = &mut a.control_center;
        let Some(session) = c.session.clone() else {
            return;
        };
        if c.radio_refreshing || c.radio_busy.is_some() {
            return;
        }
        c.radio_refreshing = true;
        session
    };
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        let result = discover_radios().await;
        let Some(app) = weak.upgrade() else { return };
        {
            let mut a = app.borrow_mut();
            let c = &mut a.control_center;
            c.radio_refreshing = false;
            if current(&session) && c.radio_busy.is_none() {
                c.radios = result.unwrap_or_default();
            }
        }
        publish_radios(&app);
        if !current(&session) {
            refresh_radios(&app);
        }
    });
}

async fn discover_radios() -> windows::core::Result<[Option<Radio>; 2]> {
    let mut radios = [None, None];
    let mut ambiguous = [false, false];
    let devices = Radio::GetRadiosAsync()?.await?;
    for i in 0..devices.Size()? {
        let radio = devices.GetAt(i)?;
        let index = match radio.Kind()? {
            RadioKind::WiFi => 0,
            RadioKind::Bluetooth => 1,
            _ => continue,
        };
        if radios[index].is_some() {
            ambiguous[index] = true;
        }
        radios[index] = Some(radio);
    }
    for index in 0..radios.len() {
        if ambiguous[index] {
            radios[index] = None;
        }
    }
    Ok(radios)
}

fn publish_radios(app: &Rc<RefCell<App>>) {
    let a = app.borrow();
    let c = &a.control_center;
    let Some(ui) = a.ui.upgrade() else { return };
    if c.session.is_none() {
        return;
    }
    let previous = c
        .menu
        .device_kind()
        .map(|kind| (kind, device_radio(&ui, kind)));
    let can_write = c
        .radio_access
        .is_none_or(|access| access == RadioAccessStatus::Allowed);
    let radio = |index: usize| {
        let state = c.radios[index]
            .as_ref()
            .and_then(|radio| radio.State().ok());
        let known = matches!(state, Some(RadioState::On | RadioState::Off));
        RadioControl {
            known,
            on: state == Some(RadioState::On),
            switchable: known && can_write,
            busy: c.radio_busy.is_some(),
        }
    };
    ui.set_control_wifi(radio(0));
    ui.set_control_bluetooth(radio(1));
    drop(a);
    if let Some((kind, before)) = previous {
        let after = device_radio(&ui, kind);
        if after.known && (!before.known || after.on != before.on) {
            request_devices(app, None);
        }
    }
}

fn device_radio(ui: &IslandWindow, kind: Kind) -> RadioControl {
    match kind {
        Kind::Wifi => ui.get_control_wifi(),
        Kind::Bluetooth => ui.get_control_bluetooth(),
    }
}

fn publish_device_off(ui: &IslandWindow, kind: Kind) -> bool {
    let radio = device_radio(ui, kind);
    if !radio.known || radio.on {
        return false;
    }
    ui.set_control_devices(Default::default());
    ui.set_control_devices_busy(false);
    ui.set_control_device_detail(
        match kind {
            Kind::Wifi => "Wi-Fi is off.",
            Kind::Bluetooth => "Bluetooth is off.",
        }
        .into(),
    );
    true
}

fn request_devices(app: &Rc<RefCell<App>>, id: Option<String>) {
    let weak = Rc::downgrade(app);
    let (kind, index, view) = {
        let mut a = app.borrow_mut();
        if a.pending_page.is_some() {
            return;
        }
        let Some(kind) = a.control_center.menu.device_kind() else {
            return;
        };
        let Some(ui) = a.ui.upgrade() else { return };
        if let Some(id) = &id {
            if ui.get_control_devices_busy()
                || !ui
                    .get_control_devices()
                    .iter()
                    .any(|device| device.enabled && device.id.as_str() == id)
            {
                return;
            }
            if a.verification {
                a.control_center.verified_device = Some(id.clone());
                return;
            }
        } else if a.verification && !verification::has_arg("--verify-control-center-devices") {
            return;
        }
        let requests = &mut a.control_center.devices;
        let Some(view) = requests.view.clone() else {
            return;
        };
        if publish_device_off(&ui, kind) {
            requests.status_delay.stop();
            return;
        }
        if ui.get_control_devices_busy() {
            return;
        }
        let index = match kind {
            Kind::Wifi => 0,
            Kind::Bluetooth => 1,
        };
        let message = match (kind, id.is_some()) {
            (Kind::Wifi, true) => "Connecting…",
            (Kind::Bluetooth, true) => "Pairing…",
            (_, false) => "Searching…",
        };
        ui.set_control_devices_busy(true);
        ui.set_control_device_detail("".into());
        let delayed_app = weak.clone();
        let delayed_view = view.clone();
        requests.status_delay.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(350),
            move || {
                let Some(app) = delayed_app.upgrade() else {
                    return;
                };
                let a = app.borrow();
                if current(&delayed_view)
                    && a.pending_page.is_none()
                    && let Some(ui) = a.ui.upgrade()
                {
                    ui.set_control_device_detail(message.into());
                }
            },
        );
        // A scan or action for a previous opening may still be unwinding.
        if std::mem::replace(&mut requests.pending[index], true) {
            return;
        }
        (kind, index, view)
    };
    let _ = slint::spawn_local(async move {
        let action = match id {
            Some(id) if current(&view) => control_devices::activate(kind, &id, view.clone()).await,
            _ => Ok(()),
        };
        let result = match action {
            Err(message) => Err(message),
            Ok(()) if current(&view) => control_devices::list(kind, true).await,
            Ok(()) => Ok(Vec::new()),
        };
        let Some(app) = weak.upgrade() else { return };
        let mut a = app.borrow_mut();
        let requests = &mut a.control_center.devices;
        requests.pending[index] = false;
        if !current(&view) {
            let reopen = a.control_center.menu.device_kind() == Some(kind);
            if reopen && let Some(ui) = a.ui.upgrade() {
                ui.set_control_devices_busy(false);
            }
            drop(a);
            if reopen {
                request_devices(&app, None);
            }
            return;
        }
        requests.status_delay.stop();
        if let Some(ui) = a.ui.upgrade() {
            ui.set_control_devices_busy(false);
            if a.pending_page.is_some() || publish_device_off(&ui, kind) {
                return;
            }
            match result {
                Ok(devices) => {
                    ui.set_control_device_detail(
                        if devices.is_empty() {
                            match kind {
                                Kind::Wifi => "No networks found.",
                                Kind::Bluetooth => "No devices found.",
                            }
                        } else {
                            ""
                        }
                        .into(),
                    );
                    ui.set_control_devices(slint::ModelRc::new(slint::VecModel::from(
                        devices
                            .into_iter()
                            .map(|device| DeviceEntry {
                                id: device.id.into(),
                                label: device.label.into(),
                                detail: device.detail.into(),
                                connected: device.connected,
                                enabled: device.enabled,
                                signal: device.signal.map_or(-1, |value| value as i32),
                            })
                            .collect::<Vec<_>>(),
                    )));
                }
                Err(message) => ui.set_control_device_detail(message.into()),
            }
        }
    });
}

fn toggle_radio(app: &Rc<RefCell<App>>, index: usize, session: Arc<AtomicBool>) {
    let (radio, cached, target) = {
        let mut a = app.borrow_mut();
        if a.verification || a.control_center.radio_busy.is_some() {
            return;
        }
        let c = &mut a.control_center;
        let Some(radio) = c.radios[index].clone() else {
            return;
        };
        let target = match radio.State() {
            Ok(RadioState::On) => RadioState::Off,
            Ok(RadioState::Off) => RadioState::On,
            _ => return,
        };
        c.radio_busy = Some(index);
        (radio, c.radio_access, target)
    };
    publish_radios(app);
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        // This begins on the Slint thread so Windows may show its consent UI.
        let access = match cached {
            Some(access) => Ok(access),
            None => match Radio::RequestAccessAsync() {
                Ok(operation) => operation.await,
                Err(error) => Err(error),
            },
        };
        let allowed = access
            .as_ref()
            .is_ok_and(|access| *access == RadioAccessStatus::Allowed);
        let changed = allowed && current(&session) && change_radio(&radio, target, &session).await;
        let Some(app) = weak.upgrade() else { return };
        {
            let mut a = app.borrow_mut();
            let c = &mut a.control_center;
            c.radio_busy = None;
            if let Ok(access) = access {
                c.radio_access = Some(access);
            }
        }
        if current(&session) && !changed {
            show_error(
                &app,
                if index == 0 {
                    "Windows couldn’t change Wi-Fi."
                } else {
                    "Windows couldn’t change Bluetooth."
                },
            );
        }
        publish_radios(&app);
        refresh_radios(&app);
        if index == 0 {
            request_hotspot(&app, None);
        }
    });
}

async fn change_radio(radio: &Radio, target: RadioState, session: &Arc<AtomicBool>) -> bool {
    let Ok(operation) = radio.SetStateAsync(target) else {
        return false;
    };
    if operation.await.ok() != Some(RadioAccessStatus::Allowed) {
        return false;
    }
    for _ in 0..10 {
        if !current(session) {
            return false;
        }
        if radio.State().ok() == Some(target) {
            return true;
        }
        let (signal, wait) = futures_channel::oneshot::channel();
        slint::Timer::single_shot(Duration::from_millis(150), move || {
            let _ = signal.send(());
        });
        let _ = wait.await;
    }
    radio.State().ok() == Some(target)
}

fn activate(app: &Rc<RefCell<App>>, action: ControlAction) {
    use ControlAction::*;
    let (session, menu) = {
        let mut a = app.borrow_mut();
        let Some(session) = a.control_center.session.clone() else {
            return;
        };
        if a.pending_page.is_some() {
            return;
        }
        if a.verification {
            a.control_center.verified_action = Some(action);
        }
        (session, a.control_center.menu)
    };
    show_error(app, "");
    match action {
        Back => {
            back(app);
        }
        Lock if matches!(menu, Menu::Controls | Menu::Power) => {
            request_power(app, PowerAction::Lock, session)
        }
        Sleep if matches!(menu, Menu::Controls | Menu::Power) => {
            request_power(app, PowerAction::Sleep, session)
        }
        Restart if menu == Menu::Power => set_menu(app, Menu::ConfirmRestart),
        ShutDown if menu == Menu::Power => set_menu(app, Menu::ConfirmShutDown),
        Confirm => {
            if let Some(action) = menu.confirmation() {
                request_power(app, action, session);
            }
        }
        RefreshDevices if menu.device_kind().is_some() => request_devices(app, None),
        _ if menu != Menu::Controls => {}
        Power => set_menu(app, Menu::Power),
        Mute => mute(app, session),
        Wifi => toggle_radio(app, 0, session),
        Bluetooth => toggle_radio(app, 1, session),
        WifiDevices | BluetoothDevices => {
            set_menu(
                app,
                if action == WifiDevices {
                    Menu::WifiDevices
                } else {
                    Menu::BluetoothDevices
                },
            );
            request_devices(app, None);
        }
        Notifications => toggle_previews(app, session),
        Hotspot => toggle_hotspot(app),
        _ => {}
    }
}

fn set_previews(app: &Rc<RefCell<App>>, paused: bool) {
    let mut settings = app.borrow().settings.clone();
    settings.notifications_paused = paused;
    settings.notifications_enabled = true;
    let persist = !app.borrow().verification;
    apply_settings(app, settings, persist);
}

fn toggle_previews(app: &Rc<RefCell<App>>, session: Arc<AtomicBool>) {
    let (access, enabled, paused) = {
        let a = app.borrow();
        (
            a.notification_access.clone(),
            a.settings.notifications_enabled,
            a.settings.notifications_paused,
        )
    };
    if access == AccessStatus::Allowed {
        set_previews(app, enabled && !paused);
        return;
    }
    {
        let mut a = app.borrow_mut();
        if a.verification || a.control_center.previews_busy || access != AccessStatus::NotRequested
        {
            return;
        }
        a.control_center.previews_busy = true;
    }
    sync(app, true);
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        let access = NotificationService::request_access().await;
        let Some(app) = weak.upgrade() else { return };
        app.borrow_mut().control_center.previews_busy = false;
        match access {
            Ok(access) => {
                let granted = access == AccessStatus::Allowed;
                let unchanged = {
                    let mut a = app.borrow_mut();
                    a.notification_access = access;
                    a.settings.notifications_enabled == enabled
                        && a.settings.notifications_paused == paused
                };
                if granted && unchanged {
                    set_previews(&app, false);
                }
                if let Some(service) = app.borrow().notifications.clone() {
                    service.refresh_access();
                }
            }
            Err(error) => {
                record(&error);
                if current(&session) {
                    show_error(&app, "Couldn’t request notification access.");
                }
            }
        }
        sync_presentation(&app);
        refresh_preferences(&app);
    });
}

fn publish_hotspot(ui: &IslandWindow, state: hotspot::State) {
    use hotspot::State::*;
    let detail = match state {
        On => "On",
        Off => "Off",
        Changing => "Changing…",
        NoConnection => "No network",
        MissingCapability => "Needs MSIX",
        Unavailable => "Unavailable",
    };
    let known = matches!(state, On | Off);
    ui.set_control_hotspot(RadioControl {
        known,
        on: state == On,
        switchable: known,
        busy: false,
    });
    ui.set_control_hotspot_detail(detail.into());
}

fn request_hotspot(app: &Rc<RefCell<App>>, enabled: Option<bool>) {
    let (session, request) = {
        let mut a = app.borrow_mut();
        let c = &mut a.control_center;
        let Some(session) = c.session.clone() else {
            return;
        };
        if c.hotspot_busy || (enabled.is_none() && c.hotspot_request.is_some()) {
            return;
        }
        let request = Rc::new(());
        c.hotspot_request = Some(request.clone());
        c.hotspot_busy = enabled.is_some();
        if enabled.is_some()
            && let Some(ui) = a.ui.upgrade()
        {
            let mut control = ui.get_control_hotspot();
            control.busy = true;
            ui.set_control_hotspot(control);
            ui.set_control_hotspot_detail("Changing…".into());
        }
        (session, request)
    };
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        let result = hotspot::request(enabled, session.clone()).await;
        let Some(app) = weak.upgrade() else { return };
        {
            let mut a = app.borrow_mut();
            let c = &mut a.control_center;
            if !c
                .hotspot_request
                .as_ref()
                .is_some_and(|pending| Rc::ptr_eq(pending, &request))
            {
                return;
            }
            c.hotspot_request = None;
            c.hotspot_busy = false;
        }
        if !current(&session) {
            request_hotspot(&app, None);
            return;
        }
        let state = result
            .as_ref()
            .copied()
            .unwrap_or(hotspot::State::Unavailable);
        if let Some(ui) = app.borrow().ui.upgrade() {
            publish_hotspot(&ui, state);
        }
        if let Err(error) = result {
            record(&format!("Hotspot: {error}"));
        }
        if let Some(enabled) = enabled {
            let target = if enabled {
                hotspot::State::On
            } else {
                hotspot::State::Off
            };
            if state != target && state != hotspot::State::Changing {
                show_error(
                    &app,
                    if enabled {
                        "Hotspot couldn’t start."
                    } else {
                        "Hotspot couldn’t stop."
                    },
                );
                request_hotspot(&app, None);
            }
            refresh_radios(&app);
        }
    });
}

fn toggle_hotspot(app: &Rc<RefCell<App>>) {
    let enabled = {
        let a = app.borrow();
        if a.verification {
            return;
        }
        let Some(ui) = a.ui.upgrade() else { return };
        let control = ui.get_control_hotspot();
        if !control.switchable || control.busy {
            return;
        }
        !control.on
    };
    request_hotspot(app, Some(enabled));
}

pub(super) fn back(app: &Rc<RefCell<App>>) -> bool {
    let menu = app.borrow().control_center.menu;
    match menu {
        Menu::Controls => return false,
        Menu::ConfirmRestart | Menu::ConfirmShutDown => set_menu(app, Menu::Power),
        _ => set_menu(app, Menu::Controls),
    }
    true
}

fn set_menu(app: &Rc<RefCell<App>>, menu: Menu) {
    let mut a = app.borrow_mut();
    let previous = a.control_center.menu.device_kind();
    let next = menu.device_kind();
    a.control_center.menu = menu;
    if let Some(ui) = a.ui.upgrade() {
        if previous != next {
            a.control_center.devices.clear(&ui);
            if next.is_some() {
                a.control_center.devices.view = Some(Arc::new(AtomicBool::new(true)));
            }
        }
        ui.set_control_menu(menu as i32);
        if a.panel_keyboard {
            ui.invoke_focus_panel();
        }
    }
    let size_changed = previous.is_some() != next.is_some();
    if size_changed {
        a.retarget_shape();
        a.render();
        drop(a);
        wake(app);
    }
}

fn request_power(app: &Rc<RefCell<App>>, action: PowerAction, session: Arc<AtomicBool>) {
    {
        let mut a = app.borrow_mut();
        if a.verification {
            a.control_center.verified_power = Some(action);
            return;
        }
        if a.control_center.power_busy {
            return;
        }
        a.control_center.power_busy = true;
        if let Some(ui) = a.ui.upgrade() {
            ui.set_control_power_busy(true);
        }
    }
    let (signal, wait) = futures_channel::oneshot::channel();
    let guard = session.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("island-power".into())
        .spawn(move || {
            if current(&guard) {
                let _ = signal.send(platform::power::request(action));
            }
        })
    {
        record(&error.to_string());
    }
    let weak = Rc::downgrade(app);
    let _ = slint::spawn_local(async move {
        let result = wait.await;
        let Some(app) = weak.upgrade() else { return };
        {
            let mut a = app.borrow_mut();
            a.control_center.power_busy = false;
            if let Some(ui) = a.ui.upgrade() {
                ui.set_control_power_busy(false);
            }
        }
        if current(&session) {
            if matches!(result, Ok(Ok(()))) {
                transition(&app, IslandState::Idle);
            } else {
                if let Ok(Err(error)) = result {
                    record(&error.to_string());
                }
                show_error(&app, "Windows couldn’t complete this power action.");
            }
        }
    });
}

fn show_error(app: &Rc<RefCell<App>>, message: &str) {
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_control_error(message.into());
    }
}
