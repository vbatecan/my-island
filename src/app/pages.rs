//! Event-driven page navigation. Wheel movement is separate from volume gestures.
use super::*;
use crate::navigation::Page;

#[derive(Default)]
pub(super) struct PageWheel {
    remainder: f64,
    last: Option<u64>,
}
impl PageWheel {
    fn advance(&mut self, delta: f64, now: u64) -> i32 {
        if !delta.is_finite() || delta == 0. {
            return 0;
        }
        if self.last.is_some_and(|last| now.saturating_sub(last) < 160) {
            self.remainder = 0.;
            return 0;
        }
        if self.remainder.signum() != delta.signum() {
            self.remainder = 0.;
        }
        self.remainder += delta.clamp(-4., 4.);
        if self.remainder.abs() < 1. {
            return 0;
        }
        let direction = if self.remainder > 0. { -1 } else { 1 };
        self.remainder = 0.;
        self.last = Some(now);
        direction
    }
    pub(super) fn reset(&mut self) {
        self.remainder = 0.;
        self.last = None;
    }
}

pub(super) fn available(app: &Rc<RefCell<App>>) -> Vec<(Page, String)> {
    let mut pages = vec![
        (Page::Home, "Home".into()),
        (Page::Media, "Media".into()),
        (Page::Timer, "Timer".into()),
        (Page::Volume, "Volume".into()),
    ];
    let host = app.borrow().plugin_host.clone();
    if let Some(host) = host {
        let mut installed = host.list();
        installed.sort_by(|a, b| {
            a.manifest
                .name
                .to_lowercase()
                .cmp(&b.manifest.name.to_lowercase())
                .then(a.manifest.id.cmp(&b.manifest.id))
        });
        for plugin in installed.into_iter().filter(|p| p.enabled) {
            for view in plugin.manifest.views {
                let label = if view.id == "main" {
                    plugin.manifest.name.clone()
                } else {
                    format!("{} · {}", plugin.manifest.name, view.label)
                };
                pages.push((
                    Page::Plugin {
                        id: plugin.manifest.id.clone(),
                        view: view.id,
                    },
                    label,
                ));
            }
        }
    }
    pages
}

fn current(app: &Rc<RefCell<App>>) -> Page {
    let a = app.borrow();
    if let Some(selected) = &a.settings.selected_page {
        return selected.clone();
    }
    match a.presentation.as_ref().map(|v| &v.content) {
        Some(Content::Media | Content::CompactMedia) => Page::Media,
        Some(Content::Timer | Content::CompactTimer | Content::TimerFinished) => Page::Timer,
        Some(Content::VolumePanel | Content::Volume) => Page::Volume,
        _ => Page::Home,
    }
}

pub(super) fn sync_hint(app: &Rc<RefCell<App>>) {
    let pages = available(app);
    let selected = current(app);
    let index = pages
        .iter()
        .position(|(page, _)| *page == selected)
        .unwrap_or(0);
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_page_index(index as i32);
        ui.set_page_count(pages.len() as i32);
        ui.set_page_label(pages[index].1.as_str().into());
    }
}

pub(super) fn wheel(app: &Rc<RefCell<App>>, delta: f64) {
    let direction = {
        let mut a = app.borrow_mut();
        let now = a.now();
        a.page_wheel.advance(delta, now)
    };
    if direction == 0 {
        return;
    }
    let pages = available(app);
    let selected = current(app);
    let index = pages
        .iter()
        .position(|(page, _)| *page == selected)
        .unwrap_or(0);
    let next = (index as i32 + direction).rem_euclid(pages.len() as i32) as usize;
    choose(app, pages[next].0.clone());
}

