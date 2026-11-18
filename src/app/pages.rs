//! Event-driven page navigation. Wheel movement is separate from volume gestures.
use super::*;
use crate::navigation::Page;

const EXIT_MS: f64 = 100.;

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
        (Page::ControlCenter, "Control Center".into()),
        (Page::Home, "Home".into()),
        (Page::Media, "Media".into()),
        (Page::Timer, "Timer".into()),
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
    match a.presentation.as_ref().map(|v| &v.content) {
        Some(Content::Launcher) => Page::Home,
        Some(Content::Media) => Page::Media,
        Some(Content::Timer) => Page::Timer,
        Some(Content::PluginPanel) => match &a.settings.selected_page {
            Some(page @ Page::Plugin { .. }) => page.clone(),
            _ => Page::ControlCenter,
        },
        _ => Page::ControlCenter,
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
        let previous = neighbor_index(index, -1, pages.len());
        let next = neighbor_index(index, 1, pages.len());
        ui.set_previous_page_label(pages[previous].1.as_str().into());
        ui.set_next_page_label(pages[next].1.as_str().into());
    }
}

fn neighbor_index(index: usize, direction: i32, count: usize) -> usize {
    (index as i32 + direction).rem_euclid(count as i32) as usize
}

pub(super) fn wheel(app: &Rc<RefCell<App>>, delta: f64) {
    let direction = {
        let mut a = app.borrow_mut();
        let now = a.now();
        a.page_wheel.advance(delta, now)
    };
    if direction != 0 {
        step(app, direction);
    }
}

pub(super) fn step(app: &Rc<RefCell<App>>, direction: i32) {
    if !app.borrow().coordinator.panel_open() || app.borrow().launcher.open {
        return;
    }
    let direction = direction.signum();
    let pages = available(app);
    let pending = app.borrow().pending_page.clone();
    let selected = pending.unwrap_or_else(|| current(app));
    let index = pages
        .iter()
        .position(|(page, _)| *page == selected)
        .unwrap_or(0);
    let next = neighbor_index(index, direction, pages.len());
    begin(app, pages[next].0.clone(), direction);
}

pub(super) fn choose(app: &Rc<RefCell<App>>, page: Page) {
    let pages = available(app);
    let Some(next) = pages.iter().position(|(p, _)| *p == page) else {
        return;
    };
    let current = current(app);
    let index = pages.iter().position(|(p, _)| *p == current).unwrap_or(0);
    begin(app, page, if next >= index { 1 } else { -1 });
}

fn begin(app: &Rc<RefCell<App>>, page: Page, direction: i32) {
    if current(app) == page
        && app.borrow().presentation.as_ref().is_some_and(|view| {
            matches!(
                view.content,
                Content::ControlCenter
                    | Content::Launcher
                    | Content::Media
                    | Content::Timer
                    | Content::PluginPanel
            )
        })
    {
        cancel(app);
        return;
    }
    let (ui, delay) = {
        let mut a = app.borrow_mut();
        a.page_switch_timer.stop();
        a.pending_page = Some(page.clone());
        (
            a.ui.upgrade(),
            if a.animations_enabled() {
                (EXIT_MS / a.settings.animation_speed).round() as u64
            } else {
                0
            },
        )
    };
    if delay == 0 {
        commit(app, page);
        return;
    }
    if let Some(ui) = ui {
        ui.invoke_begin_page_switch(direction);
    }
    animate_page(app, 0., false);
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
    cancel(app);
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
        select(&mut a, page);
    }
    sync_presentation(app);
    if let Some(ui) = ui {
        ui.invoke_end_page_switch();
    }
    animate_page(app, 1., false);
    interaction::schedule_collapse(app);
}

fn commit(app: &Rc<RefCell<App>>, page: Page) {
    if app.borrow().pending_page.as_ref() != Some(&page) {
        return;
    }
    let (open, ui) = {
        let a = app.borrow();
        (
            a.coordinator.panel_open() && !a.hidden && !a.launcher.open,
            a.ui.upgrade(),
        )
    };
    if !open
        || !available(app)
            .iter()
            .any(|(candidate, _)| *candidate == page)
    {
        cancel(app);
        return;
    }
    // This callback can borrow App, so invoke it outside all App borrows.
    if let (Page::Plugin { id, view }, Some(ui)) = (&page, &ui) {
        ui.invoke_plugin_page_select(id.as_str().into(), view.as_str().into());
    }
    {
        let mut a = app.borrow_mut();
        a.pending_page = None;
        select(&mut a, page);
    }
    sync_presentation(app);
    {
        // The page is already invisible at the handoff. Keep the geometry
        // springs moving, but mount the incoming content before it slides in.
        let mut a = app.borrow_mut();
        if let Some(geometry) = a.pending_geometry.take() {
            for (spring, target) in a.springs[..3].iter_mut().zip(geometry) {
                spring.set_target(target);
            }
        }
        a.apply_content();
    }
    if let Some(ui) = ui {
        ui.invoke_end_page_switch();
    }
    animate_page(app, 1., true);
    interaction::schedule_collapse(app);
}

fn animate_page(app: &Rc<RefCell<App>>, target: f32, arriving: bool) {
    let animate = {
        let mut a = app.borrow_mut();
        let animate = a.settings.appearance == Appearance::LiquidGlass && a.animations_enabled();
        if animate {
            if arriving {
                a.page_motion.snap(0.);
            }
            let timing = if target == 0. {
                liquid_rust::Spring::new(0.16, 0.)
            } else {
                liquid_rust::Spring::EMERGE
            };
            a.page_motion.launch_to(target, timing, 1.);
        } else {
            a.page_motion.snap(target);
        }
        if let Some(ui) = a.ui.upgrade() {
            ui.set_liquid_page_opacity(a.page_motion.value);
        }
        animate
    };
    if animate {
        wake(app);
    }
}

fn select(app: &mut App, page: Page) {
    let changed = app.settings.selected_page.as_ref() != Some(&page);
    app.settings.selected_page = Some(page.clone());
    let now = app.now();
    app.coordinator.navigate_page(page, now);
    if changed
        && !app.verification
        && let Err(error) = app.settings.save()
    {
        record(&format!("Could not remember island page: {error}"));
    }
}

pub(super) fn cancel(app: &Rc<RefCell<App>>) {
    let ui = {
        let mut a = app.borrow_mut();
        a.page_switch_timer.stop();
        a.pending_page = None;
        a.ui.upgrade()
    };
    if let Some(ui) = ui {
        ui.invoke_cancel_page_switch();
    }
    animate_page(app, 1., false);
}

pub(super) fn reconcile(app: &Rc<RefCell<App>>) {
    let selected = app.borrow().settings.selected_page.clone();
    if let Some(selected) = selected
        && !available(app).iter().any(|(page, _)| *page == selected)
    {
        if matches!(
            app.borrow().presentation.as_ref().map(|view| &view.content),
            Some(Content::PluginPanel)
        ) {
            choose(app, Page::Home);
        } else {
            let mut a = app.borrow_mut();
            a.settings.selected_page = Some(Page::Home);
            let now = a.now();
            a.coordinator.set_selected_page(Some(Page::Home), now);
            if !a.verification
                && let Err(error) = a.settings.save()
            {
                record(&format!("Could not remember island page: {error}"));
            }
            drop(a);
            sync_presentation(app);
        }
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
