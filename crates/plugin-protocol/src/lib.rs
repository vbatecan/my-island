//! Versioned data-only contract between Still Island and trusted plugin processes.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::{BufRead, Write},
    path::{Component, Path},
};
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 65536;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub entrypoint: String,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub views: Vec<View>,
    #[serde(default)]
    pub config: Vec<ConfigField>,
    #[serde(default)]
    pub assets: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigField {
    pub id: String,
    pub label: String,
    pub kind: ConfigKind,
    #[serde(default)]
    pub default: Value,
    #[serde(default)]
    pub choices: Vec<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigKind {
    String,
    Secret,
    Bool,
    Integer,
    Number,
    Choice,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Neutral,
    Green,
    Amber,
    Red,
    Blue,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Row,
    #[default]
    Column,
    Text,
    Image,
    Button,
    Progress,
    Toggle,
    Input,
    List,
}
/// Semantic text hierarchy; omitted and unknown styles preserve body text.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextRole {
    Caption,
    Title,
    Metric,
    #[default]
    #[serde(other)]
    Body,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Node {
    pub text_role: TextRole,
    pub id: String,
    pub kind: NodeKind,
    pub text: String,
    pub action: String,
    pub asset: String,
    pub value: f64,
    pub checked: bool,
    pub children: Vec<Node>,
    pub spacing: f32,
    pub padding: f32,
    pub flex: f32,
    pub accent: Accent,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Init {
        protocol_version: u32,
        config: Value,
    },
    ViewRequest {
        view_id: String,
    },
    Invoke {
        action_id: String,
        #[serde(default)]
        value: Value,
    },
    Configure {
        config: Value,
    },
    Shutdown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginMessage {
    Ready {
        protocol_version: u32,
    },
    View {
        view_id: String,
        revision: u64,
        root: Node,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compact: Option<CompactSummary>,
    },
    Activity {
        id: String,
        title: String,
        body: String,
        #[serde(default)]
        accent: Accent,
    },
    ActionResult {
        action_id: String,
        success: bool,
        message: String,
        #[serde(default)]
        open_uri: Option<String>,
    },
    Error {
        message: String,
    },
}
/// Optional persistent presentation metadata; distinct from transient Activity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactSummary {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub accent: Accent,
}
pub fn validate_compact_summary(summary: &CompactSummary) -> Result<(), String> {
    if summary.title.trim().is_empty() || summary.title.len() > 128 || summary.body.len() > 256 {
        return Err("Invalid compact summary text".into());
    }
    if summary
        .progress
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err("Compact progress must be finite and normalized".into());
    }
    Ok(())
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && !id.ends_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
pub fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && path.split('/').all(|p| {
            let base = p.split('.').next().unwrap_or("").to_ascii_uppercase();
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with(['.', ' '])
                && !matches!(
                    base.as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
}

pub fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if !valid_id(&m.id)
        || m.name.trim().is_empty()
        || m.name.len() > 100
        || !valid_id(&m.version)
        || m.version.len() > 40
    {
        return Err("Invalid identity, name, or version".into());
    }
    if m.api_version != PROTOCOL_VERSION {
        return Err(format!(
            "Plugin requires protocol {}, host supports {}",
            m.api_version, PROTOCOL_VERSION
        ));
    }
    if !safe_relative_path(&m.entrypoint) || !m.entrypoint.to_ascii_lowercase().ends_with(".exe") {
        return Err("Entry point must be a relative Windows executable".into());
    }
    for ids in [
        m.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        m.views.iter().map(|v| v.id.as_str()).collect(),
        m.config.iter().map(|f| f.id.as_str()).collect(),
    ] {
        let mut seen = HashSet::new();
        if ids.len() > 64 || ids.iter().any(|id| !valid_id(id) || !seen.insert(*id)) {
            return Err("Invalid or duplicate declaration ID".into());
        }
    }
    if m.assets.len() > 128
        || m.assets.iter().any(|p| {
            !safe_relative_path(p)
                || !matches!(
                    Path::new(p).extension().and_then(|s| s.to_str()),
                    Some("png" | "svg" | "jpg" | "jpeg")
                )
        })
    {
        return Err("Invalid packaged asset".into());
    }
    Ok(())
}
pub fn validate_layout(root: &Node, m: &Manifest) -> Result<(), String> {
    fn visit(
        n: &Node,
        m: &Manifest,
        depth: usize,
        count: &mut usize,
        ids: &mut HashSet<String>,
    ) -> Result<(), String> {
        *count += 1;
        if depth > 8 || *count > 128 {
            return Err("Layout exceeds depth or node limit".into());
        }
        if !valid_id(&n.id) || !ids.insert(n.id.clone()) || n.text.len() > 4096 {
            return Err("Invalid/duplicate node ID or text".into());
        }
        if !n.value.is_finite()
            || !n.spacing.is_finite()
            || !n.padding.is_finite()
            || !n.flex.is_finite()
            || !(0.0..=32.0).contains(&n.spacing)
            || !(0.0..=32.0).contains(&n.padding)
            || !(0.0..=100.0).contains(&n.flex)
        {
            return Err("Invalid numeric layout value".into());
        }
        if !n.action.is_empty() && !m.actions.iter().any(|a| a.id == n.action) {
            return Err("Undeclared action".into());
        }
        if !n.asset.is_empty() && !m.assets.contains(&n.asset) {
            return Err("Undeclared asset".into());
        }
        if matches!(
            n.kind,
            NodeKind::Button | NodeKind::Toggle | NodeKind::Input
        ) && n.action.is_empty()
        {
            return Err("Interactive node needs action".into());
        }
        if !matches!(n.kind, NodeKind::Row | NodeKind::Column | NodeKind::List)
            && !n.children.is_empty()
        {
            return Err("Only containers accept children".into());
        }
        if n.kind == NodeKind::Progress && !(0.0..=1.0).contains(&n.value) {
            return Err("Progress must be normalized".into());
        }
        for c in &n.children {
            visit(c, m, depth + 1, count, ids)?
        }
        Ok(())
    }
    visit(root, m, 1, &mut 0, &mut HashSet::new())
}
pub fn read_message<T: serde::de::DeserializeOwned>(
    r: &mut impl BufRead,
) -> Result<Option<T>, String> {
    let mut bytes = Vec::new();
    loop {
        let buf = r.fill_buf().map_err(|e| e.to_string())?;
        if buf.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        let end = buf
            .iter()
            .position(|b| *b == b'\n')
            .map_or(buf.len(), |i| i + 1);
        if bytes.len() + end > MAX_MESSAGE_BYTES {
            return Err("Message exceeds 64 KiB".into());
        }
        bytes.extend_from_slice(&buf[..end]);
        let done = bytes.last() == Some(&b'\n');
        r.consume(end);
        if done {
            break;
        }
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| format!("Invalid protocol JSON: {e}"))
}
pub fn write_message<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    if bytes.len() + 1 > MAX_MESSAGE_BYTES {
        return Err("Message exceeds 64 KiB".into());
    }
    w.write_all(&bytes)
        .and_then(|_| w.write_all(b"\n"))
        .and_then(|_| w.flush())
        .map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_metadata_is_optional_and_round_trips() {
        let old = r#"{"type":"view","view_id":"main","revision":1,"root":{"id":"root"}}"#;
        let message: PluginMessage = serde_json::from_str(old).unwrap();
        assert!(matches!(
            &message,
            PluginMessage::View { compact: None, .. }
        ));
        assert!(
            serde_json::to_value(&message)
                .unwrap()
                .get("compact")
                .is_none()
        );
        let minimal: CompactSummary = serde_json::from_str(r#"{"title":"Codex"}"#).unwrap();
        assert_eq!(minimal.body, "");
        assert_eq!(minimal.progress, None);
        assert_eq!(minimal.accent, Accent::Neutral);
        let summary = CompactSummary {
            title: "Codex".into(),
            body: "Reported usage".into(),
            progress: Some(0.255),
            accent: Accent::Green,
        };
        let message = PluginMessage::View {
            view_id: "main".into(),
            revision: 2,
            root: Node {
                id: "root".into(),
                ..Node::default()
            },
            compact: Some(summary.clone()),
        };
        let decoded: PluginMessage =
            serde_json::from_str(&serde_json::to_string(&message).unwrap()).unwrap();
        assert!(
            matches!(decoded, PluginMessage::View { compact: Some(actual), .. } if actual == summary)
        );
    }
    #[test]
    fn compact_metadata_bounds_text_and_normalized_progress() {
        let mut summary = CompactSummary {
            title: "x".repeat(128),
            body: "y".repeat(256),
            ..CompactSummary::default()
        };
        for progress in [None, Some(0.), Some(0.255), Some(1.)] {
            summary.progress = progress;
            assert!(validate_compact_summary(&summary).is_ok());
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.001, 1.001] {
            summary.progress = Some(value);
            assert!(validate_compact_summary(&summary).is_err());
        }
        summary.progress = None;
        summary.title.push('x');
        assert!(validate_compact_summary(&summary).is_err());
        summary.title = "Codex".into();
        summary.body.push('y');
        assert!(validate_compact_summary(&summary).is_err());
        summary.body.clear();
        summary.title = " \t".into();
        assert!(validate_compact_summary(&summary).is_err());
    }
    fn manifest() -> Manifest {
        serde_json::from_str(r#"{"id":"test.plugin","name":"Test","version":"1.0.0","api_version":1,"entrypoint":"plugin.exe","actions":[{"id":"go","label":"Go"}]}"#).unwrap()
    }
    #[test]
    fn rejects_escape_and_duplicate_declarations() {
        let mut m = manifest();
        assert!(validate_manifest(&m).is_ok());
        m.entrypoint = "../escape.exe".into();
        assert!(validate_manifest(&m).is_err());
        for p in ["C:/x.exe", "a\\b.exe", "/x.exe", "a//x.exe"] {
            assert!(!safe_relative_path(p))
        }
    }
    #[test]
    fn rejects_invalid_layout() {
        let m = manifest();
        let mut n = Node {
            id: "root".into(),
            ..Node::default()
        };
        assert!(validate_layout(&n, &m).is_ok());
        n.children.push(n.clone());
        assert!(validate_layout(&n, &m).is_err());
        n.children.clear();
        n.kind = NodeKind::Button;
        assert!(validate_layout(&n, &m).is_err());
        n.action = "go".into();
        assert!(validate_layout(&n, &m).is_ok())
    }
    #[test]
    fn bounded_messages_and_version() {
        let data = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        assert!(read_message::<HostMessage>(&mut &data[..]).is_err());
        let mut m = manifest();
        m.api_version = 2;
        assert!(validate_manifest(&m).is_err());
        let mut output = Vec::new();
        write_message(&mut output, &HostMessage::Shutdown).unwrap();
        assert!(matches!(
            read_message::<HostMessage>(&mut &output[..]).unwrap(),
            Some(HostMessage::Shutdown)
        ))
    }
}
