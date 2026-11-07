use std::{fs, path::PathBuf};
#[derive(Clone, Debug)]
pub struct Settings {
    pub animations: bool,
    pub topmost: bool,
    pub offset: f64,
    pub appearance: Appearance,
    pub opacity: f32,
    pub bounce: Bounce,
    pub follow_reduced_motion: bool,
    pub notifications_enabled: bool,
    pub notifications_paused: bool,
    pub privacy: Privacy,
    pub notification_duration: u32,
    pub blocked_apps: Vec<String>,
    pub auto_volume: bool,
    pub compact_now_playing: bool,
    pub hover_expand: bool,
    pub hover_delay: u32,
    pub outside_click: bool,
    pub shortcut_enabled: bool,
    pub fullscreen_hide: bool,
    pub monitor_id: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            animations: true,
            topmost: true,
            offset: 6.,
            appearance: Appearance::Solid,
            opacity: 0.55,
            bounce: Bounce::Balanced,
            follow_reduced_motion: true,
            notifications_enabled: false,
            notifications_paused: false,
            privacy: Privacy::Title,
            notification_duration: 5,
            blocked_apps: vec![],
            auto_volume: true,
            compact_now_playing: false,
            hover_expand: false,
            hover_delay: 350,
            outside_click: true,
            shortcut_enabled: true,
            fullscreen_hide: true,
            monitor_id: String::new(),
        }
    }
}
impl Settings {
    pub fn path() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("StillIsland")
            .join("settings.ini")
    }
    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k {
                    "animations" => s.animations = v != "false",
                    "topmost" => s.topmost = v != "false",
                    "offset" => {
                        if let Ok(v) = v.parse::<f64>() {
                            if v.is_finite() {
                                s.offset = v.clamp(0., 80.);
                            }
                        }
                    }
                    "appearance" => s.appearance = Appearance::parse(v),
                    "opacity" => {
                        if let Ok(v) = v.parse::<f32>() {
                            if v.is_finite() {
                                s.opacity = v.clamp(0.15, 0.95);
                            }
                        }
                    }
                    "bounce" => s.bounce = Bounce::parse(v),
                    "follow_reduced_motion" => s.follow_reduced_motion = v != "false",
                    "notifications_enabled" => s.notifications_enabled = v == "true",
                    "notifications_paused" => s.notifications_paused = v == "true",
                    "privacy" => s.privacy = Privacy::parse(v),
                    "notification_duration" => {
                        if let Ok(v) = v.parse::<u32>() {
                            s.notification_duration = v.clamp(3, 10)
                        }
                    }
                    "blocked_app" => {
                        if !v.is_empty() && !s.blocked_apps.iter().any(|a| a == v) {
                            s.blocked_apps.push(v.to_owned())
                        }
                    }
                    "auto_volume" => s.auto_volume = v != "false",
                    "compact_now_playing" => s.compact_now_playing = v == "true",
                    "hover_expand" => s.hover_expand = v == "true",
                    "hover_delay" => {
                        if let Ok(v) = v.parse::<u32>() {
                            s.hover_delay = v.clamp(200, 600)
                        }
                    }
                    "outside_click" => s.outside_click = v != "false",
                    "shortcut_enabled" => s.shortcut_enabled = v != "false",
                    "fullscreen_hide" => s.fullscreen_hide = v != "false",
                    "monitor_id" => s.monitor_id = v.to_owned(),
                    _ => {}
                }
            }
        }
        s
    }
    pub fn encode(&self) -> String {
        let mut text = format!(
            "animations={}\ntopmost={}\noffset={}\nappearance={}\nopacity={}\nbounce={}\nfollow_reduced_motion={}\nnotifications_enabled={}\nnotifications_paused={}\nprivacy={}\nnotification_duration={}\nauto_volume={}\ncompact_now_playing={}\nhover_expand={}\nhover_delay={}\noutside_click={}\nshortcut_enabled={}\nfullscreen_hide={}\nmonitor_id={}\n",
            self.animations,
            self.topmost,
            self.offset,
            self.appearance.key(),
            self.opacity,
            self.bounce.key(),
            self.follow_reduced_motion,
            self.notifications_enabled,
            self.notifications_paused,
            self.privacy.key(),
            self.notification_duration,
            self.auto_volume,
            self.compact_now_playing,
            self.hover_expand,
            self.hover_delay,
            self.outside_click,
            self.shortcut_enabled,
            self.fullscreen_hide,
            self.monitor_id
        );
        for app in &self.blocked_apps {
            if !app.contains(['\n', '\r']) {
                text.push_str(&format!("blocked_app={app}\n"));
            }
        }
        text
    }
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        fs::create_dir_all(path.parent().unwrap())?;
        let temp = path.with_extension("tmp");
        fs::write(&temp, self.encode())?;
        fs::rename(temp, path)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Privacy {
    #[default]
    Title,
    Identity,
    Body,
}
impl Privacy {
    pub fn parse(v: &str) -> Self {
        match v {
            "identity" => Self::Identity,
            "body" => Self::Body,
            _ => Self::Title,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Identity => "identity",
            Self::Body => "body",
        }
    }
    pub fn index(self) -> i32 {
        match self {
            Self::Title => 0,
            Self::Identity => 1,
            Self::Body => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum Appearance {
    #[default]
    Solid = 0,
    Transparent = 1,
    Acrylic = 2,
    Glass = 3,
    LiveGlass = 4,
}
impl Appearance {
    pub fn parse(v: &str) -> Self {
        match v {
            "transparent" => Self::Transparent,
            "acrylic" => Self::Acrylic,
            "glass" => Self::Glass,
            "live-glass" => Self::LiveGlass,
            _ => Self::Solid,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Transparent => "transparent",
            Self::Acrylic => "acrylic",
            Self::Glass => "glass",
            Self::LiveGlass => "live-glass",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Transparent => "Transparent",
            Self::Acrylic => "Acrylic",
            Self::Glass => "Eco glass",
            Self::LiveGlass => "Live glass",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bounce {
    Soft,
    #[default]
    Balanced,
    Lively,
}
impl Bounce {
    pub fn parse(v: &str) -> Self {
        match v {
            "soft" => Self::Soft,
            "lively" => Self::Lively,
            _ => Self::Balanced,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Balanced => "balanced",
            Self::Lively => "lively",
        }
    }
    pub fn damping(self, collapsing: bool) -> f64 {
        match (self, collapsing) {
            (Self::Soft, _) => 42.,
            (Self::Balanced, false) => 34.,
            (Self::Balanced, true) => 39.,
            (Self::Lively, false) => 28.,
            (Self::Lively, true) => 35.,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Spring;
    #[test]
    fn presets_produce_ordered_visible_bounce_without_escaping_canvas() {
        let mut peaks = Vec::new();
        for preset in [Bounce::Soft, Bounce::Balanced, Bounce::Lively] {
            let mut spring = Spring::new(126.);
            spring.config.damping = preset.damping(false);
            spring.set_target(360.);
            let mut peak = 360f64;
            for _ in 0..240 {
                spring.advance(1. / 240.);
                peak = peak.max(spring.position);
            }
            assert!(spring.is_settled());
            assert!(peak < 418.);
            peaks.push(peak);
        }
        assert!(peaks[0] < peaks[1] && peaks[1] < peaks[2]);
        assert!(peaks[1] > 370. && peaks[1] < 385.);
        assert!(peaks[2] > 385.);
    }
    #[test]
    fn customization_values_round_trip() {
        for value in [
            Appearance::Solid,
            Appearance::Transparent,
            Appearance::Acrylic,
            Appearance::Glass,
            Appearance::LiveGlass,
        ] {
            assert_eq!(Appearance::parse(value.key()), value);
        }
        for value in [Bounce::Soft, Bounce::Balanced, Bounce::Lively] {
            assert_eq!(Bounce::parse(value.key()), value);
        }
    }
}
