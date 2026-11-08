//! Logical destinations are independent of the currently rendered geometry.
use crate::shape::{FormFactor, ShapeContour};

// A persistent transparent surface leaves room for the wide media view and the
// lively spring's overshoot. The native window and renderer share these bounds.
pub const OVERLAY_WIDTH: f64 = 800.0;
pub const OVERLAY_HEIGHT: f64 = 300.0;
pub const EXPANDED_RADIUS: f64 = 12.0;
pub const NOTCH_EXPANDED_RADIUS: f64 = 8.0;
const DISPLAY_MARGIN: f64 = 24.0;
const OVERSHOOT_ALLOWANCE: f64 = 0.2;
/// Base layout dimensions, independent of user zoom and Windows DPI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizePreferences {
    pub contracted_width: f64,
    pub contracted_height: f64,
    pub expanded_width: f64,
    pub expanded_height: f64,
    pub media_width: f64,
    pub media_height: f64,
}
impl Default for SizePreferences {
    fn default() -> Self {
        Self {
            contracted_width: 126.0,
            contracted_height: 32.0,
            expanded_width: 360.0,
            expanded_height: 160.0,
            media_width: 640.0,
            media_height: 216.0,
        }
    }
}
impl SizePreferences {
    pub fn normalized(self) -> Self {
        let defaults = Self::default();
        let bounded = |value: f64, low: f64, high: f64, fallback: f64| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        Self {
            contracted_width: bounded(
                self.contracted_width,
                100.0,
                320.0,
                defaults.contracted_width,
            ),
            contracted_height: bounded(
                self.contracted_height,
                28.0,
                64.0,
                defaults.contracted_height,
            ),
            expanded_width: bounded(self.expanded_width, 320.0, 900.0, defaults.expanded_width),
            expanded_height: bounded(self.expanded_height, 160.0, 360.0, defaults.expanded_height),
            media_width: bounded(self.media_width, 480.0, 1000.0, defaults.media_width),
            media_height: bounded(self.media_height, 216.0, 400.0, defaults.media_height),
        }
    }
    pub fn canvas_size(self) -> (f64, f64) {
        let s = self.normalized();
        let width = s
            .contracted_width
            .max(220.)
            .max(s.expanded_width)
            .max(s.media_width);
        let height = s
            .contracted_height
            .max(48.)
            .max(s.expanded_height)
            .max(240.)
            .max(400.)
            .max(s.media_height);
        (
            (width + (width - s.contracted_width).max(0.) * OVERSHOOT_ALLOWANCE + 40.)
                .ceil()
                .max(OVERLAY_WIDTH),
            (height + (height - s.contracted_height).max(0.) * OVERSHOOT_ALLOWANCE + 24.)
                .ceil()
                .max(OVERLAY_HEIGHT),
        )
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IslandState {
    #[default]
    Idle,
    Compact,
    Expanded,
    Volume,
    Media,
    Notification,
    Timer,
    Battery,
    Plugin,
    AppLauncher,
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub width: f64,
    pub height: f64,
    pub radius: f64,
}

impl IslandState {
    pub const fn is_expanded(self) -> bool {
        matches!(
            self,
            Self::Expanded
                | Self::Media
                | Self::Volume
                | Self::Notification
                | Self::Timer
                | Self::Battery
                | Self::Plugin
                | Self::AppLauncher
        )
    }

    pub const fn geometry(self) -> Geometry {
        match self {
            Self::AppLauncher => Geometry {
                width: 560.,
                height: 400.,
                radius: EXPANDED_RADIUS,
            },
            Self::Idle => Geometry {
                width: 126.0,
                height: 32.0,
                radius: 16.0,
            },
            Self::Compact => Geometry {
                width: 220.0,
                height: 48.0,
                radius: 24.0,
            },
            Self::Media => Geometry {
                width: 640.0,
                height: 216.0,
                radius: EXPANDED_RADIUS,
            },
            Self::Plugin => Geometry {
                width: 480.0,
                height: 240.0,
                radius: EXPANDED_RADIUS,
            },
            Self::Expanded | Self::Volume | Self::Notification | Self::Timer | Self::Battery => {
                Geometry {
                    width: 360.0,
                    height: 160.0,
                    radius: EXPANDED_RADIUS,
                }
            }
        }
    }

    /// Resolve a destination without coupling the state model to preferences or
    /// native display APIs. `available_width` is the monitor width in logical px.
    #[cfg(test)]
    pub fn geometry_for(
        self,
        form: FormFactor,
        notch_corner_radius: f64,
        notch_contracted_radius: f64,
        available_width: f64,
    ) -> Geometry {
        self.geometry_for_sizes(
            form,
            notch_corner_radius,
            notch_contracted_radius,
            available_width,
            &SizePreferences::default(),
        )
    }

    pub fn geometry_for_sizes(
        self,
        form: FormFactor,
        notch_corner_radius: f64,
        notch_contracted_radius: f64,
        available_width: f64,
        sizes: &SizePreferences,
    ) -> Geometry {
        let sizes = sizes.normalized();
        let (width, height) = match self {
            Self::AppLauncher => (
                sizes.expanded_width.max(560.),
                sizes.expanded_height.max(400.),
            ),
            Self::Idle => (sizes.contracted_width, sizes.contracted_height),
            Self::Compact => (
                sizes.contracted_width.max(220.),
                sizes.contracted_height.max(48.),
            ),
            Self::Media => (sizes.media_width, sizes.media_height),
            Self::Plugin => (
                sizes.expanded_width.max(480.),
                sizes.expanded_height.max(240.),
            ),
            _ => (sizes.expanded_width, sizes.expanded_height),
        };
        let mut geometry = Geometry {
            width,
            height,
            radius: if self.is_expanded() {
                EXPANDED_RADIUS
            } else {
                height / 2.
            },
        };
        if form == FormFactor::Notch {
            let requested = if self.is_expanded() {
                notch_corner_radius
            } else {
                notch_contracted_radius
            };
            geometry.radius = notch_radius(requested, geometry.height);
        }
        let canvas_width = sizes.canvas_size().0;
        let available = if available_width.is_finite() && available_width > 0. {
            available_width.min(canvas_width)
        } else {
            canvas_width
        };
        let shoulders = ShapeContour::new(geometry.width, geometry.height, geometry.radius, form)
            .shoulder()
            * 2.;
        let budget = (available - shoulders - DISPLAY_MARGIN).max(1.);
        // Reserve overshoot of the travel from the smallest capsule. On unusually
        // narrow monitors keep controls usable at 360px, then fall back to 320px.
        let minimum = if budget >= 360. {
            360.
        } else {
            budget.min(320.)
        };
        if !self.is_expanded() {
            geometry.width = geometry.width.min(budget);
            return geometry;
        }
        let spring_safe =
            (budget + sizes.contracted_width * OVERSHOOT_ALLOWANCE) / (1. + OVERSHOOT_ALLOWANCE);
        geometry.width = geometry.width.min(spring_safe.max(minimum)).min(budget);
        geometry
    }
}

/// A notch radius must remain finite and fit its contour at every state size.
fn notch_radius(requested: f64, height: f64) -> f64 {
    let radius = if requested.is_finite() {
        requested
    } else {
        NOTCH_EXPANDED_RADIUS
    };
    radius.clamp(0., 24.).min(height / 2.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customized_sizes_preserve_text_scale_independent_destinations_and_canvas_room() {
        let sizes = SizePreferences {
            contracted_width: 260.,
            contracted_height: 60.,
            expanded_width: 850.,
            expanded_height: 350.,
            media_width: 1000.,
            media_height: 400.,
        };
        let idle = IslandState::Idle.geometry_for_sizes(FormFactor::Notch, 8., 8., 1920., &sizes);
        assert_eq!((idle.width, idle.height), (260., 60.));
        let compact =
            IslandState::Compact.geometry_for_sizes(FormFactor::Island, 8., 8., 1920., &sizes);
        assert_eq!(
            (compact.width, compact.height, compact.radius),
            (260., 60., 30.)
        );
        let expanded =
            IslandState::Expanded.geometry_for_sizes(FormFactor::Notch, 8., 8., 1920., &sizes);
        assert_eq!((expanded.width, expanded.height), (850., 350.));
        let media = IslandState::Media.geometry_for_sizes(FormFactor::Notch, 8., 8., 1920., &sizes);
        assert_eq!((media.width, media.height), (1000., 400.));
        let canvas = sizes.canvas_size();
        assert!(canvas.0 >= 1000. + (1000. - 260.) * 0.2 + 40.);
        assert!(canvas.1 >= 400. + (400. - 60.) * 0.2 + 24.);
        assert_eq!(SizePreferences::default().canvas_size(), (800., 498.));
        let launcher = IslandState::AppLauncher.geometry_for_sizes(
            FormFactor::Notch,
            8.,
            8.,
            1920.,
            &SizePreferences::default(),
        );
        assert_eq!((launcher.width, launcher.height), (560., 400.));
        assert!(SizePreferences::default().canvas_size().1 > launcher.height);
        let narrow = IslandState::Media.geometry_for_sizes(FormFactor::Notch, 8., 8., 640., &sizes);
        assert!(narrow.width + 16. < 640.);
    }

    #[test]
    fn compact_is_capsule_and_island_panels_keep_twelve_pixel_corners() {
        for state in [IslandState::Idle, IslandState::Compact] {
            let geometry = state.geometry();
            assert_eq!(geometry.radius, geometry.height / 2.);
            assert!(!state.is_expanded());
        }
        for state in [
            IslandState::Expanded,
            IslandState::Media,
            IslandState::Volume,
            IslandState::Notification,
            IslandState::Timer,
            IslandState::Battery,
            IslandState::Plugin,
        ] {
            let geometry = state.geometry();
            assert_eq!(geometry.radius, 12.);
            assert!(state.is_expanded());
            let dimensions = match state {
                IslandState::Media => (640., 216.),
                IslandState::Plugin => (480., 240.),
                _ => (360., 160.),
            };
            assert_eq!((geometry.width, geometry.height), dimensions);
        }
    }

    #[test]
    fn plugin_pages_have_readable_minimums_follow_larger_preferences_and_fit_monitors() {
        let standard = IslandState::Plugin.geometry_for(FormFactor::Notch, 8., 8., 1920.);
        assert_eq!((standard.width, standard.height), (480., 240.));
        let sizes = SizePreferences {
            expanded_width: 700.,
            expanded_height: 320.,
            ..SizePreferences::default()
        };
        let custom =
            IslandState::Plugin.geometry_for_sizes(FormFactor::Notch, 8., 8., 1920., &sizes);
        assert_eq!((custom.width, custom.height), (700., 320.));
        let narrow = IslandState::Plugin.geometry_for(FormFactor::Notch, 8., 8., 400.);
        assert!(narrow.width <= 360. && narrow.width + 16. + DISPLAY_MARGIN <= 400.);
        let canvas = SizePreferences::default().canvas_size();
        assert!(canvas.1 >= standard.height + (standard.height - 32.) * OVERSHOOT_ALLOWANCE + 24.);
    }
    #[test]
    fn notch_corner_preference_only_changes_expanded_notch_destinations() {
        for state in [
            IslandState::Expanded,
            IslandState::Media,
            IslandState::Volume,
            IslandState::Notification,
            IslandState::Timer,
            IslandState::Battery,
            IslandState::Plugin,
        ] {
            for (requested, expected) in [(-2., 0.), (8., 8.), (18., 18.), (40., 24.)] {
                assert_eq!(
                    state
                        .geometry_for(FormFactor::Notch, requested, 8., 1920.)
                        .radius,
                    expected
                );
            }
            assert_eq!(
                state
                    .geometry_for(FormFactor::Notch, f64::NAN, 8., 1920.)
                    .radius,
                NOTCH_EXPANDED_RADIUS
            );
            assert_eq!(
                state.geometry_for(FormFactor::Island, 8., 8., 1920.).radius,
                12.
            );
        }
        for state in [IslandState::Idle, IslandState::Compact] {
            assert_eq!(
                state.geometry_for(FormFactor::Notch, 8., 8., 1920.).radius,
                8.
            );
        }
    }

    #[test]
    fn wide_media_adapts_to_the_monitor_and_retains_usable_narrow_fallbacks() {
        for (available, expected) in [
            (1920., 640.),
            (800., 640.),
            (720., (680. + 25.2) / 1.2),
            (640., (600. + 25.2) / 1.2),
            (400., 360.),
            (360., 320.),
            (280., 240.),
        ] {
            let geometry = IslandState::Media.geometry_for(FormFactor::Notch, 8., 8., available);
            assert!((geometry.width - expected).abs() < 1e-9);
            assert_eq!(geometry.height, 216.);
            assert!(geometry.width + 16. + DISPLAY_MARGIN <= available.min(OVERLAY_WIDTH));
        }
    }

    #[test]
    fn lively_media_overshoot_fits_canvas_and_normal_logical_monitor_widths() {
        for form in [FormFactor::Island, FormFactor::Notch] {
            for available in [640_f64, 720., 800., 1920.] {
                let target = IslandState::Media.geometry_for(form, 8., 8., available);
                let idle = IslandState::Idle.geometry();
                let peak_width = target.width + (target.width - idle.width) * OVERSHOOT_ALLOWANCE;
                let peak_height =
                    target.height + (target.height - idle.height) * OVERSHOOT_ALLOWANCE;
                let peak = ShapeContour::new(peak_width, peak_height, target.radius, form);
                assert!(peak.total_width() + DISPLAY_MARGIN <= available.min(OVERLAY_WIDTH) + 1e-9);
                assert!(peak.height < OVERLAY_HEIGHT);
            }
        }
    }

    #[test]
    fn invalid_display_dimensions_have_finite_safe_destinations() {
        for available in [f64::NAN, f64::INFINITY, -1., 0., 1., 24.] {
            let target =
                IslandState::Media.geometry_for(FormFactor::Notch, f64::INFINITY, 8., available);
            assert!(target.width.is_finite() && target.width > 0.);
            assert!(target.width <= 640.);
            assert_eq!(target.radius, NOTCH_EXPANDED_RADIUS);
        }
    }

    #[test]
    fn contracted_notch_radius_is_independent_and_bounded_by_its_height() {
        for state in [IslandState::Idle, IslandState::Compact] {
            for (requested, expected) in [
                (-2., 0.),
                (0., 0.),
                (5.5, 5.5),
                (8., 8.),
                (24., state.geometry().height / 2.),
                (99., state.geometry().height / 2.),
                (f64::NAN, 8.),
                (f64::INFINITY, 8.),
            ] {
                let notch = state.geometry_for(FormFactor::Notch, 19., requested, 1920.);
                assert_eq!(notch.radius, expected);
                let island = state.geometry_for(FormFactor::Island, 19., requested, 1920.);
                assert_eq!(island.radius, island.height / 2.);
                assert_eq!((notch.width, notch.height), (island.width, island.height));
            }
        }
        assert_eq!(
            IslandState::Expanded
                .geometry_for(FormFactor::Notch, 19., 3., 1920.)
                .radius,
            19.
        );
    }
}
