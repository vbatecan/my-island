//! Shared silhouette geometry for drawing, optics, native regions and gestures.
use std::f64::consts::{FRAC_PI_2, PI};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormFactor {
    #[default]
    Notch,
    Island,
}
impl FormFactor {
    pub const fn index(self) -> i32 {
        match self {
            Self::Notch => 0,
            Self::Island => 1,
        }
    }
    pub fn parse(value: &str) -> Self {
        if value == "island" {
            Self::Island
        } else {
            Self::Notch
        }
    }
    pub const fn key(self) -> &'static str {
        match self {
            Self::Notch => "notch",
            Self::Island => "island",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ShapeContour {
    pub width: f64,
    pub height: f64,
    pub radius: f64,
    pub form: FormFactor,
}
#[derive(Clone, Copy)]
enum Edge {
    Line([f64; 2], [f64; 2]),
    Arc([f64; 2], f64, f64, f64),
}
impl ShapeContour {
    pub fn new(width: f64, height: f64, radius: f64, form: FormFactor) -> Self {
        let width = if width.is_finite() { width.max(1.) } else { 1. };
        let height = if height.is_finite() {
            height.max(1.)
        } else {
            1.
        };
        Self {
            width,
            height,
            radius: if radius.is_finite() {
                radius.clamp(0., width.min(height) / 2.)
            } else {
                0.
            },
            form,
        }
    }
    pub fn shoulder(self) -> f64 {
        if self.form == FormFactor::Notch {
            8f64.min(self.height / 2.)
        } else {
            0.
        }
    }
    pub fn total_width(self) -> f64 {
        self.width + 2. * self.shoulder()
    }
    fn edges(self) -> [Edge; 8] {
        let s = self.shoulder();
        let w = self.total_width();
        let h = self.height;
        let r = self.radius;
        if self.form == FormFactor::Notch {
            [
                Edge::Line([0., 0.], [w, 0.]),
                Edge::Arc([w, s], s, -FRAC_PI_2, -PI),
                Edge::Line([w - s, s], [w - s, h - r]),
                Edge::Arc([w - s - r, h - r], r, 0., FRAC_PI_2),
                Edge::Line([w - s - r, h], [s + r, h]),
                Edge::Arc([s + r, h - r], r, FRAC_PI_2, PI),
                Edge::Line([s, h - r], [s, s]),
                Edge::Arc([0., s], s, 0., -FRAC_PI_2),
            ]
        } else {
            [
                Edge::Line([r, 0.], [w - r, 0.]),
                Edge::Arc([w - r, r], r, -FRAC_PI_2, 0.),
                Edge::Line([w, r], [w, h - r]),
                Edge::Arc([w - r, h - r], r, 0., FRAC_PI_2),
                Edge::Line([w - r, h], [r, h]),
                Edge::Arc([r, h - r], r, FRAC_PI_2, PI),
                Edge::Line([0., h - r], [0., r]),
                Edge::Arc([r, r], r, PI, PI + FRAC_PI_2),
            ]
        }
    }
    pub fn contains(self, x: f64, y: f64) -> bool {
        let s = self.shoulder();
        let w = self.total_width();
        let h = self.height;
        let r = self.radius;
        if !x.is_finite() || !y.is_finite() || x < 0. || y < 0. || x > w || y > h {
            return false;
        }
        if self.form == FormFactor::Island {
            let dx = (r - x).max(x - (w - r)).max(0.);
            let dy = (r - y).max(y - (h - r)).max(0.);
            return dx * dx + dy * dy <= r * r;
        }
        if x < s || x > w - s {
            let dx = if x < s { x } else { w - x };
            return y <= s && dx * dx + (y - s) * (y - s) >= s * s;
        }
        let dx = (s + r - x).max(x - (w - s - r)).max(0.);
        let dy = (y - (h - r)).max(0.);
        dx * dx + dy * dy <= r * r
    }
    pub fn distance_normal(self, x: f64, y: f64) -> (f64, [f64; 2]) {
        let p = [x, y];
        let mut best = f64::INFINITY;
        let mut nearest = [x, 0.];
        for edge in self.edges() {
            let q = match edge {
                Edge::Line(a, b) => {
                    let v = [b[0] - a[0], b[1] - a[1]];
                    let length = v[0] * v[0] + v[1] * v[1];
                    let t = if length > 0. {
                        ((x - a[0]) * v[0] + (y - a[1]) * v[1]) / length
                    } else {
                        0.
                    }
                    .clamp(0., 1.);
                    [a[0] + t * v[0], a[1] + t * v[1]]
                }
                Edge::Arc(c, r, a, b) => {
                    let v = [x - c[0], y - c[1]];
                    let middle = (a + b) / 2.;
                    let length = v[0].hypot(v[1]);
                    if length > 1e-12
                        && v[0] * middle.cos() >= -1e-12
                        && v[1] * middle.sin() >= -1e-12
                    {
                        [c[0] + r * v[0] / length, c[1] + r * v[1] / length]
                    } else {
                        let start = [c[0] + r * a.cos(), c[1] + r * a.sin()];
                        let end = [c[0] + r * b.cos(), c[1] + r * b.sin()];
                        if (x - start[0]).hypot(y - start[1]) < (x - end[0]).hypot(y - end[1]) {
                            start
                        } else {
                            end
                        }
                    }
                }
            };
            let d = (p[0] - q[0]).hypot(p[1] - q[1]);
            if d < best {
                best = d;
                nearest = q;
            }
        }
        let sign = if self.contains(x, y) { -1. } else { 1. };
        (
            sign * best,
            if best > 1e-9 {
                [
                    sign * (x - nearest[0]) / best,
                    sign * (y - nearest[1]) / best,
                ]
            } else {
                [0., -1.]
            },
        )
    }
    /// Local physical-pixel contour, with at most a quarter-pixel chord error.
    pub fn flattened(self, scale: f64) -> Vec<[f64; 2]> {
        let scale = scale.max(0.01);
        let mut points = Vec::with_capacity(80);
        for edge in self.edges() {
            match edge {
                Edge::Line(a, b) => {
                    if points.is_empty() {
                        points.push([a[0] * scale, a[1] * scale]);
                    }
                    points.push([b[0] * scale, b[1] * scale]);
                }
                Edge::Arc(c, r, a, b) => {
                    let radius = r * scale;
                    let step = if radius > 0.25 {
                        2. * (1. - 0.25 / radius).acos()
                    } else {
                        FRAC_PI_2
                    };
                    let count = ((b - a).abs() / step).ceil().max(1.) as usize;
                    for i in 1..=count {
                        let t = a + (b - a) * i as f64 / count as f64;
                        points.push([(c[0] + r * t.cos()) * scale, (c[1] + r * t.sin()) * scale]);
                    }
                }
            }
        }
        points
    }
    pub fn svg_commands(self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        for edge in self.edges() {
            match edge {
                Edge::Line(a, b) => {
                    if out.is_empty() {
                        let _ = write!(out, "M {} {} ", a[0], a[1]);
                    }
                    let _ = write!(out, "L {} {} ", b[0], b[1]);
                }
                Edge::Arc(c, r, a, b) => {
                    let end = [c[0] + r * b.cos(), c[1] + r * b.sin()];
                    if r > 0. {
                        let _ = write!(
                            out,
                            "A {r} {r} 0 0 {} {} {} ",
                            if b > a { 1 } else { 0 },
                            end[0],
                            end[1]
                        );
                    } else {
                        let _ = write!(out, "L {} {} ", end[0], end[1]);
                    }
                }
            }
        }
        out.push('Z');
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notch_shoulders_and_cutouts() {
        let c = ShapeContour::new(126., 32., 16., FormFactor::Notch);
        assert_eq!(c.total_width(), 142.);
        assert!(c.contains(71., 1.));
        assert!(c.contains(4., 0.5));
        assert!(!c.contains(0.5, 4.));
        assert!(!c.contains(8., 32.));
    }
    #[test]
    fn distances_have_no_join_seam() {
        let c = ShapeContour::new(360., 160., 12., FormFactor::Notch);
        assert!(c.distance_normal(8., 4.).0 < -0.5);
        assert!(c.distance_normal(180., 80.).0 < -70.);
        for x in 0..376 {
            let (d, n) = c.distance_normal(x as f64, 5.);
            let (other, _) = c.distance_normal(376. - x as f64, 5.);
            assert!((d - other).abs() < 1e-8);
            assert!((n[0] * n[0] + n[1] * n[1] - 1.).abs() < 1e-8);
        }
    }
    #[test]
    fn island_retains_capsule() {
        let c = ShapeContour::new(126., 32., 16., FormFactor::Island);
        assert!(!c.contains(0., 0.));
        assert!(c.contains(63., 16.));
        assert_eq!(c.shoulder(), 0.);
    }
    #[test]
    fn flattened_contour_covers_dpi_and_overshoot() {
        for scale in [1., 1.25, 1.5, 2., 3.] {
            let c = ShapeContour::new(418., 200., 12., FormFactor::Notch);
            for p in c.flattened(scale) {
                assert!(p[0] >= -1e-6 && p[0] <= 434. * scale + 1e-6);
                assert!(p[1] >= -1e-6 && p[1] <= 200. * scale + 1e-6);
                let (d, _) = c.distance_normal(p[0] / scale, p[1] / scale);
                assert!(d.abs() < 1e-6);
            }
        }
    }
}
