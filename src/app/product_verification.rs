//! Owned UI regression. Uses application callbacks, never global input injection.
use super::*;

fn check(ok: bool, label: &str) {
    record(&format!(
        "product {} {label}",
        if ok { "PASS" } else { "FAILED" }
    ));
}

fn query(ui: &IslandWindow, value: &str) {
    ui.set_launcher_text(value.into());
    ui.invoke_launcher_query(value.into());
}

fn capture(ui: &IslandWindow, name: &str) {
    if let Err(error) = crate::verification::snapshot(ui, name) {
        record(&format!("product FAILED snapshot {name}: {error}"));
    }
}

fn key(ui: &IslandWindow, text: impl Into<slint::SharedString>) {
    use slint::platform::WindowEvent;

    let text = text.into();
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn actions_chord(ui: &IslandWindow) {
    use slint::platform::{Key, WindowEvent};

    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(ui, "k");
    ui.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    use slint::{Model, platform::Key};

    if let Ok(marker) = std::env::var("STILL_ISLAND_VERIFY_PREFIX_MARKER") {
        setup_launcher_prefixes(app, std::path::Path::new(&marker));
        return;
    }

    if let Ok(path) = std::env::var("STILL_ISLAND_VERIFY_FILE_SEARCH_PATH") {
        setup_file_search(app, std::path::Path::new(&path));
        return;
    }

    if let (Ok(folder), Ok(url)) = (
        std::env::var("STILL_ISLAND_VERIFY_ALIAS_FOLDER"),
        std::env::var("STILL_ISLAND_VERIFY_ALIAS_URL"),
    ) {
        setup_alias_handoffs(app, folder, url);
        return;
    }

    for step in 0..17 {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(600 + step * 1100), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            match step {
                0 => show_preferences(&app),
                1..=5 => {
                    let Some(prefs) = app.borrow().preferences() else {
                        check(false, "settings exists");
                        return;
                    };
                    let index = (step - 1) as i32;
                    prefs.set_active_tab(index);
                    if index == 1 {
                        prefs.set_hide_on_hover(true);
                        prefs.set_hover_only_maximized(true);
                        prefs.invoke_changed();
                        check(
                            app.borrow().settings.hover_only_maximized,
                            "conditional checkbox reaches settings",
                        );
                    }
                    refresh_preferences(&app);
                    check(
                        prefs.get_active_tab() == index,
                        "settings selection survives refresh",
                    );
                    match crate::verification::snapshot_preferences(&prefs) {
                        Ok(path) => {
                            let _ = std::fs::copy(
                                &path,
                                path.with_file_name(format!("product-settings-{index}.ppm")),
                            );
                        }
                        Err(error) => record(&format!("product FAILED settings snapshot {error}")),
                    }
                    if index == 4 {
                        close_preferences(&app, true);
                        let mut settings = app.borrow().settings.clone();
                        settings.hide_on_hover = false;
                        settings.animations = false;
                        apply_settings(&app, settings, false);
                    }
                }
                6 => {
                    check(
                        pages::available(&app)
                            .iter()
                            .all(|(_, label)| label != "Downloads"),
                        "Downloads is absent from page navigation",
                    );
                    pages::open(&app, crate::navigation::Page::Home);
                    for page in [
                        crate::navigation::Page::Media,
                        crate::navigation::Page::Timer,
                        crate::navigation::Page::Volume,
                    ] {
                        app.borrow_mut().page_wheel.reset();
                        let result = page_verification::send_wheel(
                            &app,
                            (f64::from(ui.get_canvas_width()) / 2., 16.),
                        );
                        check(
                            matches!(result, slint::winit_030::EventResult::PreventDefault)
                                && app.borrow().settings.selected_page.as_ref() == Some(&page),
                            "header wheel reaches the next view",
                        );
                    }
                    pages::open(&app, crate::navigation::Page::Home);
                    let mut settings = app.borrow().settings.clone();
                    settings.text_scale = 1.4;
                    apply_settings(&app, settings, false);
                    launcher::toggle(&app);
                    query(&ui, "10 minutes");
                    capture(&ui, "product-timer-action");
                    check(
                        ui.get_answer_value() == "Start 10-minute timer",
                        "duration offers timer action",
                    );
                    ui.invoke_launcher_activate(-1);
                    let mut a = app.borrow_mut();
                    let now = a.now();
                    let timer = a.countdown.snapshot(now);
                    check(
                        timer.running && timer.duration_seconds == 600,
                        "activation starts countdown",
                    );
                    check(!a.launcher.open, "timer activation dismisses launcher");
                }
                7 => {
                    launcher::toggle(&app);
                    query(&ui, "timer 45s");
                    check(
                        ui.get_answer_value() == "Replace timer with 45 seconds",
                        "replacement is explicit",
                    );
                    ui.invoke_timer_set_duration(120);
                    ui.invoke_launcher_activate(-1);
                    check(
                        app.borrow().launcher.open,
                        "stale timer action refreshes before execution",
                    );
                    ui.invoke_launcher_activate(-1);
                    let mut a = app.borrow_mut();
                    let now = a.now();
                    check(
                        a.countdown.snapshot(now).duration_seconds == 45,
                        "refreshed replacement uses new duration",
                    );
                }
                8 => {
                    launcher::toggle(&app);
                    query(&ui, "keep visible");
                    ui.invoke_launcher_activate(-1);
                    check(
                        app.borrow().visibility.keep_visible,
                        "launcher enables shared visibility hold",
                    );
                    launcher::toggle(&app);
                    query(&ui, "resume auto hide");
                    ui.invoke_launcher_activate(-1);
                    check(
                        !app.borrow().visibility.keep_visible,
                        "launcher releases shared visibility hold",
                    );
                    ui.invoke_timer_set_duration(1);
                    ui.invoke_timer_toggle();
                    {
                        let mut a = app.borrow_mut();
                        let now = a.now();
                        a.coordinator.open(Panel::Media, now);
                        a.coordinator.toggle_pin(now);
                    }
                    sync_presentation(&app);
                }
                9 => {
                    interaction::sync_countdown(&app);
                    check(
                        ui.get_timer_alert() && app.borrow().coordinator.pinned(),
                        "completion preserves pinned panel and offers alert",
                    );
                    capture(&ui, "product-panel-alert");
                    ui.invoke_open_timer();
                    check(
                        !ui.get_timer_alert(),
                        "opening timer acknowledges completion",
                    );
                    capture(&ui, "product-timer-complete");
                }
                10 => {
                    launcher::toggle(&app);
                    query(&ui, "zz-still-island-empty-fixture");
                    ui.set_launcher_status("Couldn’t open fixture. Try again.".into());
                    capture(&ui, "product-launcher-status");
                    actions_chord(&ui);
                    check(
                        ui.get_launcher_actions_open(),
                        "Ctrl+K opens actions through the launcher key handler",
                    );
                }
                11 => {
                    capture(&ui, "product-launcher-empty-actions");
                    key(&ui, Key::Return);
                    check(
                        ui.get_launcher_shortcut_open(),
                        "Enter opens the shortcut editor",
                    );
                }
                12 => {
                    key(&ui, "launcher fixture");
                    check(
                        ui.get_launcher_shortcut_alias() == "launcher fixture",
                        "keyboard input reaches the alias field",
                    );
                    key(&ui, Key::Return);
                    key(&ui, "file:///C:/Windows");
                    key(&ui, Key::Return);
                    check(
                        ui.get_launcher_shortcut_open()
                            && !ui.get_launcher_action_status().is_empty(),
                        "unsupported URL shows an error in the focused editor",
                    );
                }
                13 => {
                    capture(&ui, "product-launcher-shortcut-error");
                    ui.set_launcher_shortcut_target("https://example.com/docs".into());
                    key(&ui, Key::Return);
                    check(
                        !ui.get_launcher_shortcut_open()
                            && ui
                                .get_launcher_results()
                                .row_data(0)
                                .is_some_and(|row| row.title == "launcher fixture"),
                        "saving restores search with the new alias selected",
                    );
                }
                14 => {
                    actions_chord(&ui);
                    check(
                        ui.get_launcher_action_items().row_data(1).as_deref() == Some("Pin"),
                        "shortcut exposes contextual pin action",
                    );
                    key(&ui, Key::DownArrow);
                    key(&ui, Key::Return);
                    query(&ui, "");
                    check(
                        !ui.get_launcher_actions_open()
                            && ui
                                .get_launcher_results()
                                .row_data(0)
                                .is_some_and(|row| row.title == "launcher fixture"),
                        "keyboard pin puts the shortcut first in blank search",
                    );
                    actions_chord(&ui);
                }
                15 => {
                    capture(&ui, "product-launcher-shortcut-actions");
                    let options = ui.get_launcher_action_items();
                    check(
                        options.row_data(1).as_deref() == Some("Unpin"),
                        "a pinned result exposes Unpin",
                    );
                    let saved = crate::launcher::LauncherShortcuts::load(
                        &Settings::path().with_file_name("launcher-shortcuts.json"),
                    );
                    let expected = saved
                        .entries()
                        .into_iter()
                        .find(|entry| entry.name == "launcher fixture");
                    check(
                        expected.is_some_and(|entry| saved.is_pinned(&entry.id)),
                        "shortcut and pin persist in the isolated profile",
                    );
                    if let Some(edit) = options.iter().position(|label| label == "Edit shortcut") {
                        ui.invoke_launcher_action_activate(edit as i32);
                    }
                    check(
                        ui.get_launcher_shortcut_open()
                            && ui.get_launcher_shortcut_target() == "https://example.com/docs",
                        "edit opens with the existing alias destination",
                    );
                    key(&ui, Key::Escape);
                    check(
                        ui.get_launcher_actions_open(),
                        "Escape returns to the context menu",
                    );
                    key(&ui, Key::Escape);
                    check(
                        !ui.get_launcher_actions_open() && app.borrow().launcher.open,
                        "Escape returns to search",
                    );
                    key(&ui, Key::Escape);
                    check(!app.borrow().launcher.open, "Escape dismisses the launcher");
                }
                16 => {
                    record("product verification complete");
                    let _ = slint::quit_event_loop();
                }
                _ => unreachable!(),
            }
        });
    }
}

