//! Settings-window data binding, separate from application orchestration.
use crate::{
    NotificationApp, SettingsWindow,
    platform::MonitorInfo,
    settings::{Appearance, Bounce, Privacy, Settings},
};
use slint::{ModelRc, VecModel};
use std::rc::Rc;
pub fn populate(
    ui: &SettingsWindow,
    s: &Settings,
    monitors: &[MonitorInfo],
    apps: &[(String, String)],
    access: &str,
) {
    ui.set_appearance_index(s.appearance as i32);
    ui.set_tint(s.opacity);
    ui.set_bounce_index(match s.bounce {
        Bounce::Soft => 0,
        Bounce::Balanced => 1,
        Bounce::Lively => 2,
    });
    ui.set_animations_enabled(s.animations);
    ui.set_follow_reduced_motion(s.follow_reduced_motion);
    ui.set_top_offset(s.offset.round() as i32);
    ui.set_island_always_on_top(s.topmost);
    ui.set_notifications_enabled(s.notifications_enabled);
    ui.set_notifications_paused(s.notifications_paused);
    ui.set_notification_access(access.into());
    ui.set_privacy_index(s.privacy.index());
    ui.set_notification_duration(s.notification_duration as i32);
    ui.set_auto_volume(s.auto_volume);
    ui.set_compact_now_playing(s.compact_now_playing);
    ui.set_hover_expand(s.hover_expand);
    ui.set_hover_delay(s.hover_delay as i32);
    ui.set_outside_click(s.outside_click);
    ui.set_shortcut_enabled(s.shortcut_enabled);
    ui.set_fullscreen_hide(s.fullscreen_hide);
    let mut names = vec!["Primary display (automatic)".into()];
    names.extend(monitors.iter().map(|m| m.name.clone().into()));
    ui.set_monitor_names(ModelRc::from(Rc::new(VecModel::from(names))));
    ui.set_monitor_index(
        monitors
            .iter()
            .position(|m| m.id == s.monitor_id)
            .map_or(0, |i| i as i32 + 1),
    );
    ui.set_observed_apps(ModelRc::from(Rc::new(VecModel::from(
        apps.iter()
            .map(|(id, name)| NotificationApp {
                app_id: id.as_str().into(),
                app_name: name.as_str().into(),
                enabled: !s.blocked_apps.contains(id),
            })
            .collect::<Vec<_>>(),
    ))));
}
pub fn read(ui: &SettingsWindow, old: &Settings, monitors: &[MonitorInfo]) -> Settings {
    let mut s = old.clone();
    s.appearance = match ui.get_appearance_index() {
        1 => Appearance::Transparent,
        2 => Appearance::Acrylic,
        3 => Appearance::Glass,
        4 => Appearance::LiveGlass,
        _ => Appearance::Solid,
    };
    s.opacity = ui.get_tint().clamp(0.15, 0.95);
    s.bounce = match ui.get_bounce_index() {
        0 => Bounce::Soft,
        2 => Bounce::Lively,
        _ => Bounce::Balanced,
    };
    s.animations = ui.get_animations_enabled();
    s.follow_reduced_motion = ui.get_follow_reduced_motion();
    s.offset = ui.get_top_offset().clamp(0, 80) as f64;
    s.topmost = ui.get_island_always_on_top();
    s.notifications_enabled = ui.get_notifications_enabled();
    s.notifications_paused = ui.get_notifications_paused();
    s.privacy = match ui.get_privacy_index() {
        1 => Privacy::Identity,
        2 => Privacy::Body,
        _ => Privacy::Title,
    };
    s.notification_duration = ui.get_notification_duration().clamp(3, 10) as u32;
    s.auto_volume = ui.get_auto_volume();
    s.compact_now_playing = ui.get_compact_now_playing();
    s.hover_expand = ui.get_hover_expand();
    s.hover_delay = ui.get_hover_delay().clamp(200, 600) as u32;
    s.outside_click = ui.get_outside_click();
    s.shortcut_enabled = ui.get_shortcut_enabled();
    s.fullscreen_hide = ui.get_fullscreen_hide();
    s.monitor_id = if ui.get_monitor_index() > 0 {
        monitors
            .get(ui.get_monitor_index() as usize - 1)
            .map(|m| m.id.clone())
            .unwrap_or_default()
    } else {
        String::new()
    };
    s
}
#[cfg(test)]
mod tests {
    use crate::settings::*;
    #[test]
    fn old_settings_receive_new_defaults() {
        let s = Settings::parse("animations=false\nopacity=0.8\nbounce=lively\n");
        assert!(!s.animations);
        assert_eq!(s.opacity, 0.8);
        assert_eq!(s.bounce, Bounce::Lively);
        assert!(!s.notifications_enabled);
        assert!(s.auto_volume && s.fullscreen_hide);
        assert_eq!(s.hover_delay, 350);
    }
    #[test]
    fn all_settings_round_trip() {
        let mut s = Settings::default();
        s.monitor_id = "\\\\.\\DISPLAY2".into();
        s.notifications_enabled = true;
        s.blocked_apps = vec!["Example.App!App".into()];
        s.privacy = Privacy::Identity;
        assert_eq!(Settings::parse(&s.encode()).encode(), s.encode());
    }
    #[test]
    fn invalid_values_are_bounded() {
        let s =
            Settings::parse("opacity=NaN\nnotification_duration=999\nhover_delay=0\noffset=-12\n");
        assert_eq!(s.opacity, 0.55);
        assert_eq!(s.notification_duration, 10);
        assert_eq!(s.hover_delay, 200);
        assert_eq!(s.offset, 0.);
    }
}
