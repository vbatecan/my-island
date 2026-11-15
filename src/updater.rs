//! Finite update jobs. PowerShell hosts Windows download and restart helpers.
use crate::{IslandWindow, SettingsWindow};
use slint::ComponentHandle;
use std::os::windows::process::CommandExt;
use std::{cell::RefCell, path::PathBuf, process::Command, rc::Rc, time::Duration};

fn directory() -> PathBuf {
    crate::settings::Settings::path()
        .parent()
        .unwrap()
        .join("updates")
}
fn command(script: &std::path::Path) -> Command {
    let mut c = Command::new("powershell.exe");
    c.creation_flags(0x08000000)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script);
    c
}
fn version(value: &str) -> Option<[u32; 3]> {
    let parts = value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    parts.try_into().ok()
}
fn cached(root: &std::path::Path) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let bytes = std::fs::read(path.join("ready.json")).ok()?;
            // Windows PowerShell writes UTF-8 metadata with a BOM.
            let bytes = bytes.strip_prefix(&[239, 187, 191]).unwrap_or(&bytes);
            let metadata: serde_json::Value = serde_json::from_slice(bytes).ok()?;
            let target = PathBuf::from(metadata["executable"].as_str()?);
            let candidate = version(metadata["version"].as_str()?)?;
            (target == executable && candidate > version(env!("CARGO_PKG_VERSION"))?)
                .then_some((candidate, path))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, path)| path)
}
pub struct Updater {
    island: slint::Weak<IslandWindow>,
    settings: RefCell<slint::Weak<SettingsWindow>>,
    root: PathBuf,
    mode: UpdateMode,
    state: RefCell<UpdateState>,
    timer: slint::Timer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdateMode {
    Portable,
    Packaged,
    Store,
    Unavailable,
}

struct UpdateState {
    enabled: bool,
    generation: u64,
    busy: bool,
    ready: Option<PathBuf>,
    status: String,
}

fn version_status() -> String {
    format!("Version {}", env!("CARGO_PKG_VERSION"))
}

pub fn start(island: &IslandWindow, verification: bool) -> Rc<Updater> {
    let root = directory();
    let mode = if let Ok(package) = windows::ApplicationModel::Package::Current() {
        if package
            .SignatureKind()
            .is_ok_and(|kind| kind == windows::ApplicationModel::PackageSignatureKind::Store)
        {
            UpdateMode::Store
        } else {
            UpdateMode::Packaged
        }
    } else {
        UpdateMode::Portable
    };
    let mode = if mode != UpdateMode::Store
        && (std::fs::create_dir_all(&root).is_err()
            || std::fs::write(
                root.join("check.ps1"),
                include_str!("../scripts/app-update.ps1"),
            )
            .is_err()
            || std::fs::write(
                root.join("restart.ps1"),
                include_str!("../scripts/app-update-restart.ps1"),
            )
            .is_err())
    {
        UpdateMode::Unavailable
    } else {
        mode
    };
    let enabled = matches!(mode, UpdateMode::Portable | UpdateMode::Packaged)
        && !root.join("disabled").exists();
    let ready = if mode == UpdateMode::Portable && enabled && !verification {
        cached(&root)
    } else {
        None
    };
    let status = match mode {
        UpdateMode::Store => format!(
            "{} · Updates are managed by Microsoft Store.",
            version_status()
        ),
        UpdateMode::Unavailable => format!("{} · Update setup failed.", version_status()),
        _ if ready.is_some() => "Update ready.".into(),
        _ => version_status(),
    };
    let updater = Rc::new(Updater {
        island: island.as_weak(),
        settings: RefCell::new(slint::Weak::default()),
        root,
        mode,
        state: RefCell::new(UpdateState {
            enabled,
            generation: 0,
            busy: false,
            ready,
            status,
        }),
        timer: slint::Timer::default(),
    });
    if !cfg!(debug_assertions) && !verification && mode == UpdateMode::Portable {
        let weak = Rc::downgrade(&updater);
        updater.timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(30),
            move || {
                let Some(updater) = weak.upgrade() else {
                    return;
                };
                updater.check_automatically();
                let weak = weak.clone();
                updater.timer.start(
                    slint::TimerMode::Repeated,
                    Duration::from_secs(86400),
                    move || {
                        if let Some(updater) = weak.upgrade() {
                            updater.check_automatically();
                        }
                    },
                );
            },
        );
    }
    updater
}

