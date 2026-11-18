//! Keyboard launcher orchestration; shell work stays off the UI thread.
use super::*;
use crate::launcher::{LauncherShortcuts, RecentLaunches, search_with_shortcuts};
use crate::platform::launcher::{AppEntry, IconPixels, LaunchTarget};
use std::collections::{HashMap, VecDeque};

enum Update {
    Answer(u64, Result<Option<crate::launcher_answers::Answer>, String>),
    FileSearch(u64, Result<Vec<AppEntry>, String>),
    Failed(&'static str, String),
    Catalog(Result<platform::launcher::Catalog, String>),
    Icons(Vec<((String, u32), Option<IconPixels>)>),
    Launched(String, String, Result<(), String>),
    Revealed(String, Result<(), String>),
    Command(Result<(), String>),
}
#[derive(Clone)]
enum ContextAction {
    Open(String),
    Pin(String),
    Reveal(String),
    CopyPath(String),
    AddShortcut,
    EditShortcut(String),
    RemoveShortcut(String),
}
#[derive(Clone)]
enum LauncherResult {
    Answer,
    Command(String),
    Action {
        query: String,
        action: crate::launcher_actions::Action,
        timer_revision: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymbolQuery<'a> {
    Run(&'a str),
    Search(&'a str),
}

fn symbol_query(text: &str) -> Option<SymbolQuery<'_>> {
    let text = text.trim_start();
    if let Some(command) = text.strip_prefix('>') {
        Some(SymbolQuery::Run(command.trim()))
    } else {
        text.strip_prefix('@')
            .map(|name| SymbolQuery::Search(name.trim()))
    }
}

#[derive(Default)]
struct FileSearch {
    debounce: slint::Timer,
    pending: Option<String>,
    running: Option<u64>,
    matches: Vec<AppEntry>,
}

const FILE_RESULT_PREFIX: &str = "file:";
const MAX_FILE_RESULTS: usize = 6;

fn is_index_result(id: &str) -> bool {
    id.starts_with(FILE_RESULT_PREFIX)
}

impl FileSearch {
    fn reset(&mut self) {
        self.debounce.stop();
        self.pending = None;
        self.matches.clear();
    }
}

#[derive(Default)]
pub(super) struct Controller {
    result: Option<LauncherResult>,
    pub open: bool,
    pub(super) focus_epoch: u64,
    generation: u64,
    answer_cancel: Arc<AtomicBool>,
    answer_timer: slint::Timer,
    answer_running: bool,
    answer_pair: Option<String>,
    answer_pending: Option<String>,
    file_search: FileSearch,
    entries: Vec<AppEntry>,
    results: Vec<AppEntry>,
    recent: RecentLaunches,
    shortcuts: LauncherShortcuts,
    context_actions: Vec<ContextAction>,
    editing_shortcut: Option<String>,
    loaded: bool,
    indexed: Option<Instant>,
    loading: bool,
    icons_loading: bool,
    deferred_launch_error: Option<String>,
    shown_launch_error: bool,
    cache: HashMap<(String, u32), Option<slint::Image>>,
    inbox: Arc<Mutex<VecDeque<Update>>>,
}
fn history_path() -> std::path::PathBuf {
    Settings::path().with_file_name("launcher-recent.json")
}

fn shortcuts_path() -> std::path::PathBuf {
    Settings::path().with_file_name("launcher-shortcuts.json")
}

impl Controller {
    fn advance_query(&mut self, cancel_running_answer: bool) -> u64 {
        if cancel_running_answer {
            self.answer_cancel.store(true, Ordering::Release);
        }
        self.answer_timer.stop();
        self.answer_pending = None;
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn entry(&self, id: &str) -> Option<&AppEntry> {
        self.entries
            .iter()
            .chain(&self.file_search.matches)
            .find(|entry| entry.id == id)
    }

    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.recent = RecentLaunches::load(&history_path());
            self.shortcuts = LauncherShortcuts::load(&shortcuts_path());
            self.sync_shortcut_entries();
            self.loaded = true;
        }
    }

    fn sync_shortcut_entries(&mut self) {
        self.entries.retain(|entry| {
            !matches!(entry.target, LaunchTarget::Folder(_) | LaunchTarget::Url(_))
        });
        self.entries.extend(self.shortcuts.entries());
    }

    fn change_shortcuts<T>(
        &mut self,
        change: impl FnOnce(&mut LauncherShortcuts) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut updated = self.shortcuts.clone();
        let result = change(&mut updated)?;
        updated.save(&shortcuts_path())?;
        self.shortcuts = updated;
        self.sync_shortcut_entries();
        Ok(result)
    }
}

pub(super) fn wire_preferences(app: &Rc<RefCell<App>>, prefs: &SettingsWindow) {
    let weak = Rc::downgrade(app);
    prefs.on_clear_launcher_history(move || {
        if let Some(app) = weak.upgrade() {
            let result = {
                let mut a = app.borrow_mut();
                a.launcher.ensure_loaded();
                a.launcher.recent.clear();
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

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    let weak = Rc::downgrade(app);
    ui.on_launcher_height_changed(move || {
        let weak = weak.clone();
        // Model updates can change the measurement while the host is borrowed.
        slint::Timer::single_shot(Duration::ZERO, move || {
            let Some(app) = weak.upgrade() else { return };
            {
                let mut a = app.borrow_mut();
                if a.state != IslandState::AppLauncher || a.visibility.hiding || a.hidden {
                    return;
                }
                let target = a.pending_geometry.map_or(a.springs[1].target, |g| g[1]);
                if a.target_geometry(a.state).height == target {
                    return;
                }
                a.retarget_shape();
                if !a.animations_enabled() {
                    a.render();
                }
            }
            wake(&app);
        });
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_updated(move || {
        if let Some(app) = weak.upgrade() {
            receive(&app);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_query(move |_| {
        if let Some(app) = weak.upgrade() {
            query_changed(&app);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_navigate(move |delta| {
        if let Some(app) = weak.upgrade() {
            let a = app.borrow();
            if let Some(ui) = a.ui.upgrade()
                && !ui.get_launcher_actions_open()
                && !ui.get_launcher_shortcut_open()
            {
                let count = a.launcher.results.len() as i32;
                let offset = i32::from(ui.get_answer_visible());
                if count + offset > 0 {
                    ui.set_launcher_selected(
                        (ui.get_launcher_selected() + offset + delta).rem_euclid(count + offset)
                            - offset,
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
            if let Some(ui) = app.borrow().ui.upgrade()
                && back_from_actions(&ui)
            {
                return;
            }
            dismiss(&app, true);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_toggle_actions(move |index| {
        if let Some(app) = weak.upgrade() {
            toggle_actions(&app, index);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_action_navigate(move |delta| {
        if let Some(app) = weak.upgrade() {
            let a = app.borrow();
            let count = a.launcher.context_actions.len() as i32;
            if let Some(ui) = a.ui.upgrade()
                && ui.get_launcher_actions_open()
                && count > 0
            {
                ui.set_launcher_action_selected(
                    (ui.get_launcher_action_selected() + delta).rem_euclid(count),
                );
            }
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_action_activate(move |index| {
        if let Some(app) = weak.upgrade() {
            activate_action(&app, index);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_launcher_save_shortcut(move |alias, target| {
        if let Some(app) = weak.upgrade() {
            save_shortcut(&app, &alias, &target);
        }
    });
}

fn close_actions(ui: &IslandWindow) {
    let was_open = ui.get_launcher_actions_open() || ui.get_launcher_shortcut_open();
    ui.set_launcher_actions_open(false);
    ui.set_launcher_shortcut_open(false);
    ui.set_launcher_action_status("".into());
    if was_open {
        ui.invoke_launcher_focus();
    }
}

fn back_from_actions(ui: &IslandWindow) -> bool {
    if ui.get_launcher_shortcut_open() {
        ui.set_launcher_action_status("".into());
        ui.set_launcher_actions_open(true);
        ui.set_launcher_shortcut_open(false);
        ui.invoke_launcher_focus();
    } else if ui.get_launcher_actions_open() {
        close_actions(ui);
    } else {
        return false;
    }
    true
}

fn toggle_actions(app: &Rc<RefCell<App>>, index: i32) {
    let mut a = app.borrow_mut();
    let Some(ui) = a.ui.upgrade().filter(|_| a.launcher.open) else {
        return;
    };
    if back_from_actions(&ui) {
        return;
    }
    if ui.get_launcher_command_mode() {
        return;
    }
    let selected = usize::try_from(index)
        .ok()
        .and_then(|index| a.launcher.results.get(index));
    let title = selected.map_or_else(
        || "Launcher actions".to_owned(),
        |entry| format!("Actions for {}", entry.name),
    );
    let mut items = Vec::new();
    if let Some(entry) = selected {
        ui.set_launcher_selected(index);
        items.push(("Open", ContextAction::Open(entry.id.clone())));
        if is_index_result(&entry.id) {
            items.push(("Show in Explorer", ContextAction::Reveal(entry.id.clone())));
            items.push(("Copy path", ContextAction::CopyPath(entry.id.clone())));
        } else {
            let pin_label = if a.launcher.shortcuts.is_pinned(&entry.id) {
                "Unpin"
            } else {
                "Pin"
            };
            items.push((pin_label, ContextAction::Pin(entry.id.clone())));
            if matches!(
                entry.target,
                LaunchTarget::Folder(_) | LaunchTarget::Shortcut(_)
            ) {
                items.push(("Show in Explorer", ContextAction::Reveal(entry.id.clone())));
            }
            if a.launcher.shortcuts.shortcut(&entry.id).is_some() {
                items.push((
                    "Edit shortcut",
                    ContextAction::EditShortcut(entry.id.clone()),
                ));
                items.push((
                    "Remove shortcut",
                    ContextAction::RemoveShortcut(entry.id.clone()),
                ));
            }
        }
    }
    if !ui.get_launcher_file_mode() || selected.is_none() {
        items.push(("Add shortcut", ContextAction::AddShortcut));
    }
    let (labels, actions): (Vec<slint::SharedString>, Vec<_>) = items
        .into_iter()
        .map(|(label, action)| (label.into(), action))
        .unzip();
    a.launcher.context_actions = actions;
    ui.set_launcher_action_title(title.into());
    ui.set_launcher_action_items(slint::ModelRc::new(slint::VecModel::from(labels)));
    ui.set_launcher_action_selected(0);
    ui.set_launcher_actions_open(true);
    ui.invoke_launcher_focus();
}

fn activate_action(app: &Rc<RefCell<App>>, index: i32) {
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let (ui, action) = {
        let a = app.borrow();
        let Some(ui) =
            a.ui.upgrade()
                .filter(|ui| a.launcher.open && ui.get_launcher_actions_open())
        else {
            return;
        };
        let Some(action) = a.launcher.context_actions.get(index).cloned() else {
            return;
        };
        (ui, action)
    };
    match action {
        ContextAction::Open(ref id)
        | ContextAction::Reveal(ref id)
        | ContextAction::CopyPath(ref id) => {
            let entry = app.borrow().launcher.entry(id).cloned();
            let Some(entry) = entry else {
                ui.set_launcher_action_status("This item is no longer available.".into());
                return;
            };
            if matches!(action, ContextAction::Reveal(_)) {
                dismiss(app, false);
                start(app, "reveal", move || {
                    Update::Revealed(
                        entry.name,
                        platform::launcher::show_in_explorer(&entry.target),
                    )
                });
            } else if matches!(action, ContextAction::CopyPath(_)) {
                if let LaunchTarget::File(path) | LaunchTarget::Folder(path) = &entry.target {
                    let result = path
                        .to_str()
                        .ok_or_else(|| "This path contains unsupported characters.".to_owned())
                        .and_then(|path| {
                            platform::copy_text(path)
                                .map_err(|_| "Couldn’t copy the path. Try again.".to_owned())
                        });
                    if let Err(error) = result {
                        ui.set_launcher_action_status(error.into());
                    } else {
                        dismiss(app, true);
                    }
                }
            } else {
                launch_entry(app, entry);
            }
        }
        ContextAction::AddShortcut => open_shortcut_editor(app, &ui, None),
        ContextAction::EditShortcut(id) => open_shortcut_editor(app, &ui, Some(id)),
        ContextAction::Pin(id) => {
            let result = app
                .borrow_mut()
                .launcher
                .change_shortcuts(|shortcuts| shortcuts.toggle_pin(&id));
            finish_shortcut_change(app, &ui, result);
        }
        ContextAction::RemoveShortcut(id) => {
            let result = app.borrow_mut().launcher.change_shortcuts(|shortcuts| {
                shortcuts.remove_shortcut(&id);
                Ok(())
            });
            finish_shortcut_change(app, &ui, result);
        }
    }
}

fn finish_shortcut_change(app: &Rc<RefCell<App>>, ui: &IslandWindow, result: Result<(), String>) {
    match result {
        Ok(()) => {
            close_actions(ui);
            refresh(app);
        }
        Err(error) => ui.set_launcher_action_status(error.into()),
    }
}

fn open_shortcut_editor(app: &Rc<RefCell<App>>, ui: &IslandWindow, id: Option<String>) {
    let mut a = app.borrow_mut();
    let (alias, target) = if let Some(id) = &id {
        let Some((alias, target)) = a.launcher.shortcuts.shortcut(id) else {
            ui.set_launcher_action_status("This shortcut is no longer available.".into());
            return;
        };
        (
            slint::SharedString::from(alias),
            slint::SharedString::from(target),
        )
    } else {
        (
            slint::SharedString::default(),
            slint::SharedString::default(),
        )
    };
    ui.set_launcher_shortcut_editing(id.is_some());
    a.launcher.editing_shortcut = id;
    ui.set_launcher_action_status("".into());
    ui.set_launcher_shortcut_alias(alias);
    ui.set_launcher_shortcut_target(target);
    ui.set_launcher_shortcut_open(true);
    ui.set_launcher_actions_open(false);
    ui.invoke_launcher_focus();
}

fn save_shortcut(app: &Rc<RefCell<App>>, alias: &str, target: &str) {
    let Some(ui) = app
        .borrow()
        .ui
        .upgrade()
        .filter(|ui| ui.get_launcher_shortcut_open())
    else {
        return;
    };
    let result = {
        let mut a = app.borrow_mut();
        let previous = a.launcher.editing_shortcut.clone();
        a.launcher.change_shortcuts(|shortcuts| {
            shortcuts.put_shortcut(previous.as_deref(), alias, target)?;
            Ok(())
        })
    };
    match result {
        Ok(()) => {
            close_actions(&ui);
            let alias = alias.trim();
            let query = if symbol_query(alias).is_some() {
                format!("@{alias}")
            } else {
                alias.to_owned()
            };
            ui.set_launcher_text(query.into());
            query_changed(app);
        }
        Err(error) => ui.set_launcher_action_status(error.into()),
    }
}

pub(super) fn toggle(app: &Rc<RefCell<App>>) {
    toggle_with_focus(app, true);
}

pub(super) fn toggle_for_local_verification(app: &Rc<RefCell<App>>) {
    if verification_isolated_material() {
        toggle_with_focus(app, false);
    }
}

fn toggle_with_focus(app: &Rc<RefCell<App>>, acquire_foreground: bool) {
    app.borrow_mut().visibility.reveal_hover();
    if app.borrow().launcher.open {
        dismiss(app, true);
        return;
    }
    if app.borrow().visibility.locked || app.borrow().settings_visible {
        return;
    }
    pages::cancel(app);
    {
        let mut a = app.borrow_mut();
        a.hover_timer.stop();
        a.collapse_timer.stop();
        a.launcher.open = true;
        a.launcher.focus_epoch = a.launcher.focus_epoch.wrapping_add(1);
        a.launcher.ensure_loaded();
        let now = a.now();
        a.coordinator.open(Panel::AppLauncher, now);
    }
    sync_presentation(app);
    if acquire_foreground && let Err(error) = platform::set_keyboard_mode(true, false) {
        record(&format!("Launcher focus unavailable: {error}"));
        dismiss(app, false);
        settings_status(app, &error);
        return;
    }
    let launch_error = {
        let mut a = app.borrow_mut();
        let error = a.launcher.deferred_launch_error.take();
        a.launcher.shown_launch_error = error.is_some();
        error
    };
    if let Some(ui) = app.borrow().ui.upgrade() {
        close_actions(&ui);
        ui.set_launcher_text("".into());
        ui.set_launcher_selected(0);
        ui.set_launcher_command_mode(false);
        ui.set_launcher_file_mode(false);
        ui.set_launcher_file_status("".into());
        ui.set_answer_slow(false);
        ui.set_answer_editing(false);
        ui.set_answer_visible(false);
        ui.set_launcher_status(launch_error.unwrap_or_default().into());
        ui.invoke_launcher_focus();
    }
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(80), move || {
        if let Some(app) = weak.upgrade() {
            let a = app.borrow();
            if a.launcher.open
                && let Some(ui) = a.ui.upgrade()
                && !ui.get_launcher_actions_open()
                && !ui.get_launcher_shortcut_open()
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
        a.launcher.result = None;
        a.launcher.context_actions.clear();
        a.launcher.editing_shortcut = None;
        a.launcher.focus_epoch = a.launcher.focus_epoch.wrapping_add(1);
        a.launcher.advance_query(true);
        a.launcher.file_search.reset();
        a.launcher.results.clear();
        let now = a.now();
        a.coordinator.close(now);
        a.hover_intent.leave();
        a.hover_timer.stop();
        a.collapse_timer.stop();
    }
    if let Some(ui) = app.borrow().ui.upgrade() {
        close_actions(&ui);
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
            // Never hold the inbox lock during Shell/network work: receive runs on the UI thread.
            let update = task();
            inbox
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(update);
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
    let mut refresh_needed = false;
    let mut restore_focus = false;
    for event in events {
        refresh_needed |= !matches!(
            &event,
            Update::Launched(..)
                | Update::Revealed(..)
                | Update::Command(..)
                | Update::Failed("launch" | "reveal" | "command", _)
        );
        let mut a = app.borrow_mut();
        match event {
            Update::FileSearch(generation, result) => {
                a.launcher.file_search.running = None;
                if generation == a.launcher.generation
                    && a.launcher.open
                    && let Some(ui) = a.ui.upgrade()
                {
                    match result {
                        Ok(matches) => {
                            let status = if matches.is_empty() {
                                "No indexed files or folders found."
                            } else {
                                ""
                            };
                            a.launcher.file_search.matches = matches;
                            ui.set_launcher_file_status(status.into());
                        }
                        Err(error) => ui.set_launcher_file_status(error.into()),
                    }
                }
            }
            Update::Answer(generation, result) => {
                a.launcher.answer_running = false;
                a.launcher.answer_pair = None;
                if generation == a.launcher.generation
                    && a.launcher.open
                    && let Some(ui) = a.ui.upgrade()
                {
                    ui.set_answer_busy(false);
                    ui.set_answer_slow(false);
                    match result {
                        Ok(Some(answer)) => {
                            a.launcher.result = Some(LauncherResult::Answer);
                            ui.set_answer_action(false);
                            ui.set_answer_visible(true);
                            ui.set_answer_value(answer.value.into());
                            ui.set_answer_detail(answer.detail.into());
                            ui.set_answer_error(false);
                        }
                        Ok(None) => ui.set_answer_visible(false),
                        Err(_error) => {
                            ui.set_answer_visible(true);
                            ui.set_answer_error(true);
                            ui.set_answer_value("Unable to calculate".into());
                            ui.set_answer_detail(
                                "Check the expression or conversion and try again.".into(),
                            );
                        }
                    }
                }
            }
            Update::Failed(kind, error) => {
                match kind {
                    "catalog" => a.launcher.loading = false,
                    "icons" => a.launcher.icons_loading = false,
                    "file search" => {
                        let generation = a.launcher.file_search.running.take();
                        if generation == Some(a.launcher.generation)
                            && a.launcher.open
                            && let Some(ui) = a.ui.upgrade()
                        {
                            ui.set_launcher_file_status("Couldn’t search files. Try again.".into());
                        }
                        record(&format!("File search worker could not start: {error}"));
                        continue;
                    }
                    "answer" => {
                        a.launcher.answer_running = false;
                        if let Some(ui) = a.ui.upgrade() {
                            ui.set_answer_busy(false);
                            ui.set_answer_error(true);
                        }
                    }
                    "launch" | "reveal" => {
                        record(&format!("Launcher worker could not start: {error}"));
                        a.launcher.deferred_launch_error =
                            Some("Couldn’t open this item. Try again.".into());
                        restore_focus |= !a.launcher.open;
                        continue;
                    }
                    "command" => {
                        record("Launcher command worker could not start");
                        a.launcher.deferred_launch_error =
                            Some("Couldn’t start Command Prompt. Try again.".into());
                        restore_focus |= !a.launcher.open;
                        continue;
                    }
                    _ => {}
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
                        a.launcher.sync_shortcut_entries();
                        if !a.launcher.shown_launch_error
                            && let Some(ui) = a.ui.upgrade()
                        {
                            ui.set_launcher_status(catalog.warnings.join(" · ").into());
                        }
                    }
                    Err(error) => {
                        if !a.launcher.shown_launch_error
                            && let Some(ui) = a.ui.upgrade()
                        {
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
            Update::Launched(id, name, result) => match result {
                Ok(()) if !is_index_result(&id) => {
                    a.launcher.recent.record(&id);
                    if let Err(error) = a.launcher.recent.save(&history_path()) {
                        record(&format!("Launcher history could not be saved: {error}"));
                    }
                }
                Ok(()) => {}
                Err(error) => {
                    if is_index_result(&id) {
                        record("Launcher could not open an indexed item");
                    } else {
                        record(&format!("Launcher could not open {name}: {error}"));
                    }
                    a.launcher.deferred_launch_error = Some(if is_index_result(&id) {
                        format!("Couldn’t open {name}. Check that it’s still available.")
                    } else if a.launcher.shortcuts.shortcut(&id).is_some() {
                        format!("Couldn’t open {name}. Check the shortcut destination.")
                    } else {
                        format!("Couldn’t open {name}. Try again.")
                    });
                    restore_focus |= !a.launcher.open;
                }
            },
            Update::Revealed(name, Err(_)) => {
                record("Launcher could not show an item in Explorer");
                a.launcher.deferred_launch_error = Some(format!(
                    "Couldn’t show {name} in Explorer. Check that it’s available."
                ));
                restore_focus |= !a.launcher.open;
            }
            Update::Revealed(_, Ok(())) => {}
            Update::Command(Err(error)) => {
                a.launcher.deferred_launch_error = Some(error);
                restore_focus |= !a.launcher.open;
            }
            Update::Command(Ok(())) => {}
        }
    }
    if restore_focus {
        let _ = platform::set_keyboard_mode(false, true);
    }
    if refresh_needed {
        refresh(app);
        start_pending_answer(app);
        start_pending_files(app);
    }
}
fn icon_size(ui: &IslandWindow) -> u32 {
    (32. * ui.get_ui_scale() * ui.window().scale_factor())
        .round()
        .clamp(16., 128.) as u32
}
pub(super) fn refresh(app: &Rc<RefCell<App>>) {
    refresh_results(app, true);
}

fn refresh_results(app: &Rc<RefCell<App>>, preserve_selection: bool) {
    let Some(ui) = app.borrow().ui.upgrade() else {
        return;
    };
    let selected = ui.get_launcher_selected();
    let file_mode = ui.get_launcher_file_mode();
    let text = ui.get_launcher_text();
    let catalog_query = match symbol_query(&text) {
        Some(SymbolQuery::Run(_) | SymbolQuery::Search("")) => None,
        Some(SymbolQuery::Search(query)) => Some(query),
        None => Some(text.as_str()),
    };
    let size = icon_size(&ui);
    let mut requests: Vec<((String, u32), LaunchTarget)> = Vec::new();
    let rows = {
        let mut a = app.borrow_mut();
        let results = if file_mode {
            a.launcher.file_search.matches.clone()
        } else if let Some(query) = catalog_query {
            search_with_shortcuts(
                &a.launcher.entries,
                query,
                &a.launcher.recent,
                &a.launcher.shortcuts,
                a.settings.launcher_windows,
                if ui.get_answer_visible() { 4 } else { 6 },
            )
            .into_iter()
            .map(|index| a.launcher.entries[index].clone())
            .collect()
        } else {
            Vec::new()
        };
        if selected >= 0 {
            let previous = preserve_selection
                .then(|| a.launcher.results.get(selected as usize))
                .flatten();
            let position = previous
                .and_then(|row| results.iter().position(|entry| entry.id == row.id))
                .unwrap_or(0);
            ui.set_launcher_selected(position as i32);
        }
        ui.set_launcher_busy(!file_mode && catalog_query.is_some() && a.launcher.loading);
        let rows = results
            .iter()
            .map(|entry| {
                let use_native_icon = !file_mode
                    && !matches!(entry.target, LaunchTarget::Folder(_) | LaunchTarget::Url(_));
                let key = (entry.id.clone(), size);
                let icon = if use_native_icon {
                    a.launcher.cache.get(&key).and_then(|v| v.clone())
                } else {
                    None
                };
                if a.launcher.open
                    && !a.launcher.icons_loading
                    && use_native_icon
                    && !a.launcher.cache.contains_key(&key)
                {
                    requests.push((key, entry.target.clone()));
                }
                crate::LauncherResult {
                    id: entry.id.as_str().into(),
                    title: entry.name.as_str().into(),
                    detail: if use_native_icon
                        && a.launcher.recent.ids().contains(&entry.id)
                        && !entry.id.starts_with("windows:")
                    {
                        format!("Recently opened · {}", entry.detail).into()
                    } else {
                        entry.detail.as_str().into()
                    },
                    is_folder: matches!(entry.target, LaunchTarget::Folder(_)),
                    has_icon: icon.is_some(),
                    icon: icon.unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>();
        a.launcher.results = results;
        rows
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
    let can_activate = {
        let a = app.borrow();
        a.launcher.open
            && a.ui.upgrade().is_some_and(|ui| {
                !ui.get_launcher_actions_open()
                    && !ui.get_launcher_shortcut_open()
                    && (index == -1 || !ui.get_launcher_command_mode())
            })
    };
    if !can_activate {
        return;
    }
    if index == -1 {
        let result = app.borrow().launcher.result.clone();
        if let Some(LauncherResult::Command(command)) = result {
            let valid = app.borrow().ui.upgrade().is_some_and(|ui| {
                symbol_query(&ui.get_launcher_text()) == Some(SymbolQuery::Run(&command))
            });
            if !valid {
                query_changed(app);
                return;
            }
            dismiss(app, false);
            start(app, "command", move || {
                Update::Command(platform::launcher::run_command(&command))
            });
            return;
        }
        if let Some(LauncherResult::Action {
            query,
            action,
            timer_revision,
        }) = result
        {
            let valid = {
                let a = app.borrow();
                a.launcher.open
                    && a.ui
                        .upgrade()
                        .is_some_and(|ui| ui.get_launcher_text().as_str() == query)
                    && a.countdown_revision == timer_revision
                    && crate::launcher_actions::parse(&query) == Some(action.clone())
            };
            if !valid {
                query_changed(app);
                return;
            }
            dismiss(app, true);
            match action {
                crate::launcher_actions::Action::KeepVisible(value) => {
                    if app.borrow().visibility.keep_visible != value {
                        toggle_keep_visible(app);
                    }
                }
                crate::launcher_actions::Action::Timer(seconds) => {
                    {
                        let mut a = app.borrow_mut();
                        let now = a.now();
                        a.countdown_revision = a.countdown_revision.wrapping_add(1);
                        a.countdown.set_duration(seconds, now);
                        a.countdown.start(now);
                        a.countdown_engaged = true;
                        a.coordinator.clear_timer(now);
                        a.coordinator.close(now);
                    }
                    interaction::sync_countdown(app);
                }
            }
            return;
        }
        let ui = app.borrow().ui.upgrade();
        if let Some(ui) = ui
            && ui.get_answer_visible()
            && !ui.get_answer_busy()
            && !ui.get_answer_error()
        {
            if let Err(error) = platform::copy_text(&ui.get_answer_value()) {
                ui.set_launcher_status(error.into());
            } else {
                dismiss(app, true);
            }
        }
        return;
    }
    let entry = app.borrow().launcher.results.get(index as usize).cloned();
    if let Some(entry) = entry {
        launch_entry(app, entry);
    }
}

fn launch_entry(app: &Rc<RefCell<App>>, entry: AppEntry) {
    // Release focus and start the existing spring before asking Windows to launch.
    dismiss(app, false);
    // The isolated launcher harness keeps its owned Shell request pending long
    // enough to exercise reopening through the same activation/completion path.
    let verification_delay = app.borrow().verification;
    start(app, "launch", move || {
        if verification_delay {
            std::thread::sleep(Duration::from_millis(500));
        }
        Update::Launched(
            entry.id,
            entry.name,
            platform::launcher::launch(&entry.target),
        )
    });
}

pub(super) fn setup_verification(app: &Rc<RefCell<App>>) {
    let frame = Rc::new(std::cell::Cell::new(0));
    for (step, delay) in [
        500, 2500, 3100, 3400, 3700, 4100, 5000, 6500, 7500, 8500, 9500, 12000,
    ]
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
                                keywords: vec![],
                                name: "Launcher verification".into(),
                                detail: "Owned fixture".into(),
                                target: LaunchTarget::Shortcut(target),
                            }];
                            a.launcher.indexed = Some(Instant::now());
                            drop(a);
                            ui.set_launcher_text("".into());
                            refresh(&app);
                            key(slint::platform::Key::Return.into());
                            check(
                                !app.borrow().launcher.open,
                                "activation contracts synchronously before worker completion",
                            );
                            toggle(&app);
                            ui.set_launcher_text("fixture".into());
                            ui.invoke_launcher_query(ui.get_launcher_text());
                        }
                    } else {
                        check(false, "owned launch shortcut supplied");
                    }
                }
                6 => {
                    check(
                        app.borrow().launcher.open && ui.get_launcher_text() == "fixture",
                        "late launch completion preserves reopened launcher",
                    );
                    check(
                        RecentLaunches::load(&history_path())
                            .ids()
                            .first()
                            .is_some_and(|id| id == "fixture-launch"),
                        "accepted launch persisted as most recent",
                    );
                    let status = ui.get_launcher_status();
                    app.borrow()
                        .launcher
                        .inbox
                        .lock()
                        .unwrap()
                        .push_back(Update::Launched(
                            "fixture-failed".into(),
                            "Unavailable fixture".into(),
                            Err("owned failure".into()),
                        ));
                    ui.invoke_launcher_updated();
                    check(
                        app.borrow().launcher.open && ui.get_launcher_status() == status,
                        "late launch failure leaves current session intact",
                    );
                    dismiss(&app, false);
                    toggle(&app);
                    check(
                        ui.get_launcher_status() == "Couldn’t open Unavailable fixture. Try again."
                            && !app
                                .borrow()
                                .launcher
                                .recent
                                .ids()
                                .iter()
                                .any(|id| id == "fixture-failed"),
                        "failure deferred to next open and absent from history",
                    );
                    // Notifications stay queued while actively typing.
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
                    let prefs = settings_ui::ensure_preferences(&app);
                    check(prefs.is_some(), "launcher preferences initialize on demand");
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
                8 => {
                    toggle(&app);
                    ui.set_launcher_text("(50 + 20) / 2".into());
                    ui.invoke_launcher_query(ui.get_launcher_text());
                }
                9 => {
                    check(
                        ui.get_answer_value() == "35" && !ui.get_answer_busy(),
                        "fend answer card computes arithmetic",
                    );
                    let _ = crate::verification::snapshot(&ui, "launcher-answer");
                    ui.set_launcher_text("100 USD in USD".into());
                    ui.invoke_launcher_query(ui.get_launcher_text());
                }
                10 => {
                    check(
                        ui.get_answer_value() == "100 USD" && !ui.get_answer_error(),
                        "same currency converts locally without rates",
                    );
                    ui.invoke_launcher_activate(-1);
                    check(
                        !app.borrow().launcher.open,
                        "copying selected answer contracts notch",
                    );
                    let weak = Rc::downgrade(&app);
                    let frame = frame.clone();
                    slint::Timer::single_shot(Duration::from_millis(1500), move || {
                        if let Some(app) = weak.upgrade() {
                            frame.set(app.borrow().frames);
                        }
                    });
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

pub(super) fn query_changed(app: &Rc<RefCell<App>>) {
    let Some(ui) = app.borrow().ui.upgrade() else {
        return;
    };
    let query = ui.get_launcher_text().to_string();
    let symbol = symbol_query(&query);
    let exact_alias = symbol.is_none() && app.borrow().launcher.shortcuts.has_exact_alias(&query);
    let file_query = if symbol.is_some() || exact_alias {
        None
    } else {
        file_search_query(&query)
    };
    ui.set_launcher_command_mode(matches!(symbol, Some(SymbolQuery::Run(_))));
    ui.set_launcher_file_mode(file_query.is_some());
    ui.set_launcher_file_status("".into());
    ui.set_answer_slow(false);
    ui.set_answer_action(false);
    {
        let mut a = app.borrow_mut();
        a.launcher.result = None;
        a.launcher.file_search.reset();
    }
    if let Some(symbol) = symbol {
        let (result, label, editing) = match symbol {
            SymbolQuery::Run("") => (None, None, true),
            SymbolQuery::Run(command) => (
                Some(LauncherResult::Command(command.to_owned())),
                Some("Run in Command Prompt"),
                true,
            ),
            SymbolQuery::Search(name) => (None, None, name.is_empty()),
        };
        {
            let mut a = app.borrow_mut();
            a.launcher.advance_query(true);
            a.launcher.result = result;
        }
        show_immediate_action(&ui, label, editing);
        refresh_results(app, false);
        return;
    }
    if let Some(name) = file_query {
        query_files(app, &ui, name);
        return;
    }
    if !exact_alias && let Some(action) = crate::launcher_actions::parse(&query) {
        let label = {
            let mut a = app.borrow_mut();
            a.launcher.advance_query(true);
            a.launcher.result = Some(LauncherResult::Action {
                query: query.clone(),
                action: action.clone(),
                timer_revision: a.countdown_revision,
            });
            let now = a.now();
            let replacing = a.countdown_engaged && !a.countdown.snapshot(now).finished;
            action.label(replacing)
        };
        show_immediate_action(&ui, Some(&label), true);
        refresh_results(app, false);
        return;
    }
    let kind = if exact_alias {
        None
    } else {
        crate::launcher_answers::classify(&query)
    };
    let enabled = {
        let a = app.borrow();
        match kind {
            Some(crate::launcher_answers::Kind::Calculator) => a.settings.launcher_calculator,
            Some(crate::launcher_answers::Kind::Currency) => a.settings.launcher_currency,
            None => false,
        }
    };
    {
        let mut a = app.borrow_mut();
        let cancel_running_answer = !enabled
            || !a.launcher.answer_running
            || a.launcher.answer_pair.is_none()
            || a.launcher.answer_pair != crate::launcher_answers::currency_pair_key(&query);
        a.launcher.advance_query(cancel_running_answer);
    }
    ui.set_answer_editing(
        !exact_alias
            && (enabled
                || crate::launcher_actions::is_incomplete(&query)
                || (app.borrow().settings.launcher_calculator
                    && crate::launcher_answers::is_incomplete(&query))),
    );
    ui.set_answer_visible(false);
    ui.set_answer_busy(enabled);
    ui.set_answer_error(false);
    ui.set_answer_value("".into());
    ui.set_answer_detail("".into());
    ui.set_launcher_selected(if enabled { -1 } else { 0 });
    refresh_results(app, false);
    if enabled {
        let weak = Rc::downgrade(app);
        app.borrow().launcher.answer_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(180),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.borrow_mut().launcher.answer_pending = Some(query.clone());
                    start_pending_answer(&app);
                }
            },
        );
    }
}

fn show_immediate_action(ui: &IslandWindow, label: Option<&str>, editing: bool) {
    ui.set_answer_editing(editing);
    ui.set_answer_action(label.is_some());
    ui.set_answer_visible(label.is_some());
    ui.set_answer_busy(false);
    ui.set_answer_error(false);
    ui.set_answer_value(label.unwrap_or_default().into());
    ui.set_answer_detail("".into());
    ui.set_launcher_selected(if label.is_some() { -1 } else { 0 });
}

fn file_search_query(query: &str) -> Option<&str> {
    let query = query.trim();
    let (prefix, name) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
    prefix.eq_ignore_ascii_case("file").then(|| name.trim())
}

#[cfg(test)]
mod query_tests {
    use super::{SymbolQuery, file_search_query, symbol_query};

    #[test]
    fn symbol_queries_keep_the_body_in_its_explicit_scope() {
        assert_eq!(symbol_query(" > "), Some(SymbolQuery::Run("")));
        assert_eq!(symbol_query("@"), Some(SymbolQuery::Search("")));
        assert_eq!(
            symbol_query("  > echo \"a & b\" | findstr b "),
            Some(SymbolQuery::Run("echo \"a & b\" | findstr b"))
        );
        assert_eq!(
            symbol_query("@  file explorer"),
            Some(SymbolQuery::Search("file explorer"))
        );
        assert_eq!(symbol_query("@> foo"), Some(SymbolQuery::Search("> foo")));
        assert_eq!(symbol_query("@@foo"), Some(SymbolQuery::Search("@foo")));
        assert_eq!(symbol_query("file invoice"), None);
        assert_eq!(symbol_query("2 > 1"), None);
    }

    #[test]
    fn file_scope_requires_the_complete_prefix() {
        assert_eq!(file_search_query("file"), Some(""));
        assert_eq!(
            file_search_query(" FILE\t résumé .pdf "),
            Some("résumé .pdf")
        );
        assert_eq!(file_search_query("file invoice"), Some("invoice"));
        assert_eq!(file_search_query("files invoice"), None);
        assert_eq!(file_search_query("file.pdf"), None);
        assert_eq!(file_search_query("open file invoice"), None);
    }
}

fn query_files(app: &Rc<RefCell<App>>, ui: &IslandWindow, name: &str) {
    let generation = app.borrow_mut().launcher.advance_query(true);
    show_immediate_action(ui, None, false);
    refresh_results(app, false);
    if name.is_empty() {
        ui.set_launcher_file_status("Type a file or folder name.".into());
        return;
    }

    let name = name.to_owned();
    let weak = Rc::downgrade(app);
    app.borrow().launcher.file_search.debounce.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(160),
        move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            {
                let mut a = app.borrow_mut();
                if !a.launcher.open || a.launcher.generation != generation {
                    return;
                }
                a.launcher.file_search.pending = Some(name.clone());
            }
            start_pending_files(&app);
        },
    );
}

fn start_pending_files(app: &Rc<RefCell<App>>) {
    let (generation, name) = {
        let mut a = app.borrow_mut();
        if !a.launcher.open || a.launcher.file_search.running.is_some() {
            return;
        }
        let Some(name) = a.launcher.file_search.pending.take() else {
            return;
        };
        let generation = a.launcher.generation;
        a.launcher.file_search.running = Some(generation);
        (generation, name)
    };
    start(app, "file search", move || {
        let results = platform::file_search::search(&name, MAX_FILE_RESULTS)
            .map(|matches| matches.into_iter().filter_map(index_entry).collect());
        Update::FileSearch(generation, results)
    });
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(400), move || {
        if let Some(app) = weak.upgrade() {
            let a = app.borrow();
            if a.launcher.open
                && a.launcher.generation == generation
                && a.launcher.file_search.running == Some(generation)
                && let Some(ui) = a.ui.upgrade()
            {
                ui.set_launcher_file_status("Searching indexed files…".into());
            }
        }
    });
}

fn index_entry(item: platform::file_search::SearchMatch) -> Option<AppEntry> {
    let path = item.path;
    Some(AppEntry {
        id: format!("{FILE_RESULT_PREFIX}{}", path.display()),
        name: path.file_name()?.to_string_lossy().into_owned(),
        detail: path.parent()?.to_string_lossy().into_owned(),
        keywords: Vec::new(),
        target: if item.is_folder {
            LaunchTarget::Folder(path)
        } else {
            LaunchTarget::File(path)
        },
    })
}

fn start_pending_answer(app: &Rc<RefCell<App>>) {
    let job = {
        let mut a = app.borrow_mut();
        if a.launcher.answer_running || !a.launcher.open {
            return;
        }
        let Some(query) = a.launcher.answer_pending.take() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        a.launcher.answer_cancel = cancel.clone();
        a.launcher.answer_running = true;
        a.launcher.answer_pair = crate::launcher_answers::currency_pair_key(&query);
        (query, cancel, a.launcher.generation)
    };
    start(app, "answer", move || {
        Update::Answer(job.2, crate::launcher_answers::evaluate(&job.0, &job.1))
    });
    let generation = app.borrow().launcher.generation;
    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(350), move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let a = app.borrow();
        if a.launcher.open
            && a.launcher.generation == generation
            && a.launcher.answer_running
            && a.launcher.answer_pair.is_some()
        {
            if let Some(ui) = a.ui.upgrade() {
                ui.set_answer_slow(true);
            }
        }
    });
}
