//! Trusted executable integrations. No plugin code is loaded into the GUI.
mod native;
mod package;
mod process;
#[cfg(test)]
mod runtime_tests;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::SyncSender},
};
pub use still_island_plugin_protocol;
use still_island_plugin_protocol::{ConfigKind, HostMessage, Manifest};

#[derive(Clone, Debug)]
pub struct InstalledPlugin {
    pub manifest: Manifest,
    pub enabled: bool,
    pub status: String,
    pub error: Option<String>,
    pub directory: PathBuf,
}
#[derive(Clone, Debug)]
pub struct PluginEvent {
    pub plugin_id: String,
    pub message: Value,
}
#[derive(Default, Serialize, Deserialize)]
struct Preferences {
    enabled: bool,
    #[serde(default)]
    config: Value,
    #[serde(default)]
    pinned: Vec<String>,
}
struct Entry {
    manifest: Manifest,
    directory: PathBuf,
    prefs: Preferences,
    status: String,
    error: Option<String>,
    generation: u64,
    running: Option<Running>,
}
struct Running {
    sender: SyncSender<process::Command>,
    job: Arc<process::ProcessControl>,
}
struct Inner {
    root: PathBuf,
    entries: Mutex<BTreeMap<String, Entry>>,
    callback: Box<dyn Fn(PluginEvent) + Send + Sync>,
    logs: Mutex<BTreeMap<String, VecDeque<String>>>,
    operation: Mutex<()>,
}
pub struct PluginHost {
    inner: Arc<Inner>,
}
impl PluginHost {
    pub fn new(
        callback: impl Fn(PluginEvent) + Send + Sync + 'static,
    ) -> Result<Arc<Self>, String> {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or("LOCALAPPDATA is unavailable")?
            .join("StillIsland")
            .join("plugins");
        Self::with_root(root, callback)
    }
    pub fn with_root(
        root: impl AsRef<Path>,
        callback: impl Fn(PluginEvent) + Send + Sync + 'static,
    ) -> Result<Arc<Self>, String> {
        fs::create_dir_all(root.as_ref()).map_err(|e| e.to_string())?;
        let root = root.as_ref().canonicalize().map_err(|e| e.to_string())?;
        fs::create_dir_all(root.join("config")).map_err(|e| e.to_string())?;
        package::directory_inside(&root, &root.join("config"))?;
        let host = Arc::new(Self {
            inner: Arc::new(Inner {
                root,
                entries: Mutex::new(BTreeMap::new()),
                callback: Box::new(callback),
                logs: Mutex::new(BTreeMap::new()),
                operation: Mutex::new(()),
            }),
        });
        let mut entries = host.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
        for item in fs::read_dir(&host.inner.root).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            if !item.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let id = item.file_name().to_string_lossy().into_owned();
            if id == "config" || id.starts_with('.') {
                continue;
            }
            let selected =
                fs::read_to_string(host.inner.root.join("config").join(format!("{id}.version")))
                    .unwrap_or_default();
            if package::relative(&selected).is_err() || selected.contains('/') {
                continue;
            }
            let directory = item.path().join(selected.trim());
            if package::directory_inside(&host.inner.root, &directory).is_err() {
                continue;
            }
            if !directory
                .canonicalize()
                .is_ok_and(|p| p.starts_with(&host.inner.root))
            {
                continue;
            }
            if let Ok(manifest) = package::read_manifest(&directory.join("manifest.json")) {
                if manifest.id != id {
                    continue;
                }
                let prefs = fs::read(host.inner.root.join("config").join(format!("{id}.json")))
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                entries.insert(
                    id,
                    Entry {
                        manifest,
                        directory,
                        prefs,
                        status: "Stopped".into(),
                        error: None,
                        generation: 0,
                        running: None,
                    },
                );
            }
        }
        drop(entries);
        let background = host
            .list()
            .into_iter()
            .filter(|p| p.enabled && p.manifest.background)
            .map(|p| p.manifest.id)
            .collect::<Vec<_>>();
        for id in background {
            let _ = host.start(&id);
        }
        Ok(host)
    }
    pub fn list(&self) -> Vec<InstalledPlugin> {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|e| InstalledPlugin {
                manifest: e.manifest.clone(),
                enabled: e.prefs.enabled,
                status: e.status.clone(),
                error: e.error.clone(),
                directory: e.directory.clone(),
            })
            .collect()
    }
    pub fn import(&self, path: &Path) -> Result<Manifest, String> {
        let _operation = self
            .inner
            .operation
            .try_lock()
            .map_err(|_| "Another plugin operation is in progress")?;
        self.inner.config_dir()?;
        let (manifest, directory) = package::import(path, &self.inner.root)?;
        let id = manifest.id.clone();
        self.stop(&id);
        let mut entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
        let prefs = entries.remove(&id).map(|e| e.prefs).unwrap_or_default();
        // Updates keep configuration but require explicit re-enabling.
        let prefs = Preferences {
            enabled: false,
            ..prefs
        };
        entries.insert(
            id.clone(),
            Entry {
                manifest: manifest.clone(),
                directory,
                prefs,
                status: "Disabled".into(),
                error: None,
                generation: 0,
                running: None,
            },
        );
        drop(entries);
        fs::write(
            self.inner.root.join("config").join(format!("{id}.version")),
            &manifest.version,
        )
        .map_err(|e| e.to_string())?;
        self.save(&id)?;
        self.inner.status(&id, "Disabled", None);
        Ok(manifest)
    }
    fn save(&self, id: &str) -> Result<(), String> {
        self.inner.config_dir()?;
        let bytes = {
            let entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
            serde_json::to_vec(&entries.get(id).ok_or("Unknown plugin")?.prefs)
                .map_err(|e| e.to_string())?
        };
        let path = self.inner.root.join("config").join(format!("{id}.json"));
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        fs::rename(temp, path).map_err(|e| e.to_string())
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .try_lock()
            .map_err(|_| "Another plugin operation is in progress")?;
        let background = {
            let mut entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
            let e = entries.get_mut(id).ok_or("Unknown plugin")?;
            e.prefs.enabled = enabled;
            e.error = None;
            e.manifest.background
        };
        self.save(id)?;
        if !enabled {
            self.stop(id);
            self.inner.status(id, "Disabled", None);
        } else if background {
            self.start(id)?;
        } else {
            self.inner.status(id, "Enabled", None);
        }
        Ok(())
    }
    pub fn configuration(&self, id: &str) -> Result<Value, String> {
        let entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
        let e = entries.get(id).ok_or("Unknown plugin")?;
        let mut config = e.prefs.config.as_object().cloned().unwrap_or_default();
        for f in &e.manifest.config {
            config
                .entry(f.id.clone())
                .or_insert_with(|| f.default.clone());
            if f.kind == ConfigKind::Secret {
                config.insert(f.id.clone(), Value::String(String::new()));
            }
        }
        Ok(Value::Object(config))
    }
    pub fn configure(&self, id: &str, values: Value) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .try_lock()
            .map_err(|_| "Another plugin operation is in progress")?;
        let values = values
            .as_object()
            .ok_or("Configuration must be an object")?;
        let manifest = {
            let entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
            entries.get(id).ok_or("Unknown plugin")?.manifest.clone()
        };
        let mut config = self
            .configuration(id)?
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut secrets = self.inner.secrets(id)?;
        for f in &manifest.config {
            if let Some(v) = values.get(&f.id) {
                if f.kind == ConfigKind::Secret {
                    if v.is_null() {
                        secrets.remove(&f.id);
                    } else if let Some(s) = v.as_str() {
                        if !s.is_empty() {
                            if s.len() > 8192 {
                                return Err("Secret exceeds size limit".into());
                            }
                            secrets.insert(f.id.clone(), s.to_string());
                        }
                    } else {
                        return Err("Secret must be text".into());
                    }
                    config.remove(&f.id);
                    continue;
                }
                let valid = match f.kind {
                    ConfigKind::String => v.as_str().is_some_and(|s| s.len() <= 4096),
                    ConfigKind::Bool => v.is_boolean(),
                    ConfigKind::Integer => v.is_i64() || v.is_u64(),
                    ConfigKind::Number => v.as_f64().is_some_and(f64::is_finite),
                    ConfigKind::Choice => {
                        v.as_str().is_some_and(|s| f.choices.iter().any(|c| c == s))
                    }
                    ConfigKind::Secret => true,
                };
                if !valid {
                    return Err(format!("Invalid value for {}", f.label));
                }
                config.insert(f.id.clone(), v.clone());
            }
        }
        let encrypted = native::protect(
            &serde_json::to_vec(&secrets).map_err(|e| e.to_string())?,
            false,
        )?;
        fs::write(
            self.inner.root.join("config").join(format!("{id}.secrets")),
            encrypted,
        )
        .map_err(|e| e.to_string())?;
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(id)
            .ok_or("Unknown plugin")?
            .prefs
            .config = Value::Object(config);
        self.save(id)?;
        let config = self.inner.full_configuration(id)?;
        if self.is_running(id) {
            self.send(id, HostMessage::Configure { config })?;
        }
        Ok(())
    }
    pub fn recent_logs(&self, id: &str) -> Vec<String> {
        self.inner
            .logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default()
    }
    pub fn pinned(&self) -> Vec<(String, String)> {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, e)| e.prefs.enabled)
            .flat_map(|(id, e)| {
                e.prefs
                    .pinned
                    .iter()
                    .filter(|a| e.manifest.actions.iter().any(|v| &v.id == *a))
                    .map(|a| (id.clone(), a.clone()))
            })
            .collect()
    }
    pub fn set_pinned(&self, id: &str, action: &str, pinned: bool) -> Result<(), String> {
        let mut entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
        let e = entries.get_mut(id).ok_or("Unknown plugin")?;
        if !e.manifest.actions.iter().any(|a| a.id == action) {
            return Err("Unknown action".into());
        }
        e.prefs.pinned.retain(|a| a != action);
        if pinned {
            e.prefs.pinned.push(action.into());
        }
        drop(entries);
        self.save(id)
    }
    pub fn restart(&self, id: &str) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .try_lock()
            .map_err(|_| "Another plugin operation is in progress")?;
        self.stop(id);
        self.inner.status(id, "Stopped", None);
        self.start(id)
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .try_lock()
            .map_err(|_| "Another plugin operation is in progress")?;
        if !self
            .inner
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
        {
            return Err("Unknown plugin".into());
        }
        self.inner
            .status(id, "Removing", Some("Plugin removal is in progress".into()));
        self.stop(id);
        let path = self.inner.root.join(id);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            // A graceful process may retain its executable mapping briefly.
            // Every retry repeats canonical/reparse validation before deletion.
            match package::safe_remove(&path, &self.inner.root) {
                Ok(()) => break,
                Err(error) if std::time::Instant::now() < deadline && path.exists() => {
                    if error.contains("reparse") || error.contains("escapes") {
                        self.inner.status(id, "Unavailable", Some(error.clone()));
                        return Err(error);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => {
                    self.inner.status(id, "Unavailable", Some(error.clone()));
                    return Err(error);
                }
            }
        }
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        for ext in ["json", "version", "secrets"] {
            let p = self.inner.root.join("config").join(format!("{id}.{ext}"));
            if p.is_file() {
                fs::remove_file(p).map_err(|e| e.to_string())?;
            }
        }
        self.inner
            .emit(id, json!({"type":"host_status","status":"Removed"}));
        Ok(())
    }
    pub fn request_view(&self, id: &str, view: &str) -> Result<(), String> {
        let valid = self
            .list()
            .iter()
            .any(|p| p.manifest.id == id && p.manifest.views.iter().any(|v| v.id == view));
        if !valid {
            return Err("Unknown plugin view".into());
        }
        self.start(id)?;
        self.send(
            id,
            HostMessage::ViewRequest {
                view_id: view.into(),
            },
        )
    }
    pub fn invoke(&self, id: &str, action: &str, value: Value) -> Result<(), String> {
        let valid = self
            .list()
            .iter()
            .any(|p| p.manifest.id == id && p.manifest.actions.iter().any(|v| v.id == action));
        if !valid {
            return Err("Unknown plugin action".into());
        }
        self.start(id)?;
        self.send(
            id,
            HostMessage::Invoke {
                action_id: action.into(),
                value,
            },
        )
    }
    pub fn set_visible(&self, id: &str, visible: bool) -> Result<(), String> {
        let entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = entries.get(id).and_then(|e| e.running.as_ref()) {
            r.sender
                .try_send(process::Command::Visible(visible))
                .map_err(|_| "Plugin input queue is full".to_string())?;
        }
        Ok(())
    }
    fn send(&self, id: &str, message: HostMessage) -> Result<(), String> {
        let entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .get(id)
            .and_then(|e| e.running.as_ref())
            .ok_or("Plugin is not running")?
            .sender
            .try_send(process::Command::Send(message))
            .map_err(|_| "Plugin input queue is full".into())
    }
    fn is_running(&self, id: &str) -> bool {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .is_some_and(|e| e.running.is_some())
    }
    fn start(&self, id: &str) -> Result<(), String> {
        process::start(&self.inner, id)
    }
    fn stop(&self, id: &str) {
        let running = {
            let mut entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
            entries.get_mut(id).and_then(|e| {
                e.generation = e.generation.wrapping_add(1);
                e.running.take()
            })
        };
        if let Some(r) = running {
            r.job.cancel();
            let _ = r.sender.try_send(process::Command::Stop);
        }
    }
}
impl Drop for PluginHost {
    fn drop(&mut self) {
        for p in self.list() {
            self.stop(&p.manifest.id);
        }
    }
}
impl Inner {
    fn log(&self, id: &str, line: &str) {
        let redacted = self.redact(id, line);
        let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
        let log = logs.entry(id.into()).or_default();
        log.push_back(redacted);
        while log.len() > 16 || log.iter().map(|s| s.len()).sum::<usize>() > 65536 {
            log.pop_front();
        }
    }

