use crate::navigation::Page;
use crate::shape::FormFactor;
use std::{fs, path::PathBuf};
#[derive(Clone, Debug)]
pub struct Settings {
    pub selected_page: Option<Page>,
    pub animations: bool,
    pub animation_speed: f64,
    pub contracted_width: f64,
    pub contracted_height: f64,
    pub expanded_width: f64,
    pub expanded_height: f64,
    pub media_width: f64,
    pub media_height: f64,

    pub topmost: bool,
    pub offset: f64,
    pub appearance: Appearance,
    pub form_factor: FormFactor,
    pub notch_corner_radius: f64,
    pub notch_contracted_radius: f64,
    pub island_scale: f64,
    pub text_scale: f64,
    pub fullscreen_always_hide: Vec<String>,
    pub fullscreen_always_show: Vec<String>,
    pub launch_at_startup: bool,
    pub battery_previews: bool,
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
    pub online_lyrics: bool,
    pub hover_expand: bool,
    pub hover_delay: u32,
    pub collapse_delay: u32,
    pub scroll_volume: bool,
    pub middle_mute: bool,
    pub volume_step: u32,
    pub outside_click: bool,
    pub shortcut_enabled: bool,
    pub launcher_enabled: bool,
    pub launcher_shortcut: u32,
    pub fullscreen_hide: bool,
    pub monitor_id: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            selected_page: None,
            launcher_enabled: true,
            launcher_shortcut: 0,
            animations: true,
            animation_speed: 1.,
            contracted_width: 126.0,
            contracted_height: 32.0,
            expanded_width: 360.0,
            expanded_height: 160.0,
            media_width: 640.0,
            media_height: 216.0,

