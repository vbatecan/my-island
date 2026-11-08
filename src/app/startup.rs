//! Serialize native startup changes and show registration readback in preferences.
use super::*;
use crate::platform::startup::StartupStatus;

pub(super) fn refresh(app: &Rc<RefCell<App>>) {
    {
        let mut a = app.borrow_mut();
        if a.startup_busy {
            return;
        }
        if a.verification {
            a.startup.available = false;
            a.startup.message = "Startup registration is unchanged during verification.".into();
            return;
        }
        a.startup_busy = true;
    }
    refresh_preferences(app);
    let weak = Rc::downgrade(app);
    if let Err(error) = slint::spawn_local(async move {
        let status = crate::platform::startup::snapshot().await;
        if let Some(app) = weak.upgrade() {
            finish(&app, status, None, false);
        }
    }) {
        {
            let mut a = app.borrow_mut();
            a.startup_busy = false;
            a.startup.available = false;
            a.startup.message = format!("Cannot read Windows startup registration: {error}");
        }
        settings_status(app, &error.to_string());
        refresh_preferences(app);
    }
}

pub(super) fn request(app: &Rc<RefCell<App>>, enabled: bool) {
    {
        let mut a = app.borrow_mut();
        if a.verification {
            return;
        }
        a.startup_pending = Some(enabled);
        if a.startup_busy {
            return;
        }
        a.startup_pending = None;
        a.startup_busy = true;
        a.settings.launch_at_startup = enabled;
    }
    refresh_preferences(app);
    let weak = Rc::downgrade(app);
    if let Err(error) = slint::spawn_local(async move {
        let (status, error) = match crate::platform::startup::set_enabled(enabled).await {
            Ok(status) => (status, None),
            Err(error) => (crate::platform::startup::snapshot().await, Some(error)),
        };
        if let Some(app) = weak.upgrade() {
            finish(&app, status, error, true);
        }
    }) {
        {
            let mut a = app.borrow_mut();
            a.startup_busy = false;
            a.startup_pending = None;
            a.settings.launch_at_startup = a.startup.enabled;
            let _ = a.settings.save();
        }
        settings_status(app, &error.to_string());
        refresh_preferences(app);
    }
}

fn finish(app: &Rc<RefCell<App>>, status: StartupStatus, error: Option<String>, persist: bool) {
    let next = {
        let mut a = app.borrow_mut();
        a.settings.launch_at_startup = status.enabled;
        a.startup = status;
        a.startup_busy = false;
        a.startup_pending.take()
    };
    if let Some(error) = error {
        settings_status(app, &error);
    }
    if let Some(next) = next {
        request(app, next);
    } else {
        if persist && let Err(error) = app.borrow().settings.save() {
            settings_status(app, &format!("Preferences could not be saved: {error}"));
        }
        refresh_preferences(app);
    }
}
