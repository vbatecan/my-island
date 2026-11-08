//! Blocking, bounded stdio helpers. stdout is reserved for protocol messages.
pub use serde_json;
pub use still_island_plugin_protocol::*;
pub fn send(message: &PluginMessage) -> Result<(), String> {
    write_message(&mut std::io::stdout().lock(), message)
}
pub fn run(mut handler: impl FnMut(HostMessage) -> Result<(), String>) -> Result<(), String> {
    let input = std::io::stdin();
    let mut input = input.lock();
    while let Some(message) = read_message::<HostMessage>(&mut input)? {
        if matches!(message, HostMessage::Shutdown) {
            break;
        }
        let action = match &message {
            HostMessage::Invoke { action_id, .. } => Some(action_id.clone()),
            _ => None,
        };
        if let Err(error) = handler(message) {
            if let Some(action_id) = action {
                failure(action_id, &error)?;
            } else {
                return Err(error);
            }
        }
    }
    Ok(())
}
pub fn text(id: &str, text: &str) -> Node {
    Node {
        id: id.into(),
        kind: NodeKind::Text,
        text: text.into(),
        ..Node::default()
    }
}
pub fn button(id: &str, label: &str, action: &str) -> Node {
    Node {
        id: id.into(),
        kind: NodeKind::Button,
        text: label.into(),
        action: action.into(),
        ..Node::default()
    }
}
pub fn column(children: Vec<Node>) -> Node {
    Node {
        id: "root".into(),
        kind: NodeKind::Column,
        spacing: 6.,
        children,
        ..Node::default()
    }
}
pub fn result(action_id: String, message: &str, open_uri: Option<String>) -> Result<(), String> {
    send(&PluginMessage::ActionResult {
        action_id,
        success: true,
        message: message.into(),
        open_uri,
    })
}
/// Recoverable action errors must acknowledge the action so the host does not
/// mistake validation or network failures for an unresponsive process.
pub fn failure(action_id: String, message: &str) -> Result<(), String> {
    send(&failure_message(action_id, message))
}
fn failure_message(action_id: String, message: &str) -> PluginMessage {
    PluginMessage::ActionResult {
        action_id,
        success: false,
        message: message.into(),
        open_uri: None,
    }
}
pub fn encode_uri(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recoverable_errors_acknowledge_the_same_action_without_a_uri() {
        match failure_message("refresh".into(), "Unavailable") {
            PluginMessage::ActionResult {
                action_id,
                success,
                message,
                open_uri,
            } => {
                assert_eq!(action_id, "refresh");
                assert!(!success);
                assert_eq!(message, "Unavailable");
                assert!(open_uri.is_none());
            }
            _ => panic!("Expected action acknowledgement"),
        }
    }
}
