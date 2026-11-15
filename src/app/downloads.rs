use super::*;
use crate::services::downloads::{Config, Entry, Service};
use std::path::PathBuf;
#[derive(Default)]
pub(super) struct Controller {
    config: Config,
    service: Option<Service>,
    entries: VecDeque<Entry>,
    generation: u64,
    expiry: slint::Timer,
    status: String,
}
fn config_path() -> PathBuf {
    Settings::path().with_file_name("folders.json")
}
pub(super) fn refresh(app: &Rc<RefCell<App>>) {
    let mut a = app.borrow_mut();
    a.downloads
        .entries
        .retain(|e| e.at.elapsed() < Duration::from_secs(1800));
    if let Some(ui) = a.preferences() {
        ui.set_folders_enabled(a.downloads.config.enabled);
        ui.set_watch_downloads(a.downloads.config.downloads);
        ui.set_watched_folders(
            Rc::new(slint::VecModel::from(
                a.downloads
                    .config
                    .folders
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned().into())
                    .collect::<Vec<slint::SharedString>>(),
            ))
            .into(),
        );
        ui.set_folder_status(a.downloads.status.as_str().into());
    }
}
fn restart(app: &Rc<RefCell<App>>, persist: bool) {
    let (config, generation, weak) = {
        let mut a = app.borrow_mut();
        a.downloads.service.take();
        a.downloads.generation += 1;
        a.downloads.status.clear();
        if !a.downloads.config.enabled {
            a.downloads.status =
                "Folder monitoring is off. Enable it in Settings → Activities.".into();
            a.coordinator.clear_files();
            a.downloads.expiry.stop();
            a.downloads.entries.clear();
        }
        (
            a.downloads.config.clone(),
            a.downloads.generation,
            a.ui.clone(),
        )
    };
    if persist && !app.borrow().verification {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            std::fs::create_dir_all(config_path().parent().unwrap())?;
            std::fs::write(config_path(), serde_json::to_vec(&config)?)?;
            Ok(())
        })();
        if result.is_err() {
            app.borrow_mut().downloads.status = "Could not save folder preferences.".into();
        }
    }
    if config.enabled && (!app.borrow().verification || verification::has_arg("--verify-downloads"))
    {
        // Slint weak handles cross threads; Rc<App> stays exclusively on the UI thread.
        app.borrow_mut().downloads.service =
            Some(Service::start(config, move |entries, status| {
                let _ = weak.upgrade_in_event_loop(move |_| {
                    INBOX.with(|slot| {
                        if let Some(app) = slot.borrow().as_ref().and_then(|a| a.upgrade()) {
                            let mut a = app.borrow_mut();
                            if a.downloads.generation != generation {
                                return;
                            }
                            a.downloads.status = status;
                            let new_entries: Vec<_> = entries
                                .iter()
                                .filter(|e| {
                                    !a.downloads.entries.iter().any(|old| old.path == e.path)
                                })
                                .collect();
                            let count = new_entries.len() as u32;
                            let title = new_entries.last().map(|e| {
                                e.path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned()
                            });
                            for entry in entries {
                                a.downloads.entries.retain(|e| e.path != entry.path);
                                a.downloads.entries.push_front(entry);
                            }
                            a.downloads.entries.truncate(100);
                            let now = a.now();
                            if let Some(title) = title {
                                a.coordinator.files_changed(title, count, now);
                            }
                            drop(a);
                            refresh(&app);
                            sync_presentation(&app);
                            schedule_expiry(&app);
                        }
                    });
                });
            }));
    }
    refresh(app);
    sync_presentation(app);
}
thread_local! { static INBOX: RefCell<Option<std::rc::Weak<RefCell<App>>>> = const { RefCell::new(None) }; }
fn schedule_expiry(app: &Rc<RefCell<App>>) {
    let delay = app
        .borrow()
        .downloads
        .entries
        .back()
        .map(|e| Duration::from_secs(1800).saturating_sub(e.at.elapsed()));
    if let Some(delay) = delay {
        let weak = Rc::downgrade(app);
        app.borrow().downloads.expiry.start(
            slint::TimerMode::SingleShot,
            delay.max(Duration::from_millis(1)),
            move || {
                if let Some(app) = weak.upgrade() {
                    refresh(&app);
                    schedule_expiry(&app);
                }
            },
        );
    }
}
pub(super) fn wire_preferences(app: &Rc<RefCell<App>>, prefs: &SettingsWindow) {
    {
        let app = app.clone();
        prefs.on_folders_toggle(move |v| {
            app.borrow_mut().downloads.config.enabled = v;
            restart(&app, true);
        });
    }
    {
        let app = app.clone();
        prefs.on_downloads_toggle(move |v| {
            app.borrow_mut().downloads.config.downloads = v;
            restart(&app, true);
        });
    }
    {
        let app = app.clone();
        prefs.on_folder_remove(move |i| {
            if i >= 0 && (i as usize) < app.borrow().downloads.config.folders.len() {
                app.borrow_mut().downloads.config.folders.remove(i as usize);
                restart(&app, true);
            }
        });
    }
    {
        let app = app.clone();
        prefs.on_folder_add(move || {
            use slint::winit_030::WinitWindowAccessor;
            let Some(prefs) = app.borrow().preferences() else {
                return;
            };
            let mut owner = 0;
            prefs.window().with_winit_window(|w| {
                if let Ok(h) = w.window_handle() {
                    if let RawWindowHandle::Win32(h) = h.as_raw() {
                        owner = h.hwnd.get();
                    }
                }
            });
            let weak = app.borrow().ui.clone();
            std::thread::spawn(move || {
                let result = platform::file_dialog::choose_folder(owner);
                let _ = weak.upgrade_in_event_loop(move |_| {
                    INBOX.with(|slot| {
                        if let Some(app) = slot.borrow().as_ref().and_then(|w| w.upgrade()) {
                            match result {
                                Ok(Some(path)) => {
                                    if !app.borrow().downloads.config.folders.contains(&path) {
                                        app.borrow_mut().downloads.config.folders.push(path);
                                    }
                                    restart(&app, true);
                                }
                                Err(_) => {
                                    app.borrow_mut().downloads.status =
                                        "Could not open the folder picker. Try again.".into();
                                    refresh(&app);
                                }
                                _ => {}
                            }
                        }
                    });
                });
            });
        });
    }
}