            topmost: true,
            offset: 6.,
            appearance: Appearance::Solid,
            form_factor: FormFactor::Notch,
            notch_corner_radius: 8.,
            notch_contracted_radius: 8.,
            island_scale: 1.,
            text_scale: 1.,
            fullscreen_always_hide: vec![],
            fullscreen_always_show: vec![],
            launch_at_startup: false,
            battery_previews: true,
            opacity: 0.55,
            bounce: Bounce::Balanced,
            follow_reduced_motion: true,
            notifications_enabled: false,
            notifications_paused: false,
            privacy: Privacy::Title,
            notification_duration: 5,
            blocked_apps: vec![],
            auto_volume: true,
            compact_now_playing: true,
            online_lyrics: true,
            hover_expand: false,
            hover_delay: 350,
            collapse_delay: 350,
            scroll_volume: true,
            middle_mute: true,
            volume_step: 2,
            outside_click: true,
            shortcut_enabled: true,
            fullscreen_hide: true,
            monitor_id: String::new(),
        }
    }
}
impl Settings {
    /// Enforce supported values at every input boundary. The UI and INI parser
    /// share this policy so persisted settings cannot differ from live settings.
    pub fn normalize(&mut self) {
        self.launcher_shortcut = self.launcher_shortcut.min(2);
        let defaults = Self::default();
        if self
            .selected_page
            .as_ref()
            .is_some_and(|page| !page.is_valid())
        {
            self.selected_page = None;
        }
        self.text_scale = if self.text_scale.is_finite() {
            self.text_scale.clamp(0.8, 1.4)
        } else {
            1.
        };
        self.fullscreen_always_hide = executable_names(&self.fullscreen_always_hide.join(","));
        self.fullscreen_always_show = executable_names(&self.fullscreen_always_show.join(","));
        self.fullscreen_always_show
            .retain(|name| !self.fullscreen_always_hide.contains(name));
        let sizes = self.sizes().normalized();
        self.contracted_width = sizes.contracted_width;
        self.contracted_height = sizes.contracted_height;
        self.expanded_width = sizes.expanded_width;
        self.expanded_height = sizes.expanded_height;
        self.media_width = sizes.media_width;
        self.media_height = sizes.media_height;

        self.animation_speed = if self.animation_speed.is_finite() {
            self.animation_speed.clamp(0.5, 2.)
        } else {
            defaults.animation_speed
        };
        self.offset = if self.offset.is_finite() {
            self.offset.clamp(0., 80.)
        } else {
            defaults.offset
        };
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.15, 0.95)
        } else {
            defaults.opacity
        };
        self.island_scale = if self.island_scale.is_finite() {
            self.island_scale.clamp(0.75, 1.5)
        } else {
            defaults.island_scale
        };
        for (radius, fallback) in [
            (&mut self.notch_corner_radius, defaults.notch_corner_radius),
            (
                &mut self.notch_contracted_radius,
                defaults.notch_contracted_radius,
            ),
        ] {
            *radius = if radius.is_finite() {
                radius.clamp(0., 24.)
            } else {
                fallback
            };
        }
        self.notification_duration = self.notification_duration.clamp(3, 10);
        self.hover_delay = self.hover_delay.clamp(200, 2000);
        self.collapse_delay = self.collapse_delay.min(2000);
        self.volume_step = self.volume_step.clamp(1, 10);
    }
    pub fn sizes(&self) -> crate::state::SizePreferences {
        crate::state::SizePreferences {
            contracted_width: self.contracted_width,
            contracted_height: self.contracted_height,
            expanded_width: self.expanded_width,
            expanded_height: self.expanded_height,
            media_width: self.media_width,
            media_height: self.media_height,
        }
    }
    /// Layout presets affect presentation only, preserving system/integration choices.
    pub fn apply_layout_preset(&mut self, preset: i32) -> bool {
        let values = match preset {
            0 => (126., 32., 360., 160., 640., 216., 1.),
            1 => (180., 40., 480., 200., 800., 280., 1.1),
            2 => (150., 40., 420., 190., 720., 260., 1.25),
            _ => return false,
        };
        (
            self.contracted_width,
            self.contracted_height,
            self.expanded_width,
            self.expanded_height,
            self.media_width,
            self.media_height,
            self.text_scale,
        ) = values;
        self.island_scale = 1.;
        self.normalize();
        true
    }
    pub fn canvas_size(&self) -> (f64, f64) {
        self.sizes().canvas_size()
    }
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
                    "text_scale" => {
                        if let Ok(value) = v.parse() {
                            s.text_scale = value;
                        }
                    }
                    "fullscreen_always_hide" => {
                        s.fullscreen_always_hide.extend(executable_names(v))
                    }
                    "fullscreen_always_show" => {
                        s.fullscreen_always_show.extend(executable_names(v))
                    }
                    "selected_page" => s.selected_page = Page::parse(v),
                    "animations" => s.animations = v != "false",
                    "contracted_width" => {
                        if let Ok(value) = v.parse::<f64>() {
                            s.contracted_width = value;
                        }
                    }
                    "contracted_height" => {
                        if let Ok(value) = v.parse::<f64>() {
                            s.contracted_height = value;
                        }
                    }
                    "expanded_width" => {
                        if let Ok(value) = v.parse::<f64>() {
                            s.expanded_width = value;
                        }
                    }
                    "expanded_height" => {
                        if let Ok(value) = v.parse::<f64>() {
                            s.expanded_height = value;
                        }
                    }
                    "media_width" => {
                        if let Ok(value) = v.parse::<f64>() {
                            s.media_width = value;
                        }
                    }
                    "media_height" => {
                        if let Ok(value) = v.parse::<f64>() {
                            s.media_height = value;
                        }
                    }

                    "animation_speed" => {
                        if let Ok(value) = v.parse::<f64>() {
                            s.animation_speed = value;
                        }
                    }
                    "topmost" => s.topmost = v != "false",
                    "offset" => {
                        if let Ok(v) = v.parse::<f64>()
                            && v.is_finite()
                        {
                            s.offset = v;
                        }
                    }
                    "form_factor" => s.form_factor = FormFactor::parse(v),
                    "island_scale" => {
                        if let Ok(v) = v.parse::<f64>() {
                            s.island_scale = v;
                        }
                    }
                    "launch_at_startup" => s.launch_at_startup = v == "true",
                    "notch_corner_radius" => {
                        if let Ok(v) = v.parse::<f64>() {
                            s.notch_corner_radius = v;
                        }
                    }
                    "notch_contracted_radius" => {
                        if let Ok(v) = v.parse::<f64>() {
                            s.notch_contracted_radius = v;
                        }
                    }
                    "battery_previews" => s.battery_previews = v != "false",
                    "appearance" => s.appearance = Appearance::parse(v),
                    "opacity" => {
                        if let Ok(v) = v.parse::<f32>()
                            && v.is_finite()
                        {
                            s.opacity = v;
                        }
                    }
                    "bounce" => s.bounce = Bounce::parse(v),
                    "follow_reduced_motion" => s.follow_reduced_motion = v != "false",
                    "notifications_enabled" => s.notifications_enabled = v == "true",
                    "notifications_paused" => s.notifications_paused = v == "true",
                    "privacy" => s.privacy = Privacy::parse(v),
                    "notification_duration" => {
                        if let Ok(v) = v.parse::<u32>() {
                            s.notification_duration = v
                        }
                    }
                    "blocked_app" => {
                        if !v.is_empty() && !s.blocked_apps.iter().any(|a| a == v) {
                            s.blocked_apps.push(v.to_owned())
                        }
                    }
                    "auto_volume" => s.auto_volume = v != "false",
                    "compact_now_playing" => s.compact_now_playing = v == "true",
                    "online_lyrics" => s.online_lyrics = v != "false",
                    "hover_expand" => s.hover_expand = v == "true",
                    "hover_delay" => {
                        if let Ok(v) = v.parse::<u32>() {
                            s.hover_delay = v
                        }
                    }
                    "collapse_delay" => {
                        if let Ok(v) = v.parse::<u32>() {
                            s.collapse_delay = v;
                        }
                    }
                    "scroll_volume" => s.scroll_volume = v != "false",
                    "middle_mute" => s.middle_mute = v != "false",
                    "volume_step" => {
                        if let Ok(v) = v.parse::<u32>() {
                            s.volume_step = v;
                        }
                    }
                    "outside_click" => s.outside_click = v != "false",
                    "shortcut_enabled" => s.shortcut_enabled = v != "false",
                    "launcher_enabled" => s.launcher_enabled = v != "false",
                    "launcher_shortcut" => {
                        if let Ok(value) = v.parse() {
                            s.launcher_shortcut = value;
                        }
                    }
                    "fullscreen_hide" => s.fullscreen_hide = v != "false",
                    "monitor_id" => s.monitor_id = v.to_owned(),
                    _ => {}
                }
            }
        }
        s.normalize();
        s
    }
    pub fn encode(&self) -> String {
        // Only known chord indices are persisted; no executable command input.
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
        text.push_str(&format!(
            "launcher_enabled={}\nlauncher_shortcut={}\n",
            self.launcher_enabled, self.launcher_shortcut
        ));
        if let Some(page) = &self.selected_page
            && page.is_valid()
        {
            text.push_str(&format!("selected_page={}\n", page.encode()));
        }
        text.push_str(&format!("text_scale={}\n", self.text_scale));
        for name in executable_names(&self.fullscreen_always_hide.join(",")) {
            text.push_str(&format!("fullscreen_always_hide={name}\n"));
        }
        for name in executable_names(&self.fullscreen_always_show.join(",")) {
            text.push_str(&format!("fullscreen_always_show={name}\n"));
        }
        text.push_str(&format!("animation_speed={}\n", self.animation_speed));
        text.push_str(&format!(
            "collapse_delay={}\nscroll_volume={}\nmiddle_mute={}\nvolume_step={}\n",
            self.collapse_delay, self.scroll_volume, self.middle_mute, self.volume_step
        ));
        text.push_str(&format!(
            "form_factor={}\nbattery_previews={}\nnotch_corner_radius={}\nnotch_contracted_radius={}\nonline_lyrics={}\n",
            self.form_factor.key(),
            self.battery_previews,
            self.notch_corner_radius,
            self.notch_contracted_radius,
            self.online_lyrics
        ));
        text.push_str(&format!(
            "island_scale={}\nlaunch_at_startup={}\n",
            self.island_scale, self.launch_at_startup
        ));
        text.push_str(&format!("contracted_width={}\n", self.contracted_width));
        text.push_str(&format!("contracted_height={}\n", self.contracted_height));
        text.push_str(&format!("expanded_width={}\n", self.expanded_width));
        text.push_str(&format!("expanded_height={}\n", self.expanded_height));
        text.push_str(&format!("media_width={}\n", self.media_width));
        text.push_str(&format!("media_height={}\n", self.media_height));
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
/// Startup/fullscreen matching uses basenames, never arbitrary paths or patterns.
pub fn executable_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for name in text
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(name) = crate::platform::normalize_fullscreen_executable(name)
            && !names.contains(&name)
        {
            names.push(name);
        }
        if names.len() == 64 {
            break;
        }
    }
    names
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Privacy {
    #[default]
    Title,
    Identity,
    Body,
}
impl Privacy {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Identity,
            2 => Self::Body,
            _ => Self::Title,
        }
    }
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
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Transparent,
            2 => Self::Acrylic,
            3 => Self::Glass,
            4 => Self::LiveGlass,
            _ => Self::Solid,
        }
    }
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
    pub fn index(self) -> i32 {
        match self {
            Self::Soft => 0,
            Self::Balanced => 1,
            Self::Lively => 2,
        }
    }
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Soft,
            2 => Self::Lively,
            _ => Self::Balanced,
        }
    }
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
    fn dimension_preferences_migrate_bound_and_persist() {
        let old = Settings::parse("animations=false\n");
        assert_eq!(old.sizes(), crate::state::SizePreferences::default());
        let configured = Settings::parse(
            "contracted_width=240\ncontracted_height=56\nexpanded_width=720\nexpanded_height=300\nmedia_width=960\nmedia_height=380\n",
        );
        assert_eq!(
            Settings::parse(&configured.encode()).sizes(),
            configured.sizes()
        );
        assert_eq!(configured.contracted_width, 240.);
        assert_eq!(configured.media_height, 380.);
        let invalid = Settings::parse(
            "contracted_width=NaN\ncontracted_height=999\nexpanded_width=-1\nexpanded_height=inf\nmedia_width=9999\nmedia_height=2\n",
        );
        assert_eq!(invalid.contracted_width, 126.);
        assert_eq!(invalid.contracted_height, 64.);
        assert_eq!(invalid.expanded_width, 320.);
        assert_eq!(invalid.expanded_height, 160.);
        assert_eq!(invalid.media_width, 1000.);
        assert_eq!(invalid.media_height, 216.);
    }
    #[test]
    fn animation_speed_migrates_validates_and_persists() {
        assert_eq!(Settings::parse("animations=true\n").animation_speed, 1.);
        let settings = Settings::parse("animation_speed=1.35\n");
        assert_eq!(Settings::parse(&settings.encode()).animation_speed, 1.35);
        for (value, expected) in [
            ("0.1", 0.5),
            ("5", 2.),
            ("NaN", 1.),
            ("inf", 1.),
            ("bad", 1.),
        ] {
            assert_eq!(
                Settings::parse(&format!("animation_speed={value}\n")).animation_speed,
                expected
            );
        }
        let mut live = Settings {
            animation_speed: f64::NEG_INFINITY,
            ..Settings::default()
        };
        live.normalize();
        assert_eq!(live.animation_speed, 1.);
    }
    #[test]
    fn notch_migration_and_shape_preferences_round_trip() {
        assert_eq!(
            Settings::parse("offset=19\n").form_factor,
            FormFactor::Notch
        );
        let s = Settings::parse("form_factor=island\nbattery_previews=false\noffset=19\n");
        assert_eq!(s.form_factor, FormFactor::Island);
        assert!(!s.battery_previews);
        assert_eq!(Settings::parse(&s.encode()).encode(), s.encode());
        assert_eq!(s.offset, 19.);
    }
    #[test]
    fn notch_radius_and_lyrics_preferences_migrate_validate_and_persist() {
        let defaults = Settings::parse("animations=false\n");
        assert_eq!(defaults.notch_corner_radius, 8.);
        assert!(defaults.online_lyrics);
        let configured = Settings::parse("notch_corner_radius=11.5\nonline_lyrics=false\n");
        assert_eq!(configured.notch_corner_radius, 11.5);
        assert!(!configured.online_lyrics);
        assert_eq!(
            Settings::parse(&configured.encode()).encode(),
            configured.encode()
        );
        for (value, expected) in [("-1", 0.), ("99", 24.), ("NaN", 8.), ("inf", 8.)] {
            let parsed = Settings::parse(&format!("notch_corner_radius={value}\n"));
            assert_eq!(parsed.notch_corner_radius, expected);
        }
        let mut live = configured;
        live.notch_corner_radius = f64::NAN;
        live.normalize();
        assert_eq!(live.notch_corner_radius, 8.);
    }
    #[test]
    fn contracted_radius_migrates_and_persists_independently_of_expanded() {
        let migrated = Settings::parse("notch_corner_radius=18\n");
        assert_eq!(migrated.notch_contracted_radius, 8.);
        assert_eq!(migrated.notch_corner_radius, 18.);
        let configured = Settings::parse("notch_corner_radius=18\nnotch_contracted_radius=5.5\n");
        assert_eq!(configured.notch_contracted_radius, 5.5);
        assert_eq!(
            Settings::parse(&configured.encode()).encode(),
            configured.encode()
        );
        for (value, expected) in [
            ("-1", 0.),
            ("99", 24.),
            ("NaN", 8.),
            ("inf", 8.),
            ("bad", 8.),
        ] {
            let parsed = Settings::parse(&format!("notch_contracted_radius={value}\n"));
            assert_eq!(parsed.notch_contracted_radius, expected);
            assert_eq!(parsed.notch_corner_radius, 8.);
        }
        let mut live = configured;
        live.notch_contracted_radius = f64::INFINITY;
        live.normalize();
        assert_eq!(live.notch_contracted_radius, 8.);
        assert_eq!(live.notch_corner_radius, 18.);
    }
    #[test]
    fn size_and_startup_preferences_migrate_validate_and_persist() {
        let old = Settings::parse("notch_contracted_radius=5\n");
        assert_eq!(old.island_scale, 1.);
        assert!(!old.launch_at_startup);
        let configured = Settings::parse("island_scale=1.25\nlaunch_at_startup=true\n");
        assert_eq!(configured.island_scale, 1.25);
        assert!(configured.launch_at_startup);
        assert_eq!(
            Settings::parse(&configured.encode()).encode(),
            configured.encode()
        );
        for (value, expected) in [
            ("0", 0.75),
            ("0.75", 0.75),
            ("1.5", 1.5),
            ("9", 1.5),
            ("NaN", 1.),
            ("inf", 1.),
            ("bad", 1.),
        ] {
            assert_eq!(
                Settings::parse(&format!("island_scale={value}\n")).island_scale,
                expected
            );
        }
        assert!(!Settings::parse("launch_at_startup=false\n").launch_at_startup);
        assert!(!Settings::parse("launch_at_startup=invalid\n").launch_at_startup);
        let mut live = configured;
        live.island_scale = f64::NEG_INFINITY;
        live.normalize();
        assert_eq!(live.island_scale, 1.);
        assert!(live.launch_at_startup);
    }
    #[test]
    fn parsed_and_live_settings_share_validation() {
        let mut live = Settings {
            offset: -1.,
            opacity: 9.,
            notification_duration: 99,
            hover_delay: 0,
            collapse_delay: u32::MAX,
            volume_step: 0,
            ..Settings::default()
        };
        live.normalize();
        let parsed = Settings::parse(
            "offset=-1\nopacity=9\nnotification_duration=99\nhover_delay=0\ncollapse_delay=4294967295\nvolume_step=0\n",
        );
        assert_eq!(live.encode(), parsed.encode());
        live.offset = f64::NAN;
        live.opacity = f32::INFINITY;
        live.normalize();
        assert_eq!(live.offset, Settings::default().offset);
        assert_eq!(live.opacity, Settings::default().opacity);
    }

    #[test]
    fn preference_indices_round_trip_and_unknown_values_fall_back() {
        for value in [
            Appearance::Solid,
            Appearance::Transparent,
            Appearance::Acrylic,
            Appearance::Glass,
            Appearance::LiveGlass,
        ] {
            assert_eq!(Appearance::from_index(value as i32), value);
        }
        for value in [Bounce::Soft, Bounce::Balanced, Bounce::Lively] {
            assert_eq!(Bounce::from_index(value.index()), value);
        }
        for value in [Privacy::Title, Privacy::Identity, Privacy::Body] {
            assert_eq!(Privacy::from_index(value.index()), value);
        }
        assert_eq!(Appearance::from_index(99), Appearance::Solid);
        assert_eq!(Bounce::from_index(-1), Bounce::Balanced);
        assert_eq!(Privacy::from_index(99), Privacy::Title);
    }
    #[test]
    fn interaction_preferences_round_trip_and_clamp() {
        let configured = Settings::parse(
            "collapse_delay=1250\nhover_delay=1750\nscroll_volume=false\nmiddle_mute=false\nvolume_step=7\n",
        );
        assert_eq!(configured.collapse_delay, 1250);
        assert_eq!(configured.hover_delay, 1750);
        assert_eq!(configured.volume_step, 7);
        assert!(!configured.scroll_volume && !configured.middle_mute);
        let restored = Settings::parse(&configured.encode());
        assert_eq!(restored.encode(), configured.encode());
        let maximum = Settings::parse(
            "collapse_delay=4294967295\nhover_delay=4294967295\nvolume_step=4294967295\n",
        );
        assert_eq!(maximum.collapse_delay, 2000);
        assert_eq!(maximum.hover_delay, 2000);
        assert_eq!(maximum.volume_step, 10);
        let minimum = Settings::parse("collapse_delay=0\nhover_delay=0\nvolume_step=0\n");
        assert_eq!(minimum.collapse_delay, 0);
        assert_eq!(minimum.hover_delay, 200);
        assert_eq!(minimum.volume_step, 1);
        let malformed = Settings::parse("collapse_delay=-1\nvolume_step=NaN\n");
        assert_eq!(malformed.collapse_delay, Settings::default().collapse_delay);
        assert_eq!(malformed.volume_step, Settings::default().volume_step);
    }
    #[test]
    fn now_playing_defaults_on_and_respects_saved_preference() {
        assert!(Settings::default().compact_now_playing);
        assert!(Settings::parse("offset=6\n").compact_now_playing);
        let disabled = Settings::parse("compact_now_playing=false\n");
        assert!(!disabled.compact_now_playing);
        assert!(!Settings::parse(&disabled.encode()).compact_now_playing);
    }

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
    #[test]
    fn text_scale_migrates_clamps_and_round_trips() {
        assert_eq!(Settings::parse("opacity=0.5\n").text_scale, 1.);
        for (raw, expected) in [
            ("0.1", 0.8),
            ("2", 1.4),
            ("NaN", 1.),
            ("inf", 1.),
            ("1.25", 1.25),
        ] {
            let settings = Settings::parse(&format!("text_scale={raw}\n"));
            assert_eq!(settings.text_scale, expected);
            assert_eq!(Settings::parse(&settings.encode()).text_scale, expected);
        }
    }
    #[test]
    fn fullscreen_rules_normalize_basename_lists_and_hide_wins() {
        assert_eq!(
            executable_names(" Game ; GAME.exe, player.exe, C:\\apps\\bad.exe, ../bad, "),
            vec!["game.exe", "player.exe"]
        );
        let settings =
            Settings::parse("fullscreen_always_hide=Game\nfullscreen_always_show=GAME.exe,Music\n");
        assert_eq!(settings.fullscreen_always_hide, vec!["game.exe"]);
        assert_eq!(settings.fullscreen_always_show, vec!["music.exe"]);
        assert_eq!(
            Settings::parse(&settings.encode()).encode(),
            settings.encode()
        );
        assert!(
            Settings::parse("offset=4\n")
                .fullscreen_always_hide
                .is_empty()
        );
    }
    #[test]
    fn layout_presets_preserve_integrations_and_material() {
        for index in 0..3 {
            let mut settings = Settings {
                appearance: Appearance::Acrylic,
                opacity: 0.8,
                launch_at_startup: true,
                fullscreen_always_hide: vec!["game.exe".into()],
                ..Settings::default()
            };
            assert!(settings.apply_layout_preset(index));
            assert_eq!(settings.appearance, Appearance::Acrylic);
            assert_eq!(settings.opacity, 0.8);
            assert!(settings.launch_at_startup);
            assert_eq!(settings.fullscreen_always_hide, vec!["game.exe"]);
            assert_eq!(
                Settings::parse(&settings.encode()).encode(),
                settings.encode()
            );
        }
        let mut settings = Settings::default();
        let before = settings.encode();
        assert!(!settings.apply_layout_preset(99));
        assert_eq!(settings.encode(), before);
    }
    #[test]
    fn selected_page_migrates_and_round_trips_without_plugin_indexes() {
        assert_eq!(Settings::parse("opacity=0.5\n").selected_page, None);
        for page in [
            "home",
            "media",
            "timer",
            "volume",
            "plugin:org.stillisland.codex-usage:main",
        ] {
            let settings = Settings::parse(&format!("selected_page={page}\n"));
            assert_eq!(settings.selected_page, Page::parse(page));
            assert_eq!(
                Settings::parse(&settings.encode()).selected_page,
                settings.selected_page
            );
        }
        for invalid in ["plugin:bad name:main", "plugin:valid:../main", "unknown"] {
            assert_eq!(
                Settings::parse(&format!("selected_page={invalid}\n")).selected_page,
                None
            );
        }
        let mut settings = Settings {
            selected_page: Some(Page::Plugin {
                id: "invalid/path".into(),
                view: "main".into(),
            }),
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.selected_page, None);
    }
}