pub(super) fn choose(app: &Rc<RefCell<App>>, page: Page) {
    if !page.is_valid() || !available(app).iter().any(|(p, _)| *p == page) {
        return;
    }
    let (ui, delay) = {
        let mut a = app.borrow_mut();
        a.page_switch_timer.stop();
        a.settings.selected_page = Some(page.clone());
        let now = a.now();
        a.coordinator.set_selected_page(Some(page.clone()), now);
        if !a.verification
            && let Err(error) = a.settings.save()
        {
            record(&format!("Could not remember island page: {error}"));
        }
        (
            a.ui.upgrade(),
            if a.animations_enabled() {
                (100. / a.settings.animation_speed).round() as u64
            } else {
                0
            },
        )
    };
    if let Some(ui) = ui {
        ui.invoke_begin_page_switch();
    }
    if delay == 0 {
        commit(app, page);
        return;
    }
    let weak = Rc::downgrade(app);
    app.borrow().page_switch_timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(delay),
        move || {
            if let Some(app) = weak.upgrade() {
                commit(&app, page.clone());
            }
        },
    );
}

pub(super) fn open(app: &Rc<RefCell<App>>, page: Page) {
    if !available(app).iter().any(|(p, _)| *p == page) {
        return;
    }
    app.borrow().page_switch_timer.stop();
    if app.borrow().coordinator.panel_open() {
        choose(app, page);
        return;
    }
    let ui = app.borrow().ui.upgrade();
    if let (Page::Plugin { id, view }, Some(ui)) = (&page, &ui) {
        ui.invoke_plugin_page_select(id.as_str().into(), view.as_str().into());
    }
    {
        let mut a = app.borrow_mut();
        a.settings.selected_page = Some(page.clone());
        let now = a.now();
        a.coordinator.navigate_page(page, now);
        if !a.verification
            && let Err(error) = a.settings.save()
        {
            record(&format!("Could not remember island page: {error}"));
        }
    }
    sync_presentation(app);
    if let Some(ui) = ui {
        ui.invoke_end_page_switch();
    }
    interaction::schedule_collapse(app);
}

fn commit(app: &Rc<RefCell<App>>, page: Page) {
    if app.borrow().settings.selected_page.as_ref() != Some(&page) {
        if let Some(ui) = app.borrow().ui.upgrade() {
            ui.invoke_end_page_switch();
        }
        return;
    }
    let (open, ui) = {
        let a = app.borrow();
        (a.coordinator.panel_open(), a.ui.upgrade())
    };
    // This callback can borrow App, so invoke it outside all App borrows.
    if let (Page::Plugin { id, view }, Some(ui)) = (&page, &ui) {
        ui.invoke_plugin_page_select(id.as_str().into(), view.as_str().into());
    }
    {
        let mut a = app.borrow_mut();
        let now = a.now();
        // Leaving during the finite crossfade must not reopen the notch.
        if open {
            a.coordinator.navigate_page(page, now);
        }
    }
    sync_presentation(app);
    if let Some(ui) = ui {
        ui.invoke_end_page_switch();
    }
    interaction::schedule_collapse(app);
}

pub(super) fn reconcile(app: &Rc<RefCell<App>>) {
    let selected = app.borrow().settings.selected_page.clone();
    if let Some(selected) = selected
        && !available(app).iter().any(|(page, _)| *page == selected)
    {
        choose(app, Page::Home);
    }
    sync_hint(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_accumulates_trackpad_motion_without_bursting_through_pages() {
        let mut wheel = PageWheel::default();
        assert_eq!(wheel.advance(-0.4, 0), 0);
        assert_eq!(wheel.advance(-0.6, 10), 1);
        assert_eq!(wheel.advance(-10., 50), 0);
        assert_eq!(wheel.advance(-10., 170), 1);
        assert_eq!(wheel.advance(1., 330), -1);
        assert_eq!(wheel.advance(f64::NAN, 500), 0);
        wheel.reset();
        assert_eq!(wheel.advance(0.6, 600), 0);
        assert_eq!(wheel.advance(-0.6, 700), 0);
        assert_eq!(wheel.advance(-0.4, 710), 1);
    }
}