pub(super) fn wire(app: &Rc<RefCell<App>>, _ui: &IslandWindow) {
    if !app.borrow().verification {
        app.borrow_mut().downloads.config = std::fs::read(config_path())
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or(Config {
                downloads: true,
                ..Default::default()
            });
    }
    INBOX.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(app)));
    restart(app, false);
    if verification::has_arg("--verify-downloads") {
        let folder = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("downloads-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("baseline.txt"), b"existing").unwrap();
        app.borrow_mut().downloads.config = Config {
            enabled: true,
            downloads: false,
            folders: vec![folder.clone()],
        };
        restart(app, false);
        let app = app.clone();
        slint::Timer::single_shot(Duration::from_secs(2), move || {
            sync_presentation(&app);
            let folder2 = folder.clone();
            let app2 = app.clone();
            slint::Timer::single_shot(Duration::from_secs(1), move || {
                if let Some(ui) = app2.borrow().ui.upgrade() {
                    let _ = crate::verification::snapshot(&ui, "downloads-empty");
                }
                std::fs::write(folder2.join("A long downloaded filename with spaces and readable content.pdf.crdownload"), b"partial").unwrap();
                std::fs::rename(folder2.join("A long downloaded filename with spaces and readable content.pdf.crdownload"), folder2.join("A long downloaded filename with spaces and readable content.pdf")).unwrap();
            });
            slint::Timer::single_shot(Duration::from_secs(5), move || {
                let a = app.borrow();
                record(&format!(
                    "downloads {} real watcher: final file only, silent baseline",
                    if a.downloads.entries.len() == 1 && a.downloads.entries[0].added {
                        "PASS"
                    } else {
                        "FAILED"
                    }
                ));
                if let Some(ui) = a.ui.upgrade() {
                    let _ = crate::verification::snapshot(&ui, "downloads-active");
                }
                drop(a);
                app.borrow_mut().downloads.service.take();
                let _ = std::fs::remove_file(folder.join("baseline.txt"));
                let _ = std::fs::remove_file(
                    folder.join("A long downloaded filename with spaces and readable content.pdf"),
                );
                let _ = std::fs::remove_dir(folder);
                let _ = slint::quit_event_loop();
            });
        });
    }
}