fn setup_alias_handoffs(app: &Rc<RefCell<App>>, folder: String, url: String) {
    use slint::{
        Model,
        platform::{Key, WindowEvent},
    };

    const FOLDER_ALIAS: &str = "handoff fixture folder";
    const URL_ALIAS: &str = "handoff fixture website";
    if !verification_isolated_material()
        || !url.starts_with("http://127.0.0.1:")
        || !std::path::Path::new(&folder).is_dir()
    {
        check(
            false,
            "alias handoffs require an isolated fixture folder and loopback page",
        );
        let _ = slint::quit_event_loop();
        return;
    }

    let weak = Rc::downgrade(app);
    slint::Timer::single_shot(Duration::from_millis(900), move || {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let Some(ui) = app.borrow().ui.upgrade() else {
            return;
        };
        ui.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        launcher::toggle_for_local_verification(&app);
        for (alias, target) in [(FOLDER_ALIAS, &folder), (URL_ALIAS, &url)] {
            actions_chord(&ui);
            let options = ui.get_launcher_action_items();
            if let Some(add) = options.iter().position(|label| label == "Add shortcut") {
                ui.invoke_launcher_action_activate(add as i32);
            }
            ui.invoke_launcher_save_shortcut(alias.into(), target.as_str().into());
            let saved = !ui.get_launcher_shortcut_open()
                && ui
                    .get_launcher_results()
                    .row_data(0)
                    .is_some_and(|row| row.title == alias);
            check(saved, &format!("saved {alias} through the launcher"));
            if !saved {
                return;
            }
        }

        // Each activation uses the real Shell worker. Key events stay
        // inside the fixture even after Explorer or the browser takes focus.
        for alias in [FOLDER_ALIAS, URL_ALIAS] {
            if !app.borrow().launcher.open {
                launcher::toggle_for_local_verification(&app);
            }
            query(&ui, "");
            key(&ui, alias);
            let selected = app.borrow().launcher.open
                && ui
                    .get_launcher_results()
                    .row_data(0)
                    .is_some_and(|row| row.title == alias);
            check(selected, &format!("typing selects {alias}"));
            if !selected {
                return;
            }
            key(&ui, Key::Return);
            check(
                !app.borrow().launcher.open,
                &format!("Enter activates {alias}"),
            );
        }
    });

    slint::Timer::single_shot(Duration::from_secs(7), move || {
        let path = Settings::path();
        let saved = crate::launcher::LauncherShortcuts::load(
            &path.with_file_name("launcher-shortcuts.json"),
        );
        let history =
            crate::launcher::RecentLaunches::load(&path.with_file_name("launcher-recent.json"));
        let entries = saved.entries();
        for alias in [FOLDER_ALIAS, URL_ALIAS] {
            check(
                entries
                    .iter()
                    .any(|entry| entry.name == alias && history.ids().contains(&entry.id)),
                &format!("Windows accepted {alias}"),
            );
        }
        record("product verification complete");
        let _ = slint::quit_event_loop();
    });
}

