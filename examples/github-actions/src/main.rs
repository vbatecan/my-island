use serde_json::Value;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use still_island_plugin_sdk::*;
#[derive(Default)]
struct State {
    config: Value,
    run: Value,
    revision: u64,
    etag: String,
    error: String,
    delay: u64,
}
fn main() {
    if let Err(e) = plugin() {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
fn plugin() -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let input = std::io::stdin();
        let mut input = input.lock();
        loop {
            match read_message::<HostMessage>(&mut input) {
                Ok(Some(m)) => {
                    let stop = matches!(m, HostMessage::Shutdown);
                    if tx.send(m).is_err() || stop {
                        break;
                    }
                }
                _ => {
                    let _ = tx.send(HostMessage::Shutdown);
                    break;
                }
            }
        }
    });
    let mut state = State::default();
    let mut due = Instant::now() + Duration::from_secs(300);
    loop {
        let message = if state.config["monitor"].as_bool() == Some(true) {
            match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => break,
            }
        } else {
            rx.recv().ok()
        };
        match message {
            Some(HostMessage::Shutdown) => break,
            Some(HostMessage::Init {
                protocol_version,
                config,
            }) => {
                if protocol_version != PROTOCOL_VERSION {
                    return Err("Unsupported protocol".into());
                }
                state.config = config;
                send(&PluginMessage::Ready {
                    protocol_version: PROTOCOL_VERSION,
                })?;
                state.refresh(false);
            }
            Some(HostMessage::Configure { config }) => {
                state.config = config;
                state.etag.clear();
                state.run = Value::Null;
                state.refresh(false);
            }
            Some(HostMessage::ViewRequest { .. }) => state.refresh(false),
            Some(HostMessage::Invoke { action_id, .. }) => {
                if action_id == "open" {
                    let url = state.run["html_url"]
                        .as_str()
                        .filter(|u| u.starts_with("https://github.com/"));
                    if let Some(url) = url {
                        result(action_id, "Open workflow run", Some(url.to_owned()))?
                    } else {
                        failure(
                            action_id,
                            "Refresh a valid repository to select a workflow run",
                        )?
                    }
                } else if action_id == "refresh" {
                    state.refresh(false);
                    if state.error.is_empty() {
                        result(action_id, "Refresh finished", None)?
                    } else {
                        failure(action_id, &state.error)?
                    }
                } else {
                    failure(action_id, "Unknown action")?;
                }
            }
            None => state.refresh(true),
        }
        state.publish()?;
        due = Instant::now()
            + Duration::from_secs(state.delay.max(
                if state.run["status"] == "in_progress" || state.run["status"] == "queued" {
                    30
                } else {
                    300
                },
            ));
    }
    Ok(())
}
impl State {
    fn refresh(&mut self, notify: bool) {
        self.delay = 0;
        match self.fetch(notify) {
            Ok(()) => self.error.clear(),
            Err(e) => {
                self.error = e;
                self.delay = self.delay.max(300);
            }
        }
    }
    fn fetch(&mut self, notify: bool) -> Result<(), String> {
        let repository = self.config["repository"].as_str().unwrap_or("");
        if !valid_repository(repository) {
            return Err("Configure a repository as owner/name".into());
        }
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(5)))
                .http_status_as_error(false)
                .build(),
        );
        let mut request = agent
            .get(format!(
                "https://api.github.com/repos/{repository}/actions/runs?per_page=1"
            ))
            .header("User-Agent", "StillIsland-GitHub-Plugin/0.1")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = self.config["token"].as_str().filter(|s| !s.is_empty()) {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if !self.etag.is_empty() {
            request = request.header("If-None-Match", &self.etag)
        }
        let mut response = request
            .call()
            .map_err(|_| "GitHub connection failed; retry later".to_string())?;
        let status = response.status().as_u16();
        if status == 304 {
            return Ok(());
        }
        if status == 403 || status == 429 {
            self.delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(300)
                .clamp(30, 3600);
            return Err("GitHub rate limit or access restriction; backing off".into());
        }
        if status != 200 {
            return Err(format!(
                "GitHub returned HTTP {status}; check repository and read-only token"
            ));
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let body: Value = response
            .body_mut()
            .with_config()
            .limit(1_048_576)
            .read_json()
            .map_err(|_| "Invalid GitHub response".to_string())?;
        let run = body["workflow_runs"]
            .as_array()
            .and_then(|runs| runs.first())
            .cloned()
            .unwrap_or(Value::Null);
        if notify
            && !self.run.is_null()
            && (run["id"] != self.run["id"] || self.run["status"] != "completed")
            && run["status"] == "completed"
        {
            let conclusion = run["conclusion"].as_str().unwrap_or("completed");
            let _ = send(&PluginMessage::Activity {
                id: "workflow-completed".into(),
                title: format!("Build {conclusion}"),
                body: repository.into(),
                accent: if conclusion == "success" {
                    Accent::Green
                } else {
                    Accent::Red
                },
            });
        }
        self.run = run;
        self.etag = etag;
        Ok(())
    }
    fn publish(&mut self) -> Result<(), String> {
        self.revision += 1;
        let status = if !self.error.is_empty() {
            self.error.clone()
        } else if self.run.is_null() {
            "No workflow runs".into()
        } else {
            format!(
                "{} · {}",
                self.run["name"].as_str().unwrap_or("Workflow"),
                self.run["conclusion"]
                    .as_str()
                    .or(self.run["status"].as_str())
                    .unwrap_or("Unknown")
            )
        };
        send(&PluginMessage::View {
            compact: None,
            view_id: "main".into(),
            revision: self.revision,
            root: column(vec![
                text("title", "GitHub Actions"),
                text("status", &status),
                Node {
                    id: "actions".into(),
                    kind: NodeKind::Row,
                    spacing: 6.,
                    children: vec![
                        button("refresh", "Refresh", "refresh"),
                        button("open", "Open run", "open"),
                    ],
                    ..Node::default()
                },
            ]),
        })
    }
}
fn valid_repository(s: &str) -> bool {
    let p: Vec<_> = s.split('/').collect();
    p.len() == 2
        && p.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 100
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && *p != "."
                && *p != ".."
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repository_is_path_safe() {
        assert!(valid_repository("owner/my-repo"));
        for s in ["owner/../repo", "/repo", "o/r?token=x", "o/r#x", "a/b/c"] {
            assert!(!valid_repository(s))
        }
    }
}
