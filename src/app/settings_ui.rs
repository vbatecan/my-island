//! Settings-window lifecycle, persistence, and platform preference application.
use super::*;

pub(super) fn access_label(access: &AccessStatus) -> String {
    match access{AccessStatus::UnavailablePortable=>"Portable build: install the MSIX package to enable Windows notifications.".into(),AccessStatus::NotRequested=>"Access has not been granted. Choose Enable access to request it.".into(),AccessStatus::Denied=>"Windows denied access. Allow notification access in Windows Settings, then choose Retry.".into(),AccessStatus::Allowed=>"Windows notification access is allowed. Previews never clear Notification Center.".into(),AccessStatus::Error(e)=>format!("Notification access unavailable: {e}")}
}

pub(super) fn settings_status(app: &Rc<RefCell<App>>, message: &str) {
    if let Some(ui) = app.borrow().preferences.upgrade() {
        ui.set_status_text(message.into());
    }
}

pub(super) fn refresh_preferences(app: &Rc<RefCell<App>>) {
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
        let a = app.borrow();
        ui.set_startup_available(a.startup.available && !a.startup_busy);
        ui.set_startup_status(if a.startup_busy {
            "Checking Windows startup…".into()
        } else {
            a.startup.message.as_str().into()
        });
    }
    app.borrow_mut().settings_sync = false;
}

pub(super) fn apply_settings(app: &Rc<RefCell<App>>, mut settings: Settings, persist: bool) {
    settings.normalize();
    let (old, attached, service) = {
        let mut a = app.borrow_mut();
        let old = a.settings.clone();
        a.settings = settings.clone();
        a.sync_canvas(false);
        if old.island_scale != settings.island_scale || old.sizes() != settings.sizes() {
            a.pointer = None;
            a.wheel.reset();
        }
        let now = a.now();
        a.coordinator
            .set_selected_page(settings.selected_page.clone(), now);
        a.coordinator.set_dwell(settings.notification_duration, now);
        if !settings.battery_previews {
            a.coordinator.clear_battery(now);
        }
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
        if let Err(error) = platform::configure_launcher_shortcut(
            settings.launcher_enabled,
            settings.launcher_shortcut,
        ) {
            settings_status(app, &error);
        }
        if let Err(error) = platform::set_fullscreen_overrides(
            &settings.fullscreen_always_hide,
            &settings.fullscreen_always_show,
        ) {
            settings_status(app, &error);
        }
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
        if old.appearance != settings.appearance
            || old.offset != settings.offset
            || old.monitor_id != settings.monitor_id
            || old.form_factor != settings.form_factor
            || old.island_scale != settings.island_scale
            || old.sizes() != settings.sizes()
        {
            configure_material(app);
        } else {
            let _ = platform::set_appearance(settings.appearance);
        }
    }
    let mut canvas_shrunk = false;
    {
        let mut a = app.borrow_mut();
        if !a.animations_enabled() {
            let state = a.state;
            a.target(state);
            a.timer.stop();
            canvas_shrunk = a.sync_canvas(true);
        } else if old.notch_corner_radius != settings.notch_corner_radius
            || old.notch_contracted_radius != settings.notch_contracted_radius
            || old.form_factor != settings.form_factor
            || old.monitor_id != settings.monitor_id
            || old.island_scale != settings.island_scale
            || old.sizes() != settings.sizes()
        {
            a.retarget_shape();
        }
        a.render();
    }
    if canvas_shrunk && attached {
        configure_material(app);
    }
    if app.borrow().animations_enabled() {
        wake(app);
    }
    if old.privacy != settings.privacy {
        let a = app.borrow();
        if let (Some(ui), Some(view)) = (a.ui.upgrade(), a.rendered_presentation.as_ref()) {
            apply_content_properties(&ui, view, &a.settings);
        }
    }
    if persist && let Err(e) = app.borrow().settings.save() {
        settings_status(app, &format!("Preferences could not be saved: {e}"));
    }
    if old.launch_at_startup != settings.launch_at_startup {
        startup::request(app, settings.launch_at_startup);
    }
    sync_presentation(app);
    interaction::refresh_pointer(app);
    refresh_preferences(app);
}

pub(super) fn show_preferences(app: &Rc<RefCell<App>>) {
    if app.borrow().launcher.open {
        launcher::dismiss(app, false);
    }
    startup::refresh(app);
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

pub(super) fn close_preferences(app: &Rc<RefCell<App>>, hide: bool) {
    let ui = app.borrow().preferences.upgrade();
    if hide && let Some(ui) = ui {
        let _ = ui.hide();
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

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow, prefs: &SettingsWindow) {
    let preview_generation = Rc::new(std::cell::Cell::new(0u64));
    {
        let app = app.clone();
        let generation = preview_generation.clone();
        prefs.on_preview_changed(move || {
            generation.set(generation.get().wrapping_add(1));
            if let Some(ui) = app.borrow().preferences.upgrade() {
                ui.set_preview_animating(false);
            }
            refresh_preferences(&app);
        });
    }
    {
        let app = app.clone();
        prefs.on_apply_layout_preset(move |index| {
            let mut settings = app.borrow().settings.clone();
            if settings.apply_layout_preset(index) {
                let persist = !app.borrow().verification;
                apply_settings(&app, settings, persist);
            }
        });
    }
    {
        let app = app.clone();
        ui.on_settings_requested(move || show_preferences(&app));
    }
    {
        let app = app.clone();
        prefs.on_changed(move || {
            if app.borrow().settings_sync {
                return;
            }
            let ui = app.borrow().preferences.upgrade();
            if let Some(ui) = ui {
                for raw in [ui.get_fullscreen_always_hide_text(), ui.get_fullscreen_always_show_text()] {
                    if let Some(invalid) = raw.split([',', ';']).map(str::trim).filter(|name| !name.is_empty())
                        .find(|name| platform::normalize_fullscreen_executable(name).is_none()) {
                        settings_status(&app, &format!("Invalid app name: {invalid}. Use a filename such as game.exe, without a path."));
                        return;
                    }
                }
                let settings =
                    crate::preferences::read(&ui, &app.borrow().settings, &platform::monitors());
                let persist = !app.borrow().verification;
                settings_status(&app, "Saved locally on this PC.");
                apply_settings(&app, settings, persist);
            }
        });
    }
    {
        let app = app.clone();
        prefs.on_reset_defaults(move || {
            let persist = !app.borrow().verification;
            apply_settings(&app, Settings::default(), persist);
        });
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
        let generation = preview_generation;
        prefs.on_preview_motion(move || {
            let Some(ui) = app.borrow().preferences.upgrade() else {
                return;
            };
            let original = ui.get_preview_state();
            let token = generation.get().wrapping_add(1);
            generation.set(token);
            let duration = (320. / app.borrow().settings.animation_speed) as u64;
            ui.set_preview_animating(false);
            ui.set_preview_state(0);
            refresh_preferences(&app);
            // Three finite callbacks; manual preview selection cancels this cycle.
            for (at, state, animated) in [
                (30, 2, true),
                (duration + 130, original, true),
                (duration * 2 + 230, original, false),
            ] {
                let weak = Rc::downgrade(&app);
                let generation = generation.clone();
                slint::Timer::single_shot(Duration::from_millis(at), move || {
                    if generation.get() != token {
                        return;
                    }
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    if let Some(ui) = app.borrow().preferences.upgrade() {
                        ui.set_preview_animating(animated);
                        ui.set_preview_state(state);
                    }
                    refresh_preferences(&app);
                });
            }
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
}
