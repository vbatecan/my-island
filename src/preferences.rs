//! Settings-window data binding, separate from application orchestration.
use crate::{
    NotificationApp, SettingsWindow,
    platform::MonitorInfo,
    settings::{Appearance, Bounce, Privacy, Settings},
};
use slint::{ModelRc, VecModel};
use std::rc::Rc;
/// Each Slint window has its own GL context and owns its own borrowed images.
pub fn install_preview_renderer(ui: &SettingsWindow) {
    use slint::ComponentHandle;
    let weak = ui.as_weak();
    let mut renderer: Option<crate::liquid::Renderer> = None;
    let mut sample = crate::material::Backdrop {
        width: 512,
        height: 256,
        pixels: Vec::with_capacity(512 * 256 * 4),
    };
    for y in 0..256 {
        for x in 0..512 {
            sample
                .pixels
                .extend_from_slice(if (x / 60 + y / 40) % 2 == 0 {
                    &[215, 222, 235, 255]
                } else {
                    &[75, 117, 150, 255]
                });
        }
    }
    let result = ui.window().set_rendering_notifier(move |state, api| {
        if matches!(state, slint::RenderingState::RenderingTeardown) {
            if let Some(ui) = weak.upgrade() {
                ui.set_liquid_preview(slint::Image::default());
            }
            if let Some(gpu) = renderer.take() {
                unsafe {
                    gpu.destroy();
                }
            }
            return;
        }
        let Some(ui) = weak.upgrade() else { return };
        match state {
            slint::RenderingState::RenderingSetup => {
                if let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api {
                    // SAFETY: notifier owns this current GL context.
                    renderer = unsafe {
                        crate::liquid::Renderer::new(glow::Context::from_loader_function_cstr(
                            get_proc_address,
                        ))
                    }
                    .ok();
                }
            }
            slint::RenderingState::BeforeRendering if ui.get_appearance_index() == 5 => {
                let Some(gpu) = renderer.as_mut() else { return };
                let form = if ui.get_form_factor_index() == 0 {
                    crate::shape::FormFactor::Notch
                } else {
                    crate::shape::FormFactor::Island
                };
                let w = ui.get_preview_width();
                let h = ui.get_preview_height();
                let radius = if ui.get_preview_state() == 0 {
                    ui.get_notch_contracted_radius()
                } else {
                    ui.get_notch_corner_radius()
                };
                let shoulder = if form == crate::shape::FormFactor::Notch {
                    8f32.min(h / 2.)
                } else {
                    0.
                };
                let contour = crate::shape::ShapeContour::new(
                    (w - 2. * shoulder) as f64,
                    h as f64,
                    radius as f64,
                    form,
                );
                let geometry =
                    crate::liquid::Geometry::new([w, h], contour, 1., ui.get_tint(), false);
                if let Ok(Some(image)) = gpu.render(&sample, 1, geometry, contour, true) {
                    ui.set_liquid_preview(image);
                }
            }
            slint::RenderingState::RenderingTeardown => {
                ui.set_liquid_preview(slint::Image::default());
                if let Some(gpu) = renderer.take() {
                    unsafe {
                        gpu.destroy();
                    }
                }
            }
            _ => {}
        }
    });
    if let Err(error) = result {
        eprintln!("Liquid preview renderer unavailable: {error:?}");
    }
}
pub fn populate(
    ui: &SettingsWindow,
    s: &Settings,
    monitors: &[MonitorInfo],
    apps: &[(String, String)],
    access: &str,
) {
    ui.set_appearance_index(s.appearance as i32);
    ui.set_form_factor_index(s.form_factor.index());
    ui.set_notch_corner_radius(s.notch_corner_radius.round() as i32);
    ui.set_notch_contracted_radius(s.notch_contracted_radius.round() as i32);
    ui.set_text_scale_percent((s.text_scale * 100.).round() as i32);
    ui.set_fullscreen_always_hide_text(s.fullscreen_always_hide.join(", ").into());
    ui.set_fullscreen_always_show_text(s.fullscreen_always_show.join(", ").into());
    let state = match ui.get_preview_state() {
        0 => crate::state::IslandState::Idle,
        2 => crate::state::IslandState::Media,
        _ => crate::state::IslandState::Expanded,
    };
    let geometry = state.geometry_for_sizes(
        s.form_factor,
        s.notch_corner_radius,
        s.notch_contracted_radius,
        s.canvas_size().0,
        &s.sizes(),
    );
    let contour = crate::shape::ShapeContour::new(
        geometry.width,
        geometry.height,
        geometry.radius,
        s.form_factor,
    );
    ui.set_preview_width(contour.total_width() as f32);
    ui.set_preview_height(geometry.height as f32);
    ui.set_preview_path(contour.svg_commands().into());
    ui.set_island_scale_percent((s.island_scale * 100.).round() as i32);
    ui.set_launch_at_startup(s.launch_at_startup);
    ui.set_battery_previews(s.battery_previews);
    ui.set_tint(s.opacity);
    ui.set_bounce_index(s.bounce.index());
    ui.set_contracted_width(s.contracted_width.round() as i32);
    ui.set_contracted_height(s.contracted_height.round() as i32);
    ui.set_expanded_width(s.expanded_width.round() as i32);
    ui.set_expanded_height(s.expanded_height.round() as i32);
    ui.set_media_width(s.media_width.round() as i32);
    ui.set_media_height(s.media_height.round() as i32);
    ui.set_animations_enabled(s.animations);
    ui.set_animation_speed_percent((s.animation_speed * 100.).round() as i32);
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
    ui.set_online_lyrics(s.online_lyrics);
    ui.set_hover_expand(s.hover_expand);
    ui.set_hover_delay(s.hover_delay as i32);
    ui.set_collapse_delay(s.collapse_delay as i32);
    ui.set_scroll_volume(s.scroll_volume);
    ui.set_middle_mute(s.middle_mute);
    ui.set_volume_step(s.volume_step as i32);
    ui.set_outside_click(s.outside_click);
    ui.set_shortcut_enabled(s.shortcut_enabled);
    ui.set_launcher_enabled(s.launcher_enabled);
    ui.set_launcher_windows(s.launcher_windows);
    ui.set_launcher_calculator(s.launcher_calculator);
    ui.set_launcher_currency(s.launcher_currency);
    ui.set_launcher_shortcut(s.launcher_shortcut as i32);
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
    s.appearance = Appearance::from_index(ui.get_appearance_index());
    s.form_factor = if ui.get_form_factor_index() == 1 {
        crate::shape::FormFactor::Island
    } else {
        crate::shape::FormFactor::Notch
    };
    s.battery_previews = ui.get_battery_previews();
    s.notch_corner_radius = ui.get_notch_corner_radius() as f64;
    s.notch_contracted_radius = ui.get_notch_contracted_radius() as f64;
    s.text_scale = ui.get_text_scale_percent() as f64 / 100.;
    s.fullscreen_always_hide =
        crate::settings::executable_names(&ui.get_fullscreen_always_hide_text());
    s.fullscreen_always_show =
        crate::settings::executable_names(&ui.get_fullscreen_always_show_text());
    s.island_scale = ui.get_island_scale_percent() as f64 / 100.;
    s.launch_at_startup = ui.get_launch_at_startup();
    s.opacity = ui.get_tint();
    s.bounce = Bounce::from_index(ui.get_bounce_index());
    s.contracted_width = ui.get_contracted_width() as f64;
    s.contracted_height = ui.get_contracted_height() as f64;
    s.expanded_width = ui.get_expanded_width() as f64;
    s.expanded_height = ui.get_expanded_height() as f64;
    s.media_width = ui.get_media_width() as f64;
    s.media_height = ui.get_media_height() as f64;
    s.animations = ui.get_animations_enabled();
    s.animation_speed = ui.get_animation_speed_percent() as f64 / 100.;
    s.follow_reduced_motion = ui.get_follow_reduced_motion();
    s.offset = ui.get_top_offset() as f64;
    s.topmost = ui.get_island_always_on_top();
    s.notifications_enabled = ui.get_notifications_enabled();
    s.notifications_paused = ui.get_notifications_paused();
    s.privacy = Privacy::from_index(ui.get_privacy_index());
    s.notification_duration = ui.get_notification_duration().max(0) as u32;
    s.auto_volume = ui.get_auto_volume();
    s.compact_now_playing = ui.get_compact_now_playing();
    s.online_lyrics = ui.get_online_lyrics();
    s.hover_expand = ui.get_hover_expand();
    s.hover_delay = ui.get_hover_delay().max(0) as u32;
    s.collapse_delay = ui.get_collapse_delay().max(0) as u32;
    s.scroll_volume = ui.get_scroll_volume();
    s.middle_mute = ui.get_middle_mute();
    s.volume_step = ui.get_volume_step().max(0) as u32;
    s.outside_click = ui.get_outside_click();
    s.shortcut_enabled = ui.get_shortcut_enabled();
    s.launcher_enabled = ui.get_launcher_enabled();
    s.launcher_windows = ui.get_launcher_windows();
    s.launcher_calculator = ui.get_launcher_calculator();
    s.launcher_currency = ui.get_launcher_currency();
    s.launcher_shortcut = ui.get_launcher_shortcut().max(0) as u32;
    s.fullscreen_hide = ui.get_fullscreen_hide();
    s.monitor_id = if ui.get_monitor_index() > 0 {
        monitors
            .get(ui.get_monitor_index() as usize - 1)
            .map(|m| m.id.clone())
            .unwrap_or_default()
    } else {
        String::new()
    };
    s.normalize();
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
        assert_eq!(s.notch_contracted_radius, 8.);
        assert_eq!(s.island_scale, 1.);
        assert!(!s.launch_at_startup);
    }
    #[test]
    fn all_settings_round_trip() {
        let s = Settings {
            monitor_id: "\\\\.\\DISPLAY2".into(),
            notifications_enabled: true,
            blocked_apps: vec!["Example.App!App".into()],
            privacy: Privacy::Identity,
            notch_contracted_radius: 6.,
            notch_corner_radius: 17.,
            island_scale: 1.25,
            text_scale: 1.2,
            fullscreen_always_hide: vec!["game.exe".into()],
            fullscreen_always_show: vec!["player.exe".into()],
            launch_at_startup: true,
            ..Settings::default()
        };
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
