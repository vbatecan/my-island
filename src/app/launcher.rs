//! Keyboard launcher orchestration; shell work stays off the UI thread.
use super::*;
use crate::launcher::{RecentLaunches, search};
use crate::platform::launcher::{AppEntry, IconPixels, LaunchTarget};
use std::collections::{HashMap, VecDeque};

enum Update {
    Failed(&'static str, String),
    Catalog(Result<platform::launcher::Catalog, String>),
    Icons(Vec<((String, u32), Option<IconPixels>)>),
    Launched(String, Result<(), String>),
}
#[derive(Default)]
pub(super) struct Controller {
    pub open: bool,
    entries: Vec<AppEntry>,
    results: Vec<usize>,
    recent: RecentLaunches,
    loaded: bool,
    indexed: Option<Instant>,
    loading: bool,
    icons_loading: bool,
    launching: bool,
    cache: HashMap<(String, u32), Option<slint::Image>>,
    inbox: Arc<Mutex<VecDeque<Update>>>,
}
fn history_path() -> std::path::PathBuf {
    Settings::path().with_file_name("launcher-recent.json")
}
pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    if let Some(prefs) = app.borrow().preferences.upgrade() {
        let weak = Rc::downgrade(app);
        prefs.on_clear_launcher_history(move || {
            if let Some(app) = weak.upgrade() {
                let result = {
                    let mut a = app.borrow_mut();
                    a.launcher.recent.clear();
                    a.launcher.loaded = true;
                    a.launcher.recent.save(&history_path())
                };
                if let Err(error) = result {
                    settings_status(&app, &error);
                } else {
                    settings_status(&app, "Recent launches cleared.");
                }
                refresh(&app);
            }
        });
    }
    let weak = Rc::downgrade(app);
    ui.on_launcher_updated(move || {
        if let Some(app) = weak.upgrade() {
            receive(&app);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_query(move |_| {
        if let Some(app) = weak.upgrade() {
            refresh(&app);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_navigate(move |delta| {
        if let Some(app) = weak.upgrade() {
            let a = app.borrow();
            if let Some(ui) = a.ui.upgrade() {
                let count = a.launcher.results.len() as i32;
                if count > 0 {
                    ui.set_launcher_selected(
                        (ui.get_launcher_selected() + delta).rem_euclid(count),
                    );
                }
            }
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_activate(move |index| {
        if let Some(app) = weak.upgrade() {
            activate(&app, index);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_dismiss(move || {
        if let Some(app) = weak.upgrade() {
            dismiss(&app, true);
        }
    });
}
pub(super) fn toggle(app: &Rc<RefCell<App>>) {
    if app.borrow().launcher.open {
        dismiss(app, true);
        return;
    }
    if app.borrow().hidden || app.borrow().settings_visible {
        return;
    }
    {
        let mut a = app.borrow_mut();
        a.hover_timer.stop();
        a.collapse_timer.stop();
        a.page_switch_timer.stop();
        a.launcher.open = true;
        if !a.launcher.loaded {
            a.launcher.recent = RecentLaunches::load(&history_path());
            a.launcher.loaded = true;
        }
        let now = a.now();
        a.coordinator.open(Panel::AppLauncher, now);
    }
    sync_presentation(app);
    if let Err(error) = platform::set_keyboard_mode(true, false) {
        record(&format!("Launcher focus unavailable: {error}"));
        dismiss(app, false);
        settings_status(app, &error);
        return;
    }
    if let Some(ui) = app.borrow().ui.upgrade() {
        ui.set_launcher_text("".into());
        ui.set_launcher_status("".into());
        ui.invoke_launcher_focus();
    }
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(80), move || {
        if let Some(app) = weak.upgrade() {
            let a = app.borrow();
            if a.launcher.open
                && let Some(ui) = a.ui.upgrade()
            {
                ui.invoke_launcher_focus();
            }
        }
    });
    refresh(app);
    let scan = {
        let a = app.borrow();
        !a.launcher.loading
            && a.launcher
                .indexed
                .is_none_or(|t| t.elapsed() > Duration::from_secs(60))
    };
    if scan {
        app.borrow_mut().launcher.loading = true;
        refresh(app);
        start(app, "catalog", || {
            Update::Catalog(platform::launcher::enumerate())
        });
    }
}
pub(super) fn dismiss(app: &Rc<RefCell<App>>, restore: bool) {
    if !app.borrow().launcher.open {
        return;
    }
    {
        let mut a = app.borrow_mut();
        a.launcher.open = false;
        let now = a.now();
        a.coordinator.close(now);
        a.hover_intent.leave();
        a.hover_timer.stop();
        a.collapse_timer.stop();
    }
    let _ = platform::set_keyboard_mode(false, restore);
    sync_presentation(app);
}
fn start(
    app: &Rc<RefCell<App>>,
    kind: &'static str,
    task: impl FnOnce() -> Update + Send + 'static,
) {
    let (inbox, weak) = {
        let a = app.borrow();
        (a.launcher.inbox.clone(), a.ui.clone())
    };
    // Each on-demand task exits once done. No idle worker, timer, or polling.
    let failure_inbox = inbox.clone();
    let failure_ui = weak.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("island-launcher".into())
        .spawn(move || {
            inbox
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(task());
            let _ = weak.upgrade_in_event_loop(|ui| ui.invoke_launcher_updated());
        })
    {
        failure_inbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(Update::Failed(kind, error.to_string()));
        let _ = failure_ui.upgrade_in_event_loop(|ui| ui.invoke_launcher_updated());
    }
}
fn receive(app: &Rc<RefCell<App>>) {
    let events = {
        let a = app.borrow();
        a.launcher
            .inbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect::<Vec<_>>()
    };
    let mut close = false;
    for event in events {
        let mut a = app.borrow_mut();
        match event {
            Update::Failed(kind, error) => {
                match kind {
                    "catalog" => a.launcher.loading = false,
                    "icons" => a.launcher.icons_loading = false,
                    _ => a.launcher.launching = false,
                }
                if let Some(ui) = a.ui.upgrade() {
                    ui.set_launcher_status(error.into());
                }
            }
            Update::Catalog(result) => {
                a.launcher.loading = false;
                a.launcher.indexed = Some(Instant::now());
                match result {
                    Ok(catalog) => {
                        a.launcher.entries = catalog.entries;
                        if let Some(ui) = a.ui.upgrade() {
                            ui.set_launcher_status(catalog.warnings.join(" · ").into());
                        }
                    }
                    Err(error) => {
                        if let Some(ui) = a.ui.upgrade() {
                            ui.set_launcher_status(error.into());
                        }
                    }
                }
            }
            Update::Icons(icons) => {
                a.launcher.icons_loading = false;
                if a.launcher.cache.len() + icons.len() > 128 {
                    a.launcher.cache.clear();
                }
                for (key, pixels) in icons {
                    let image = pixels.map(|p| {
                        slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                            &p.rgba, p.width, p.height,
                        ))
                    });
                    a.launcher.cache.insert(key, image);
                }
            }
            Update::Launched(id, result) => {
                a.launcher.launching = false;
                match result {
                    Ok(()) => {
                        a.launcher.recent.record(&id);
                        if let Err(error) = a.launcher.recent.save(&history_path()) {
                            record(&format!("Launcher history could not be saved: {error}"));
                        }
                        close = true;
                    }
                    Err(error) => {
                        if let Some(ui) = a.ui.upgrade() {
                            ui.set_launcher_status(error.into());
                        }
                    }
                }
            }
        }
    }
    if close {
        dismiss(app, false);
    }
    refresh(app);
}
fn icon_size(ui: &IslandWindow) -> u32 {
    (32. * ui.get_ui_scale() * ui.window().scale_factor())
        .round()
        .clamp(16., 128.) as u32
}
pub(super) fn refresh(app: &Rc<RefCell<App>>) {
    let Some(ui) = app.borrow().ui.upgrade() else {
        return;
    };
    let size = icon_size(&ui);
    let mut requests: Vec<((String, u32), LaunchTarget)> = Vec::new();
    let rows = {
        let mut a = app.borrow_mut();
        let indices = search(
            &a.launcher.entries,
            &ui.get_launcher_text(),
            &a.launcher.recent,
            6,
        );
        if a.launcher.results != indices {
            ui.set_launcher_selected(0);
        }
        a.launcher.results = indices.clone();
        ui.set_launcher_busy(a.launcher.loading || a.launcher.launching);
        indices
            .into_iter()
            .map(|index| {
                let entry = &a.launcher.entries[index];
                let key = (entry.id.clone(), size);
                let icon = a.launcher.cache.get(&key).and_then(|v| v.clone());
                if a.launcher.open
                    && !a.launcher.icons_loading
                    && !a.launcher.cache.contains_key(&key)
                {
                    requests.push((key, entry.target.clone()));
                }
                crate::LauncherResult {
                    id: entry.id.as_str().into(),
                    title: entry.name.as_str().into(),
                    detail: if a.launcher.recent.ids().contains(&entry.id) {
                        format!("Recently opened · {}", entry.detail).into()
                    } else {
                        entry.detail.as_str().into()
                    },
                    has_icon: icon.is_some(),
                    icon: icon.unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>()
    };
    ui.set_launcher_results(slint::ModelRc::from(Rc::new(slint::VecModel::from(rows))));
    if !requests.is_empty() {
        app.borrow_mut().launcher.icons_loading = true;
        start(app, "icons", move || {
            Update::Icons(
                requests
                    .into_iter()
                    .map(|(key, target)| {
                        let pixels = platform::launcher::load_icon(&target, key.1).ok();
                        (key, pixels)
                    })
                    .collect(),
            )
        });
    }
}
fn activate(app: &Rc<RefCell<App>>, index: i32) {
    let target = {
        let mut a = app.borrow_mut();
        if !a.launcher.open || a.launcher.launching || index < 0 {
            return;
        }
        let Some(entry) = a
            .launcher
            .results
            .get(index as usize)
            .and_then(|i| a.launcher.entries.get(*i))
            .cloned()
        else {
            return;
        };
        a.launcher.launching = true;
        (entry.id, entry.target)
    };
    refresh(app);
    start(app, "launch", move || {
        Update::Launched(target.0, platform::launcher::launch(&target.1))
    });
}

pub(super) fn setup_verification(app: &Rc<RefCell<App>>) {
    let frame = Rc::new(std::cell::Cell::new(0));
    for (step, delay) in [500, 2500, 3100, 3400, 3700, 4100, 5000, 6500, 7500]
        .into_iter()
        .enumerate()
    {
        let weak = Rc::downgrade(app);
        let frame = frame.clone();
        slint::Timer::single_shot(Duration::from_millis(delay + 1500), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            let check = |ok, label| {
                record(&format!(
                    "launcher {} {label}",
                    if ok { "PASS" } else { "FAILED" }
                ))
            };
            let key = |text: slint::SharedString| {
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                        text: text.clone(),
                    });
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
            };
            match step {
                0 => {
                    // Test WM_HOTKEY's real foreground grant, rather than
                    // asking a background timer to steal keyboard focus.
                    if platform::configure_launcher_shortcut(true, 0).is_ok() {
                        use windows::Win32::UI::Input::KeyboardAndMouse::*;
                        let event = |key, flags| INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: key,
                                    dwFlags: flags,
                                    ..Default::default()
                                },
                            },
                        };
                        let input = [
                            event(VK_MENU, KEYBD_EVENT_FLAGS(0)),
                            event(VK_SPACE, KEYBD_EVENT_FLAGS(0)),
                            event(VK_SPACE, KEYEVENTF_KEYUP),
                            event(VK_MENU, KEYEVENTF_KEYUP),
                        ];
                        // SAFETY: finite opt-in test sends only the chord we
                        // just registered to this process, never arbitrary input.
                        check(
                            unsafe { SendInput(&input, std::mem::size_of::<INPUT>() as i32) } == 4,
                            "registered shortcut injected through native keyboard path",
                        );
                    } else {
                        check(false, "launcher shortcut registration available");
                    }
                }
                1 => {
                    let a = app.borrow();
                    check(
                        a.launcher.open && a.state == IslandState::AppLauncher,
                        "shortcut destination morphs to launcher",
                    );
                    check(
                        !a.launcher.entries.is_empty() && !a.launcher.loading,
                        "real installed catalog loaded asynchronously",
                    );
                    check(
                        a.springs[0].position >= 500. && a.springs[1].position >= 390.,
                        "launcher geometry contains six results",
                    );
                    drop(a);
                    let native = platform::diagnostic_snapshot();
                    record(&format!("launcher native {native}"));
                    check(
                        native.contains("noactivate=false") && native.contains("foreground=true"),
                        "launcher owns actual keyboard focus without taskbar activation style",
                    );
                    let _ = crate::verification::snapshot(&ui, "app-launcher");
                    key("zzzzzzzzzzzzzzzzzzzzzzzz".into());
                }
                2 => {
                    check(
                        ui.get_launcher_text().starts_with("zzzz"),
                        "typing reaches focused search field",
                    );
                    check(
                        app.borrow().launcher.results.is_empty(),
                        "unmatched typed query has empty results",
                    );
                    ui.set_launcher_text("".into());
                    refresh(&app);
                    key(slint::platform::Key::DownArrow.into());
                }
                3 => {
                    check(
                        ui.get_launcher_selected() == 1,
                        "down arrow selects next application",
                    );
                    key(slint::platform::Key::Escape.into());
                }
                4 => {
                    check(
                        !app.borrow().launcher.open
                            && platform::diagnostic_snapshot().contains("noactivate=true"),
                        "Escape contracts and restores nonactivating mode",
                    );
                    toggle(&app);
                }
                5 => {
                    check(
                        app.borrow().launcher.open,
                        "launcher reopens after dismissal",
                    );
                    let target = std::env::args().find_map(|a| {
                        a.strip_prefix("--verify-launcher-target=")
                            .map(std::path::PathBuf::from)
                    });
                    if let Some(target) = target {
                        let valid = target.canonicalize().ok().is_some_and(|p| {
                            std::env::current_dir().ok().is_some_and(|cwd| {
                                cwd.join("target")
                                    .canonicalize()
                                    .ok()
                                    .is_some_and(|root| p.starts_with(root))
                            })
                        });
                        check(
                            valid && target.extension().is_some_and(|e| e == "lnk"),
                            "launch test restricted to owned target shortcut",
                        );
                        if valid {
                            let mut a = app.borrow_mut();
                            a.launcher.entries = vec![AppEntry {
                                id: "fixture-launch".into(),
                                name: "Launcher verification".into(),
                                detail: "Owned fixture".into(),
                                target: LaunchTarget::Shortcut(target),
                            }];
                            a.launcher.indexed = Some(Instant::now());
                            drop(a);
                            ui.set_launcher_text("".into());
                            refresh(&app);
                            key(slint::platform::Key::Return.into());
                        }
                    } else {
                        check(false, "owned launch shortcut supplied");
                    }
                }
                6 => {
                    check(
                        !app.borrow().launcher.open,
                        "Enter launch dismisses launcher",
                    );
                    check(
                        RecentLaunches::load(&history_path())
                            .ids()
                            .first()
                            .is_some_and(|id| id == "fixture-launch"),
                        "accepted launch persisted as most recent",
                    );
                    // Notifications stay queued while actively typing.
                    toggle(&app);
                    {
                        let mut a = app.borrow_mut();
                        let now = a.now();
                        a.coordinator.push(
                            NotificationRecord {
                                id: 9,
                                app_id: "fixture".into(),
                                app_name: "Fixture".into(),
                                title: "Queued".into(),
                                body: String::new(),
                                created_ticks: 0,
                                icon: None,
                            },
                            now,
                        );
                    }
                    sync_presentation(&app);
                    check(
                        matches!(
                            app.borrow().presentation.as_ref().map(|p| &p.content),
                            Some(Content::AppLauncher)
                        ),
                        "notification cannot replace active launcher",
                    );
                    dismiss(&app, true);
                    {
                        let mut a = app.borrow_mut();
                        let now = a.now();
                        a.coordinator.clear_notifications(now);
                    }
                    sync_presentation(&app);
                    let prefs = app.borrow().preferences.upgrade();
                    if let Some(prefs) = prefs {
                        prefs.set_launcher_enabled(true);
                        prefs.set_launcher_shortcut(2);
                        prefs.invoke_changed();
                        check(
                            app.borrow().settings.launcher_shortcut == 2
                                && Settings::parse(&app.borrow().settings.encode())
                                    .launcher_shortcut
                                    == 2,
                            "settings callback changes and persists launcher chord",
                        );
                        check(
                            platform::configure_launcher_shortcut(false, 2).is_ok(),
                            "launcher shortcut can be disabled cleanly",
                        );
                    }
                }
                7 => {
                    frame.set(app.borrow().frames);
                }
                _ => {
                    check(
                        !app.borrow().launcher.open
                            && !app.borrow().timer.running()
                            && app.borrow().frames == frame.get(),
                        "dismissed launcher has no recurring spring frames",
                    );
                    check(
                        platform::diagnostic_snapshot().contains("noactivate=true"),
                        "ordinary notch remains nonactivating",
                    );
                    record("launcher verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}