fn finish_verification() {
    record("product verification complete");
    let _ = slint::quit_event_loop();
}

fn open_file_search_actions(ui: &IslandWindow, name: &str, parent: &str) -> bool {
    use slint::{Model, platform::Key};

    let results = ui.get_launcher_results();
    let index = results.iter().position(|row| {
        row.title.as_str().eq_ignore_ascii_case(name)
            && row.detail.as_str().eq_ignore_ascii_case(parent)
    });
    let Some(index) = index else {
        check(
            false,
            &format!(
                "Windows Search returned the fixture with its exact parent folder (status {:?})",
                ui.get_launcher_file_status()
            ),
        );
        return false;
    };
    check(
        true,
        "Windows Search returned the fixture with its exact parent folder",
    );

    let steps = (index as i32 - ui.get_launcher_selected()).rem_euclid(results.row_count() as i32);
    for _ in 0..steps {
        key(ui, Key::DownArrow);
    }
    if ui.get_launcher_selected() != index as i32 {
        check(false, "keyboard selection reached the indexed fixture");
        return false;
    }
    actions_chord(ui);
    let actions = ui.get_launcher_action_items();
    let expected = ["Open", "Show in Explorer", "Copy path"];
    let menu_matches = ui.get_launcher_actions_open()
        && actions.row_count() == expected.len()
        && expected
            .iter()
            .all(|label| actions.iter().any(|action| action.as_str() == *label));
    check(
        menu_matches,
        "Ctrl+K shows Open, Show in Explorer and Copy path for the indexed result",
    );
    menu_matches
}