    fn config_dir(&self) -> Result<(), String> {
        package::directory_inside(&self.root, &self.root.join("config"))
    }

    fn secrets(&self, id: &str) -> Result<HashMap<String, String>, String> {
        self.config_dir()?;
        let path = self.root.join("config").join(format!("{id}.secrets"));
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        serde_json::from_slice(&native::protect(&bytes, true)?)
            .map_err(|_| "Invalid protected configuration".into())
    }
    fn full_configuration(&self, id: &str) -> Result<Value, String> {
        let mut config = {
            let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let e = entries.get(id).ok_or("Unknown plugin")?;
            let mut c = e.prefs.config.as_object().cloned().unwrap_or_default();
            for f in &e.manifest.config {
                if f.kind != ConfigKind::Secret {
                    c.entry(f.id.clone()).or_insert_with(|| f.default.clone());
                }
            }
            c
        };
        for (k, v) in self.secrets(id)? {
            config.insert(k, Value::String(v));
        }
        Ok(Value::Object(config))
    }
    fn emit(&self, id: &str, message: Value) {
        (self.callback)(PluginEvent {
            plugin_id: id.into(),
            message,
        });
    }
    fn status(&self, id: &str, status: &str, error: Option<String>) {
        let error = error.map(|e| self.redact(id, &e));
        {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(e) = entries.get_mut(id) {
                e.status = status.into();
                e.error = error.clone();
            }
        }
        self.emit(
            id,
            json!({"type":"host_status","status":status,"error":error}),
        );
    }
    fn redact(&self, id: &str, text: &str) -> String {
        let mut text = text.to_owned();
        if let Ok(secrets) = self.secrets(id) {
            for secret in secrets.values().filter(|s| !s.is_empty()) {
                text = text.replace(secret, "[redacted]");
            }
        }
        text.chars().take(4096).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "plugin-unit-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn archive(root: &Path, id: &str, version: &str) -> PathBuf {
        use std::io::Write;
        let path = root.join("test.zip");
        let file = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(serde_json::to_string(&json!({"id":id,"name":"Test","version":version,"api_version":1,"entrypoint":"plugin.exe","actions":[{"id":"go","label":"Go"}],"config":[{"id":"token","label":"Token","kind":"secret","default":""}]})).unwrap().as_bytes()).unwrap();
        zip.start_file("plugin.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"MZtest").unwrap();
        zip.finish().unwrap();
        path
    }
    #[test]
    fn imports_are_disabled_and_configuration_secrets_are_protected() {
        let root = root();
        let host = PluginHost::with_root(root.join("storage"), |_| {}).unwrap();
        host.import(&archive(&root, "org.test", "1.0.0")).unwrap();
        assert!(!host.list()[0].enabled);
        assert!(host.request_view("org.test", "missing").is_err());
        let secret = "secret-crossing-the-output-limit";
        host.configure("org.test", json!({"token":secret})).unwrap();
        assert_eq!(host.configuration("org.test").unwrap()["token"], "");
        let plain = fs::read_to_string(root.join("storage/config/org.test.json")).unwrap();
        assert!(!plain.contains(secret));
        assert_eq!(
            host.inner.full_configuration("org.test").unwrap()["token"],
            secret
        );
        host.configure("org.test", json!({"token":""})).unwrap();
        assert_eq!(
            host.inner.full_configuration("org.test").unwrap()["token"],
            secret
        );
        let output = format!("{}{}", "x".repeat(4080), secret);
        let redacted = host.inner.redact("org.test", &output);
        assert!(!redacted.contains("secret-crossing"));
        assert!(redacted.contains("[redacted]"));
        host.set_pinned("org.test", "go", true).unwrap();
        host.set_enabled("org.test", true).unwrap();
        assert_eq!(host.pinned(), vec![("org.test".into(), "go".into())]);
        host.set_enabled("org.test", false).unwrap();
        assert!(host.pinned().is_empty());
        host.remove("org.test").unwrap();
        assert!(host.list().is_empty());
    }
    #[test]
    fn refuses_reserved_namespace_and_version_escape() {
        let root = root();
        let host = PluginHost::with_root(root.join("storage"), |_| {}).unwrap();
        for (id, version) in [
            ("config", "1.0.0"),
            ("Config", "1.0.0"),
            ("org.test", "../../escape"),
            ("..", "1.0.0"),
            ("org.test", "C:/outside"),
        ] {
            assert!(
                host.import(&archive(&root, id, version)).is_err(),
                "{id} {version}"
            );
        }
        assert!(host.list().is_empty());
    }
}
