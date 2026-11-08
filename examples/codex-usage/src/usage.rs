//! Only server-reported quota percentages are displayed; token counts are not quotas.
use std::time::{SystemTime, UNIX_EPOCH};
use still_island_plugin_sdk::{serde_json::Value, *};

#[derive(Debug)]
pub struct Window {
    pub used: f64,
    pub minutes: Option<u64>,
    pub reset: Option<u64>,
}

#[derive(Debug)]
pub struct Snapshot {
    pub windows: Vec<Window>,
    pub at: SystemTime,
}

impl Snapshot {
    pub fn parse(result: &Value, at: SystemTime) -> Result<Self, String> {
        let bucket = result
            .get("rateLimitsByLimitId")
            .and_then(|v| v.get("codex"))
            .filter(|v| !v.is_null())
            .or_else(|| {
                result.get("rateLimits").filter(|v| {
                    v.get("limitId")
                        .and_then(Value::as_str)
                        .is_none_or(|id| id == "codex")
                })
            })
            .ok_or("No Codex quota was returned. Sign in to Codex with ChatGPT, then refresh.")?;
        let mut windows = Vec::new();
        for key in ["primary", "secondary"] {
            let Some(window) = bucket.get(key).filter(|v| !v.is_null()) else {
                continue;
            };
            let used = window
                .get("usedPercent")
                .and_then(Value::as_f64)
                .filter(|p| p.is_finite() && (0.0..=100.).contains(p))
                .ok_or("Codex returned an invalid quota percentage. Update Codex and refresh.")?;
            windows.push(Window {
                used,
                minutes: window
                    .get("windowDurationMins")
                    .and_then(Value::as_u64)
                    .filter(|m| *m > 0),
                reset: window.get("resetsAt").and_then(Value::as_u64),
            });
        }
        if windows.is_empty() {
            return Err("Quota unavailable for this Codex sign-in. ChatGPT quota data is required; API-key usage is not estimated.".into());
        }
        Ok(Self { windows, at })
    }

    pub fn nodes(&self, now: SystemTime) -> Vec<Node> {
        let age = now.duration_since(self.at).unwrap_or_default();
        let columns = self
            .windows
            .iter()
            .enumerate()
            .map(|(index, window)| {
                let mut label = text(&format!("window-{index}"), &window.label());
                label.text_role = TextRole::Caption;
                let mut metric = text(
                    &format!("metric-{index}"),
                    &format!("{}%", percent(window.used)),
                );
                metric.text_role = TextRole::Metric;
                let mut remaining = text(
                    &format!("remaining-{index}"),
                    &format!("used · {}% left", percent(100. - window.used)),
                );
                remaining.text_role = TextRole::Caption;
                let progress = Node {
                    id: format!("progress-{index}"),
                    kind: NodeKind::Progress,
                    value: window.used / 100.,
                    accent: window.accent(),
                    ..Node::default()
                };
                let reset = window.reset.map_or_else(
                    || "Reset unavailable".into(),
                    |reset| {
                        let seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                        if reset <= seconds {
                            "Reset expected · refresh".into()
                        } else {
                            format!("Resets in {}", short_duration(reset - seconds))
                        }
                    },
                );
                let mut reset = text(&format!("reset-{index}"), &reset);
                reset.text_role = TextRole::Caption;
                Node {
                    id: format!("quota-{index}"),
                    kind: NodeKind::Column,
                    spacing: 2.,
                    flex: 1.,
                    children: vec![label, metric, remaining, progress, reset],
                    ..Node::default()
                }
            })
            .collect();
        let grid = Node {
            id: "quotas".into(),
            kind: NodeKind::Row,
            spacing: 22.,
            children: columns,
            ..Node::default()
        };
        let mut age = text(
            "snapshot",
            &format!(
                "Updated {} ago · refresh for latest",
                short_duration(age.as_secs())
            ),
        );
        age.text_role = TextRole::Caption;
        vec![grid, age]
    }

    /// Compact mode emphasizes the most constrained reported quota window.
    pub fn compact(&self) -> CompactSummary {
        let window = self
            .windows
            .iter()
            .max_by(|a, b| a.used.total_cmp(&b.used))
            .expect("validated quota has a window");
        CompactSummary {
            title: format!("Codex · {}% used", percent(window.used)),
            body: format!("{} · {}% left", window.label(), percent(100. - window.used)),
            progress: Some(window.used / 100.),
            accent: window.accent(),
        }
    }

    pub fn preview(&self) -> PluginMessage {
        let summary = self.compact();
        PluginMessage::Activity {
            id: "quota".into(),
            title: summary.title,
            body: format!("{} · reported snapshot", summary.body),
            accent: summary.accent,
        }
    }
}

impl Window {
    fn label(&self) -> String {
        match self.minutes {
            Some(m) if m % 1440 == 0 => format!("{}-day limit", m / 1440),
            Some(m) if m % 60 == 0 => format!("{}-hour limit", m / 60),
            Some(m) => format!("{m}-minute limit"),
            None => "Quota window".into(),
        }
    }
    fn accent(&self) -> Accent {
        if self.used >= 95. {
            Accent::Red
        } else if self.used >= 80. {
            Accent::Amber
        } else {
            Accent::Green
        }
    }
}

