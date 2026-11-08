use still_island_plugin_sdk::*;
fn main() {
    if let Err(e) = plugin() {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
fn plugin() -> Result<(), String> {
    let mut config = serde_json::Value::Null;
    let mut note = String::new();
    let mut revision = 0;
    run(|message| {
        match message {
            HostMessage::Init {
                protocol_version,
                config: c,
            } => {
                if protocol_version != PROTOCOL_VERSION {
                    return Err("Unsupported protocol".into());
                }
                config = c;
                send(&PluginMessage::Ready {
                    protocol_version: PROTOCOL_VERSION,
                })?
            }
            HostMessage::Configure { config: c } => config = c,
            HostMessage::Invoke { action_id, value } => {
                let vault = config["vault"].as_str().unwrap_or("");
                match action_id.as_str() {
                    "text" => {
                        note = value.as_str().unwrap_or("").chars().take(2000).collect();
                        result(action_id, "Text updated", None)?
                    }
                    "daily" | "capture" => {
                        if vault.is_empty() {
                            failure(action_id, "Configure an Obsidian vault name first")?;
                        } else {
                            let uri = if action_id == "daily" {
                                format!("obsidian://daily?vault={}", encode_uri(vault))
                            } else {
                                format!(
                                    "obsidian://new?vault={}&name=Quick%20capture&content={}",
                                    encode_uri(vault),
                                    encode_uri(&note)
                                )
                            };
                            result(action_id, "Opening Obsidian", Some(uri))?;
                        }
                    }
                    _ => return Err("Unknown action".into()),
                }
            }
            HostMessage::ViewRequest { .. } => {}
            HostMessage::Shutdown => return Ok(()),
        }
        revision += 1;
        send(&PluginMessage::View {
            compact: None,
            view_id: "main".into(),
            revision,
            root: column(vec![
                text("title", "Obsidian quick capture"),
                Node {
                    id: "note".into(),
                    kind: NodeKind::Input,
                    text: note.clone(),
                    action: "text".into(),
                    ..Node::default()
                },
                Node {
                    id: "actions".into(),
                    kind: NodeKind::Row,
                    spacing: 6.,
                    children: vec![
                        button("daily", "Daily note", "daily"),
                        button("create", "Create note", "capture"),
                    ],
                    ..Node::default()
                },
            ]),
        })
    })
}
