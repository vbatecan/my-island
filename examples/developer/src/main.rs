use still_island_plugin_sdk::*;
fn main() {
    if let Err(e) = run_plugin() {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
fn run_plugin() -> Result<(), String> {
    let mut value = 0.;
    let mut revision = 0;
    let mut input = "Hello from a plugin".to_string();
    let mut checked = false;
    run(|message| {
        match message {
            HostMessage::Init {
                protocol_version, ..
            } => {
                if protocol_version != PROTOCOL_VERSION {
                    return Err("Unsupported protocol".into());
                }
                send(&PluginMessage::Ready {
                    protocol_version: PROTOCOL_VERSION,
                })?;
            }
            HostMessage::ViewRequest { .. } => {}
            HostMessage::Invoke {
                action_id,
                value: v,
            } => {
                match action_id.as_str() {
                    "increment" => {
                        value = if value >= 1. {
                            0.
                        } else {
                            (value + 0.1f64).min(1.)
                        }
                    }
                    "input" => input = v.as_str().unwrap_or("").chars().take(200).collect(),
                    "toggle" => checked = v.as_bool().unwrap_or(!checked),
                    "notify" => send(&PluginMessage::Activity {
                        id: "sample".into(),
                        title: "Plugin activity".into(),
                        body: "A native preview from a separate process".into(),
                        accent: Accent::Green,
                    })?,
                    _ => return Err("Unknown action".into()),
                }
                result(action_id, "Done", None)?
            }
            HostMessage::Configure { .. } => {}
            HostMessage::Shutdown => return Ok(()),
        }
        revision += 1;
        let root = column(vec![
            text("title", "Developer sample"),
            Node {
                id: "progress".into(),
                kind: NodeKind::Progress,
                value,
                ..Node::default()
            },
            Node {
                id: "row".into(),
                kind: NodeKind::Row,
                spacing: 6.,
                children: vec![
                    button("advance", "Advance", "increment"),
                    button("notify", "Preview", "notify"),
                ],
                ..Node::default()
            },
            Node {
                id: "toggle".into(),
                kind: NodeKind::Toggle,
                text: "Example toggle".into(),
                action: "toggle".into(),
                checked,
                ..Node::default()
            },
            Node {
                id: "input".into(),
                kind: NodeKind::Input,
                text: input.clone(),
                action: "input".into(),
                ..Node::default()
            },
        ]);
        send(&PluginMessage::View {
            compact: None,
            view_id: "main".into(),
            revision,
            root,
        })
    })
}
