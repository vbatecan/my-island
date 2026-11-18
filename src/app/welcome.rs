//! A one-time cue in the existing idle contour.
use super::*;

const DISPLAY_TIME: Duration = Duration::from_secs(9);

pub(super) fn width(text_scale: f64) -> f64 {
    (174. * text_scale).ceil() + 28.
}

pub(super) fn show(app: &Rc<RefCell<App>>) {
    {
        let a = app.borrow();
        let available = platform::available_width() / a.settings.island_scale;
        if a.state != IslandState::Idle
            || a.hidden
            || a.visibility.hiding
            || a.visibility.locked
            || a.settings_visible
            || available < width(a.settings.text_scale) + 40.
        {
            return;
        }
    }
    let settings = Settings::path();
    let Some(profile) = settings.parent() else {
        return;
    };
    if std::fs::create_dir_all(profile).is_err()
        || std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(profile.join("welcome-shown"))
            .is_err()
    {
        return;
    }

    set_active(app, true);
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(DISPLAY_TIME, move || {
        if let Some(app) = weak.upgrade()
            && app.borrow().welcome_hint
        {
            set_active(&app, false);
        }
    });
}

fn set_active(app: &Rc<RefCell<App>>, active: bool) {
    {
        let mut a = app.borrow_mut();
        a.welcome_hint = active;
        a.retarget_shape();
        a.render();
    }
    sync_presentation(app);
    interaction::schedule_hover(app);
}