fn activate_file_search_action(ui: &IslandWindow, label: &str) -> bool {
    use slint::{Model, platform::Key};

    let actions = ui.get_launcher_action_items();
    let Some(index) = actions.iter().position(|action| action.as_str() == label) else {
        check(
            false,
            &format!("{label} remains available for the indexed fixture"),
        );
        return false;
    };
    let steps =
        (index as i32 - ui.get_launcher_action_selected()).rem_euclid(actions.row_count() as i32);
    for _ in 0..steps {
        key(ui, Key::DownArrow);
    }
    if ui.get_launcher_action_selected() != index as i32 {
        check(false, &format!("keyboard selection reached {label}"));
        return false;
    }
    key(ui, Key::Return);
    check(true, &format!("{label} activated from the action menu"));
    true
}

fn setup_file_search(app: &Rc<RefCell<App>>, path: &std::path::Path) {
    use slint::{
        Model,
        platform::{Key, WindowEvent},
    };
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;

    let Some((name, parent)) = path.file_name().zip(path.parent()) else {
        check(false, "file search fixture has a name and parent folder");
        finish_verification();
        return;
    };
    if !verification_isolated_material() || !path.is_absolute() || !path.exists() {
        check(
            false,
            "file search requires an existing absolute path and isolated fixture",
        );
        finish_verification();
        return;
    }
    let name = name.to_string_lossy().into_owned();
    let text = format!("file {name}");
    let parent = parent.to_string_lossy().into_owned();
    let reveal = std::env::var_os("STILL_ISLAND_VERIFY_FILE_SEARCH_REVEAL").is_some();
    let clipboard_sequence = std::env::var("STILL_ISLAND_VERIFY_FILE_SEARCH_CLIPBOARD")
        .ok()
        .and_then(|sequence| sequence.parse::<u32>().ok());

    // Allow the unmatched index query to finish in either fixture mode.
    let checkpoints = [900, 3_400, if reveal { 5_900 } else { 10_900 }, 13_400];
    for (step, at) in checkpoints.into_iter().enumerate() {
        let weak = Rc::downgrade(app);
        let (name, parent, text) = (name.clone(), parent.clone(), text.clone());
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            let submit_query = |ui: &IslandWindow| {
                query(ui, "");
                key(ui, text.as_str());
                check(
                    ui.get_launcher_file_mode(),
                    "typing file and a name enters Windows Search",
                );
                check(
                    ui.get_launcher_results().row_count() == 0,
                    "waiting for Windows Search clears previous results",
                );
            };
            let submit_missing = |ui: &IslandWindow| {
                query(
                    ui,
                    &format!("file still-island-missing-fixture-{}", std::process::id()),
                );
                check(
                    ui.get_launcher_file_mode() && ui.get_launcher_results().row_count() == 0,
                    "changing the file query clears the previous match immediately",
                );
            };
            match step {
                0 => {
                    ui.window()
                        .dispatch_event(WindowEvent::WindowActiveChanged(true));
                    launcher::toggle_for_local_verification(&app);
                    key(&ui, "file");
                    check(
                        ui.get_launcher_file_mode() && ui.get_launcher_results().row_count() == 0,
                        "file without a name waits without showing unrelated results",
                    );
                    submit_query(&ui);
                }
                1 => {
                    capture(&ui, "product-file-search-result");
                    if !open_file_search_actions(&ui, &name, &parent) {
                        finish_verification();
                        return;
                    }
                    capture(&ui, "product-file-search-actions");
                    if let Some(sequence) = clipboard_sequence {
                        // SAFETY: querying a sequence number doesn't open or alter the clipboard.
                        if unsafe { GetClipboardSequenceNumber() } == sequence {
                            if !activate_file_search_action(&ui, "Copy path") {
                                finish_verification();
                                return;
                            }
                        } else {
                            record(
                                "product SKIP Copy path because the clipboard changed during the run",
                            );
                        }
                    }
                    if ui.get_launcher_actions_open() {
                        key(&ui, Key::Escape);
                    }
                    if !app.borrow().launcher.open {
                        launcher::toggle_for_local_verification(&app);
                    }
                    if reveal {
                        submit_query(&ui);
                    } else {
                        submit_missing(&ui);
                    }
                }
                2 if reveal => {
                    if !open_file_search_actions(&ui, &name, &parent)
                        || !activate_file_search_action(&ui, "Show in Explorer")
                    {
                        finish_verification();
                        return;
                    }
                    if !app.borrow().launcher.open {
                        launcher::toggle_for_local_verification(&app);
                    }
                    submit_missing(&ui);
                }
                _ => {
                    check(
                        ui.get_launcher_file_mode()
                            && ui.get_launcher_results().row_count() == 0
                            && ui.get_launcher_file_status()
                                == "No indexed files or folders found.",
                        "the replacement query reports a settled empty state without stale rows",
                    );
                    capture(&ui, "product-file-search-empty");
                    query(&ui, "");
                    check(
                        !ui.get_launcher_file_mode(),
                        "clearing the query returns to app search",
                    );
                    finish_verification();
                }
            }
        });
    }
}

