//! Event-driven directory observations. No inference of download completion.
use notify::{EventKind, RecursiveMode, Watcher};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub downloads: bool,
    pub folders: Vec<PathBuf>,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub added: bool,
    pub at: Instant,
}
enum Message {
    Event(notify::Result<notify::Event>),
    Stop,
}
pub struct Service {
    stop: mpsc::SyncSender<Message>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.stop.send(Message::Stop);
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}
pub fn temporary(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| {
        ["crdownload", "part", "partial", "tmp", "download"]
            .iter()
            .any(|v| e.eq_ignore_ascii_case(v))
    })
}
pub fn known_downloads() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::{
        Win32::System::Com::CoTaskMemFree,
        Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
        let result = PathBuf::from(std::ffi::OsString::from_wide(p.as_wide()));
        CoTaskMemFree(Some(p.0.cast()));
        Some(result)
    }
}
impl Service {
    pub fn start(config: Config, callback: impl Fn(Vec<Entry>, String) + Send + 'static) -> Self {
        let (stop, rx) = mpsc::sync_channel(256);
        let tx = stop.clone();
        let worker = std::thread::spawn(move || {
            let overflow = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let dropped = overflow.clone();
            let mut watcher = match notify::recommended_watcher(move |e| {
                if tx.try_send(Message::Event(e)).is_err() {
                    dropped.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }) {
                Ok(w) => w,
                Err(_) => {
                    callback(
                        vec![],
                        "Folder monitoring unavailable. Disable and enable to retry.".into(),
                    );
                    return;
                }
            };
            let mut folders = config.folders;
            if config.downloads {
                if let Some(p) = known_downloads() {
                    folders.push(p);
                }
            }
            let mut seen = std::collections::HashSet::new();
            let mut unavailable = false;
            for folder in folders {
                if let Ok(path) = folder.canonicalize() {
                    if seen.insert(path.to_string_lossy().to_lowercase())
                        && watcher.watch(&path, RecursiveMode::NonRecursive).is_err()
                    {
                        unavailable = true;
                    }
                } else {
                    unavailable = true;
                }
            }
            let status = if unavailable {
                "Some folders are unavailable. Check paths, then disable and enable to retry."
            } else {
                ""
            }
            .to_owned();
            callback(vec![], status.clone());
            let mut pending = BTreeMap::<PathBuf, (bool, Instant)>::new();
            loop {
                if overflow.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    pending.clear();
                    callback(vec![], "Some rapid folder changes were skipped.".into());
                }
                let message = if pending.is_empty() {
                    rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                } else {
                    rx.recv_timeout(Duration::from_millis(100))
                };
                match message {
                    Ok(Message::Stop) => break,
                    Ok(Message::Event(Ok(event))) => {
                        if event.need_rescan() {
                            pending.clear();
                            callback(vec![], "Folder activity changed too quickly; earlier changes were skipped.".into());
                            continue;
                        }
                        let added = matches!(
                            event.kind,
                            EventKind::Create(_)
                                | EventKind::Modify(notify::event::ModifyKind::Name(_))
                        );
                        if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                            for path in event.paths {
                                if !temporary(&path) && path.is_file() {
                                    if pending.len() < 256 || pending.contains_key(&path) {
                                        pending
                                            .entry(path)
                                            .and_modify(|v| v.0 |= added)
                                            .or_insert((added, Instant::now()));
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Event(Err(_))) => {
                        callback(
                            vec![],
                            "Folder monitoring interrupted. Disable and enable to retry.".into(),
                        );
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    _ => {}
                }
                let now = Instant::now();
                let ready: Vec<_> = pending
                    .iter()
                    .filter(|(_, (_, at))| now.duration_since(*at) >= Duration::from_secs(1))
                    .map(|(p, (added, _))| Entry {
                        path: p.clone(),
                        added: *added,
                        at: now,
                    })
                    .collect();
                for e in &ready {
                    pending.remove(&e.path);
                }
                if !ready.is_empty() {
                    callback(ready, status.clone());
                }
            }
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_files_are_filtered_case_insensitively() {
        assert!(temporary(std::path::Path::new("a.CRDOWNLOAD")));
        assert!(temporary(std::path::Path::new("a.part")));
        assert!(!temporary(std::path::Path::new("a.pdf")));
    }
    #[test]
    fn native_watcher_baselines_filters_and_stops() {
        let root = std::env::temp_dir().join(format!(
            "still-island-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("baseline.txt"), b"existing").unwrap();
        let (tx, rx) = mpsc::channel();
        let service = Service::start(
            Config {
                enabled: true,
                downloads: false,
                folders: vec![root.clone(), root.clone()],
            },
            move |e, s| {
                let _ = tx.send((e, s));
            },
        );
        let (entries, status) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(entries.is_empty() && status.is_empty());
        std::fs::write(root.join("new.part"), b"data").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(1200)).is_err());
        std::fs::rename(root.join("new.part"), root.join("final.pdf")).unwrap();
        let (entries, _) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].path,
            root.canonicalize().unwrap().join("final.pdf")
        );
        assert!(entries[0].added);
        let now = Instant::now();
        drop(service);
        assert!(now.elapsed() < Duration::from_secs(2));
        std::fs::remove_file(root.join("baseline.txt")).unwrap();
        std::fs::remove_file(root.join("final.pdf")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