fn percent(value: f64) -> String {
    if value.fract().abs() < 0.0001 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}
fn short_duration(seconds: u64) -> String {
    if seconds >= 86400 {
        format!("{}d {}h", seconds / 86400, seconds % 86400 / 3600)
    } else if seconds >= 3600 {
        format!("{}h {}m", seconds / 3600, seconds % 3600 / 60)
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use still_island_plugin_sdk::serde_json::json;
    fn leaves(nodes: &[Node]) -> Vec<&Node> {
        let mut result = Vec::new();
        for node in nodes {
            result.push(node);
            result.extend(leaves(&node.children));
        }
        result
    }
    #[test]
    fn prefers_general_codex_bucket_and_keeps_two_independent_windows() {
        let s = Snapshot::parse(&json!({"rateLimits":{"limitId":"other","primary":{"usedPercent":99}}, "rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":25.5,"windowDurationMins":300},"secondary":{"usedPercent":70,"windowDurationMins":10080}}}}), UNIX_EPOCH).unwrap();
        assert_eq!(s.windows.len(), 2);
        assert_eq!(s.windows[0].used, 25.5);
        assert_eq!(s.windows[0].label(), "5-hour limit");
        assert_eq!(s.windows[1].label(), "7-day limit");
        assert!(
            leaves(&s.nodes(UNIX_EPOCH))
                .iter()
                .any(|n| n.text.contains("74.5% left"))
        );
    }
    #[test]
    fn legacy_missing_window_and_unavailable_are_not_fabricated() {
        assert_eq!(
            Snapshot::parse(
                &json!({"rateLimits":{"primary":{"usedPercent":0},"secondary":null}}),
                UNIX_EPOCH
            )
            .unwrap()
            .windows
            .len(),
            1
        );
        for value in [
            json!({}),
            json!({"rateLimits":{"primary":null}}),
            json!({"rateLimits":{"limitId":"other","primary":{"usedPercent":5}}}),
        ] {
            assert!(Snapshot::parse(&value, UNIX_EPOCH).is_err());
        }
    }
    #[test]
    fn invalid_percentages_do_not_become_zero_or_full_progress() {
        for value in [json!(-1), json!(101), json!("25"), Value::Null] {
            assert!(
                Snapshot::parse(
                    &json!({"rateLimits":{"primary":{"usedPercent":value}}}),
                    UNIX_EPOCH
                )
                .is_err()
            );
        }
    }
    #[test]
    fn elapsed_reset_keeps_reported_usage_and_asks_for_refresh() {
        let s = Snapshot::parse(
            &json!({"rateLimits":{"primary":{"usedPercent":100,"resetsAt":10}}}),
            UNIX_EPOCH,
        )
        .unwrap();
        let nodes = s.nodes(UNIX_EPOCH + Duration::from_secs(20));
        assert!(
            leaves(&nodes)
                .iter()
                .any(|n| n.text.contains("Reset expected"))
        );
        assert!(
            leaves(&nodes)
                .iter()
                .any(|n| n.kind == NodeKind::Progress && n.value == 1.)
        );
    }
    #[test]
    fn omitted_and_unknown_text_roles_keep_legacy_body_style() {
        for input in [
            json!({"id":"legacy","kind":"text"}),
            json!({"id":"new","kind":"text","text_role":"future_style"}),
        ] {
            let node: Node = serde_json::from_value(input).unwrap();
            assert_eq!(node.text_role, TextRole::Body);
        }
    }
    #[test]
    fn compact_uses_most_constrained_window_without_inventing_quota() {
        let s = Snapshot::parse(&json!({"rateLimits":{"primary":{"usedPercent":25,"windowDurationMins":300},"secondary":{"usedPercent":82,"windowDurationMins":10080}}}), UNIX_EPOCH).unwrap();
        let compact = s.compact();
        assert_eq!(compact.title, "Codex · 82% used");
        assert_eq!(compact.body, "7-day limit · 18% left");
        assert_eq!(compact.progress, Some(0.82));
        assert_eq!(compact.accent, Accent::Amber);
        let nodes = s.nodes(UNIX_EPOCH);
        assert_eq!(
            leaves(&nodes)
                .iter()
                .filter(|n| n.text_role == TextRole::Metric)
                .count(),
            2
        );
    }
    #[test]
    fn manifest_and_rendered_view_obey_host_protocol() {
        let manifest: Manifest = serde_json::from_str(include_str!("../manifest.json")).unwrap();
        validate_manifest(&manifest).unwrap();
        let s = Snapshot::parse(
            &json!({"rateLimits":{"primary":{"usedPercent":42}}}),
            UNIX_EPOCH,
        )
        .unwrap();
        validate_layout(&column(s.nodes(UNIX_EPOCH)), &manifest).unwrap();
    }
}