fn setup_launcher_prefixes(app: &Rc<RefCell<App>>, marker: &std::path::Path) {
    use slint::{
        Model,
        platform::{Key, WindowEvent},
    };

    const ALIAS: &str = "file catalog prefix fixture";
    const OUTPUT: &str = "still-island-prefix-verified";
    const COMMAND: &str =
        r#"> echo still-island-prefix-verified > "%STILL_ISLAND_VERIFY_PREFIX_MARKER%" & exit /b"#;

    let directory = std::env::current_dir().ok();
    if !verification_isolated_material()
        || !marker.is_absolute()
        || marker.parent() != directory.as_deref()
        || !marker
            .file_name()
            .is_some_and(|name| name == "prefix-command-marker.txt")
        || !matches!(marker.try_exists(), Ok(false))
    {
        check(
            false,
            "launcher prefixes require an isolated fixture with a fresh marker path",
        );
        finish_verification();
        return;
    }

    let marker = marker.to_path_buf();
    for (step, at) in [900, 4_400, 5_900, 9_400].into_iter().enumerate() {
        let weak = Rc::downgrade(app);
        let marker = marker.clone();
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(ui) = app.borrow().ui.upgrade() else {
                return;
            };
            let type_query = |value| {
                query(&ui, "");
                key(&ui, value);
            };
            match step {
                0 => {
                    ui.window()
                        .dispatch_event(WindowEvent::WindowActiveChanged(true));
                    launcher::toggle_for_local_verification(&app);
                    actions_chord(&ui);
                    let actions = ui.get_launcher_action_items();
                    if let Some(add) = actions.iter().position(|label| label == "Add shortcut") {
                        ui.invoke_launcher_action_activate(add as i32);
                    }
                    let target = marker.parent().unwrap().to_string_lossy();
                    ui.invoke_launcher_save_shortcut(ALIAS.into(), target.as_ref().into());
                    check(
                        !ui.get_launcher_shortcut_open() && ui.get_launcher_text() == ALIAS,
                        "saved an isolated shortcut with a file-prefixed name",
                    );
                    for prefix in ["@", ">"] {
                        type_query(prefix);
                        let empty =
                            ui.get_launcher_results().row_count() == 0 && !ui.get_answer_visible();
                        check(empty, &format!("bare {prefix} offers no result"));
                        if empty {
                            key(&ui, Key::Return);
                            check(
                                app.borrow().launcher.open,
                                &format!("Enter on bare {prefix} keeps the launcher open"),
                            );
                        }
                    }
                }
                1 => {
                    for (input, title) in [
                        ("@windows update", "Windows Update"),
                        ("@file catalog prefix fixture", ALIAS),
                        ("@file explorer", "File Explorer"),
                    ] {
                        type_query(input);
                        check(
                            !ui.get_launcher_file_mode()
                                && !ui.get_answer_visible()
                                && ui
                                    .get_launcher_results()
                                    .iter()
                                    .any(|row| row.title.as_str().eq_ignore_ascii_case(title)),
                            &format!("{input} returns {title} from the catalog"),
                        );
                    }
                    capture(&ui, "product-prefix-catalog");
                    type_query("@3 + 4");
                    check(
                        !ui.get_launcher_file_mode() && !ui.get_answer_visible(),
                        "@ keeps a calculation in catalog search",
                    );
                    type_query(COMMAND);
                }
                2 => {
                    let ready = ui.get_launcher_text() == COMMAND
                        && ui.get_launcher_selected() == -1
                        && ui.get_answer_visible()
                        && ui.get_answer_action()
                        && ui.get_answer_value() == "Run in Command Prompt";
                    let unexecuted = matches!(marker.try_exists(), Ok(false));
                    check(ready, "typing a command offers the Command Prompt action");
                    check(
                        unexecuted,
                        "typing a command leaves the marker absent before Enter",
                    );
                    capture(&ui, "product-prefix-command");
                    if ready && unexecuted {
                        key(&ui, Key::Return);
                    }
                }
                _ => {
                    let output = std::fs::read_to_string(&marker);
                    check(
                        output.is_ok_and(|contents| contents.trim() == OUTPUT),
                        "Enter runs the compound command and writes the exact marker",
                    );
                    finish_verification();
                }
            }
        });
    }
}
