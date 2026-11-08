//! Stable page identity, independent of plugin ordering or rendered geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Media,
    Timer,
    Volume,
    Plugin { id: String, view: String },
}

impl Page {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Plugin { id, view } => {
                still_island_plugin_protocol::valid_id(id)
                    && still_island_plugin_protocol::valid_id(view)
            }
            _ => true,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let page = match value {
            "home" => Self::Home,
            "media" => Self::Media,
            "timer" => Self::Timer,
            "volume" => Self::Volume,
            _ => {
                let rest = value.strip_prefix("plugin:")?;
                let (id, view) = rest.split_once(':')?;
                Self::Plugin {
                    id: id.into(),
                    view: view.into(),
                }
            }
        };
        page.is_valid().then_some(page)
    }

    pub fn encode(&self) -> String {
        match self {
            Self::Home => "home".into(),
            Self::Media => "media".into(),
            Self::Timer => "timer".into(),
            Self::Volume => "volume".into(),
            Self::Plugin { id, view } => format!("plugin:{id}:{view}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_identity_round_trips_without_index_coupling() {
        for value in [
            "home",
            "media",
            "timer",
            "volume",
            "plugin:org.stillisland.codex-usage:main",
        ] {
            assert_eq!(Page::parse(value).unwrap().encode(), value);
        }
    }

    #[test]
    fn malformed_or_unbounded_plugin_identity_is_rejected() {
        for value in [
            "",
            "automatic",
            "plugin:bad name:main",
            "plugin:../bad:main",
            "plugin:valid:",
            "plugin:valid:main:extra",
            "plugin:valid:main\nother",
        ] {
            assert!(Page::parse(value).is_none(), "{value}");
        }
        assert!(Page::parse(&format!("plugin:{}:main", "a".repeat(129))).is_none());
    }
}