impl Updater {
    pub fn bind(self: &Rc<Self>, ui: &SettingsWindow) {
        *self.settings.borrow_mut() = ui.as_weak();
        ui.set_update_packaged(matches!(
            self.mode,
            UpdateMode::Packaged | UpdateMode::Store
        ));
        ui.set_update_store(self.mode == UpdateMode::Store);
        self.refresh();
        let weak = Rc::downgrade(self);
        ui.on_check_update(move || {
            if let Some(updater) = weak.upgrade() {
                updater.check();
            }
        });
        if matches!(self.mode, UpdateMode::Portable | UpdateMode::Packaged) {
            let weak = Rc::downgrade(self);
            ui.on_auto_update_changed(move |enabled| {
                if let Some(updater) = weak.upgrade() {
                    updater.set_enabled(enabled);
                }
            });
            let weak = Rc::downgrade(self);
            ui.on_install_update(move || {
                if let Some(updater) = weak.upgrade() {
                    updater.install();
                }
            });
        }
    }

    fn refresh(&self) {
        if let Some(ui) = self.settings.borrow().upgrade() {
            let state = self.state.borrow();
            ui.set_auto_update(state.enabled);
            ui.set_update_ready(state.ready.is_some());
            ui.set_update_busy(state.busy);
            ui.set_update_status(state.status.as_str().into());
        }
    }

    fn set_enabled(&self, enabled: bool) {
        if enabled {
            let _ = std::fs::remove_file(self.root.join("disabled"));
        } else {
            let _ = std::fs::write(self.root.join("disabled"), b"");
        }
        {
            let mut state = self.state.borrow_mut();
            state.enabled = enabled;
            state.generation = state.generation.wrapping_add(1);
            state.ready = None;
            state.busy = false;
            state.status = version_status();
        }
        self.refresh();
    }

    fn check_automatically(self: &Rc<Self>) {
        let should_check = {
            let state = self.state.borrow();
            state.enabled && state.ready.is_none()
        };
        if should_check {
            self.check();
        }
    }

    fn check(self: &Rc<Self>) {
        match self.mode {
            UpdateMode::Store => {
                if Command::new("explorer.exe")
                    .arg("ms-windows-store://pdp/?ProductId=9ND16NLPK4SQ")
                    .spawn()
                    .is_err()
                {
                    self.state.borrow_mut().status =
                        "Open Microsoft Store to check for updates.".into();
                    self.refresh();
                }
                return;
            }
            UpdateMode::Packaged => {
                let _ = Command::new("explorer.exe")
                    .arg("https://github.com/vbatecan/my-island/releases/latest")
                    .spawn();
                return;
            }
            UpdateMode::Unavailable => return,
            UpdateMode::Portable => {}
        }
        let Some(island) = self.island.upgrade() else {
            return;
        };
        let token = {
            let mut state = self.state.borrow_mut();
            if state.busy {
                return;
            }
            state.generation = state.generation.wrapping_add(1);
            state.ready = None;
            state.busy = true;
            state.status = "Checking for updates…".into();
            state.generation
        };
        self.refresh();
        let script = self.root.join("check.ps1");
        let window = island.as_weak();
        let (tx, rx) = std::sync::mpsc::channel();
        let weak = Rc::downgrade(self);
        island.on_update_finished(move || {
            if let (Some(updater), Ok(value)) = (weak.upgrade(), rx.try_recv()) {
                updater.finish(token, value);
            }
        });
        if std::thread::Builder::new()
            .name("island-update".into())
            .spawn(move || {
                let value = command(&script)
                    .args(["-CurrentVersion", env!("CARGO_PKG_VERSION"), "-Executable"])
                    .arg(std::env::current_exe().unwrap_or_default())
                    .output()
                    .ok()
                    .filter(|result| result.status.success())
                    .map(|result| String::from_utf8_lossy(&result.stdout).trim().to_owned());
                let _ = tx.send(value);
                let _ = window.upgrade_in_event_loop(|ui| ui.invoke_update_finished());
            })
            .is_err()
        {
            self.finish(token, None);
        }
    }

    fn finish(&self, token: u64, result: Option<String>) {
        {
            let mut state = self.state.borrow_mut();
            if state.generation != token {
                return;
            }
            state.busy = false;
            state.status = match result {
                Some(value) if value == "Current" => "You're up to date.".into(),
                Some(value) => {
                    state.ready = Some(PathBuf::from(value));
                    "Update ready.".into()
                }
                None => "Could not download the update. Try again.".into(),
            };
        }
        self.refresh();
    }

    fn install(&self) {
        if let Some(work) = self.state.borrow().ready.as_ref()
            && command(&self.root.join("restart.ps1"))
                .arg("-Work")
                .arg(work)
                .arg("-Executable")
                .arg(std::env::current_exe().unwrap_or_default())
                .arg("-ParentPid")
                .arg(std::process::id().to_string())
                .spawn()
                .is_ok()
        {
            let _ = slint::quit_event_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::version;
    #[test]
    fn versions_compare_numerically_and_reject_incomplete_or_prerelease() {
        assert!(version("1.10.0") > version("1.9.9"));
        assert_eq!(version("1.2"), None);
        assert_eq!(version("1.2.3.4"), None);
        assert_eq!(version("1.2.3-rc1"), None);
    }
}
