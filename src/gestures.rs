//! Normalized wheel accumulation and visible-silhouette hit testing.
#[derive(Default)]
pub struct WheelVolume {
    remainder: f64,
}
impl WheelVolume {
    pub fn advance(&mut self, notches: f64, step: u32) -> i32 {
        if !notches.is_finite() {
            return 0;
        }
        self.remainder += notches.clamp(-10., 10.);
        let whole = self.remainder.trunc();
        self.remainder -= whole;
        whole as i32 * step.clamp(1, 10) as i32
    }
    pub fn reset(&mut self) {
        self.remainder = 0.;
    }
}
#[cfg(test)]
pub fn contains(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
    form: crate::shape::FormFactor,
) -> bool {
    contains_in_canvas(
        x,
        y,
        width,
        height,
        radius,
        form,
        crate::state::OVERLAY_WIDTH,
    )
}

pub fn contains_in_canvas(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
    form: crate::shape::FormFactor,
    canvas_width: f64,
) -> bool {
    if [x, y, width, height, radius, canvas_width]
        .iter()
        .any(|v| !v.is_finite())
        || width <= 0.
        || height <= 0.
        || canvas_width <= 0.
    {
        return false;
    }
    let contour = crate::shape::ShapeContour::new(width, height, radius, form);
    contour.contains(x - (canvas_width - contour.total_width()) / 2., y)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_canvas_keeps_the_contour_and_pointer_centered() {
        use crate::shape::FormFactor;
        for canvas in [800., 1200., 1600.] {
            assert!(contains_in_canvas(
                canvas / 2.,
                80.,
                960.,
                320.,
                8.,
                FormFactor::Notch,
                canvas
            ));
            assert!(!contains_in_canvas(
                canvas / 2. - 485.,
                80.,
                960.,
                320.,
                8.,
                FormFactor::Notch,
                canvas
            ));
        }
        assert!(!contains_in_canvas(
            600.,
            80.,
            960.,
            320.,
            8.,
            FormFactor::Notch,
            f64::NAN
        ));
    }
    fn contains(x: f64, y: f64, w: f64, h: f64, r: f64) -> bool {
        super::contains(x, y, w, h, r, crate::shape::FormFactor::Island)
    }
    #[test]
    fn accumulates_touchpad_and_reverses() {
        let mut w = WheelVolume::default();
        assert_eq!(w.advance(0.4, 2), 0);
        assert_eq!(w.advance(0.7, 2), 2);
        assert_eq!(w.advance(-1.2, 2), -2);
        assert_eq!(w.advance(f64::NAN, 2), 0);
        w.reset();
        assert_eq!(w.advance(1., 5), 5);
    }
    #[test]
    fn excludes_transparent_capsule_corners() {
        let center = crate::state::OVERLAY_WIDTH / 2.;
        assert!(contains(center, 16., 126., 32., 16.));
        assert!(!contains(center - 63., 0., 126., 32., 16.));
        assert!(!contains(0., 0., 126., 32., 16.));
        let expanded = crate::state::IslandState::Expanded.geometry();
        assert!(contains(
            center - expanded.width / 2. + 20.,
            10.,
            expanded.width,
            expanded.height,
            expanded.radius
        ));
    }
    #[test]
    fn rejects_invalid_geometry_without_panicking() {
        let center = crate::state::OVERLAY_WIDTH / 2.;
        assert!(!contains(f64::NAN, 16., 126., 32., 16.));
        assert!(!contains(center, 16., -126., 32., 16.));
        assert!(!contains(center, 16., 126., 0., 16.));
        assert!(!contains(center, 16., 126., 32., f64::INFINITY));
    }

    #[test]
    fn wide_media_hit_bounds_remain_aligned_at_fractional_display_scales() {
        use crate::{
            shape::{FormFactor, ShapeContour},
            state::{IslandState, OVERLAY_WIDTH},
        };
        for form in [FormFactor::Island, FormFactor::Notch] {
            for radius in [0., 8., 24.] {
                let geometry = IslandState::Media.geometry_for(form, radius, 8., 1920.);
                let contour =
                    ShapeContour::new(geometry.width, geometry.height, geometry.radius, form);
                let left = (OVERLAY_WIDTH - contour.total_width()) / 2.;
                for dpi in [1., 1.25, 1.5, 1.75, 2., 2.5] {
                    for zoom in [0.75, 1., 1.25, 1.5] {
                        let scale = dpi * zoom;
                        // Native coordinates are physical pixels; Slint events are
                        // logical pixels. Stay two pixels off edges to allow rounding.
                        let hit = |x: f64, y: f64| {
                            super::contains(
                                (x * scale).round() / scale,
                                (y * scale).round() / scale,
                                geometry.width,
                                geometry.height,
                                geometry.radius,
                                form,
                            )
                        };
                        assert!(hit(OVERLAY_WIDTH / 2., geometry.height / 2.));
                        assert!(!hit(left - 2., geometry.height / 2.));
                        assert!(!hit(
                            left + contour.total_width() + 2.,
                            geometry.height / 2.
                        ));
                        assert!(!hit(OVERLAY_WIDTH / 2., geometry.height + 2.));
                        assert!(hit(
                            left + contour.shoulder() + geometry.radius + 2.,
                            geometry.height - 2.
                        ));
                        if form == FormFactor::Notch {
                            assert!(!hit(left + 2., geometry.height / 2.));
                        }
                        for point in contour.flattened(scale) {
                            assert!(
                                point[0] >= -1e-9
                                    && point[0] <= contour.total_width() * scale + 1e-9
                            );
                            assert!(
                                point[1] >= -1e-9 && point[1] <= geometry.height * scale + 1e-9
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn bounds_extreme_wheel_input_and_discards_reset_remainder() {
        let mut wheel = WheelVolume::default();
        assert_eq!(wheel.advance(100000., 100), 100);
        assert_eq!(wheel.advance(-100000., 0), -10);
        assert_eq!(wheel.advance(0.9, 2), 0);
        wheel.reset();
        assert_eq!(wheel.advance(0.2, 2), 0);
        assert_eq!(wheel.advance(f64::INFINITY, 2), 0);
        assert_eq!(wheel.advance(0.8, 2), 2);
    }
}
