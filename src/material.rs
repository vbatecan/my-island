//! A small CPU lens compositor for the optional captured-desktop glass material.
//! No time uniform, animation loop, screen capture, or global state lives here.
//! The rounded-box distance gradient defines a beveled normal; Snell refraction
//! displaces backdrop samples and Schlick Fresnel supplies restrained rim reflection.
use crate::shape::{FormFactor, ShapeContour};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

#[derive(Clone, Debug)]
pub struct Backdrop {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
#[derive(Default)]
pub struct Glass {
    pub backdrop: Option<Backdrop>,
    revision: u64,
    source_size: (u32, u32),
    unfiltered: Option<Backdrop>,
    pub liquid_backdrop: Option<Backdrop>,
    pub liquid_revision: u64,
    pub palette: crate::liquid::Palette,
    cache: Option<(GlassCacheKey, Image)>,
}
#[derive(Clone, PartialEq, Eq)]
struct GlassCacheKey {
    width: u32,
    height: u32,
    radius_subpixels: u32,
    scale_bits: u32,
    capture_scale_bits: u32,
    canvas_width_bits: u32,
    backdrop_revision: u64,
    form: FormFactor,
    profile: OpticalProfile,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum OpticalProfile {
    Frosted,
    Glass,
}
impl Glass {
    pub fn set_liquid_backdrop(&mut self, frame: Backdrop) -> bool {
        if frame.width == 0
            || frame.height == 0
            || frame.pixels.len() != frame.width as usize * frame.height as usize * 4
        {
            return false;
        }
        self.liquid_backdrop = Some(frame);
        self.liquid_revision = self.liquid_revision.wrapping_add(1);
        true
    }
    pub fn set_backdrop(&mut self, frame: Backdrop) -> bool {
        let Some(mut reduced) = reduced_backdrop(&frame) else {
            return false;
        };
        let source_size = (frame.width, frame.height);
        // Compare before the two blur passes. Unchanged frames perform no blur,
        // optical rasterization or image upload. Keep only bounded raw samples.
        if self.source_size == source_size
            && self.unfiltered.as_ref().is_some_and(|old| {
                old.width == reduced.width
                    && old.height == reduced.height
                    && old.pixels == reduced.pixels
            })
        {
            return false;
        }
        self.unfiltered = Some(reduced.clone());
        let radius = (reduced.width / 140).clamp(1, 12) as usize;
        blur(&mut reduced, radius);
        blur(&mut reduced, radius);
        if self.source_size == source_size
            && self.backdrop.as_ref().is_some_and(|old| {
                old.width == reduced.width
                    && old.height == reduced.height
                    && old.pixels == reduced.pixels
            })
        {
            return false;
        }
        self.source_size = source_size;
        self.backdrop = Some(reduced);
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn clear(&mut self) {
        self.backdrop = None;
        self.unfiltered = None;
        self.liquid_backdrop = None;
        self.source_size = (0, 0);
        self.cache = None;
    }
    #[cfg(test)]
    pub fn image(
        &mut self,
        width: f64,
        height: f64,
        radius: f64,
        scale: f32,
        form: FormFactor,
    ) -> Image {
        self.image_in_canvas(
            width,
            height,
            radius,
            scale,
            form,
            crate::state::OVERLAY_WIDTH,
        )
    }

    pub fn image_in_canvas(
        &mut self,
        width: f64,
        height: f64,
        radius: f64,
        scale: f32,
        form: FormFactor,
        canvas_width: f64,
    ) -> Image {
        self.compose(
            width,
            height,
            radius,
            scale,
            form,
            canvas_width,
            OpticalProfile::Glass,
        )
    }
    pub fn frosted_image_in_canvas(
        &mut self,
        width: f64,
        height: f64,
        radius: f64,
        scale: f32,
        form: FormFactor,
        canvas_width: f64,
    ) -> Image {
        self.compose(
            width,
            height,
            radius,
            scale,
            form,
            canvas_width,
            OpticalProfile::Frosted,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn compose(
        &mut self,
        width: f64,
        height: f64,
        radius: f64,
        scale: f32,
        form: FormFactor,
        canvas_width: f64,
        profile: OpticalProfile,
    ) -> Image {
        // User zoom can reduce the effective pixel scale below one. Preserve the
        // exact capture-to-canvas mapping instead of sampling a larger phantom crop.
        let capture_scale = if scale.is_finite() {
            scale.clamp(0.5, 8.)
        } else {
            1.
        };
        let canvas_width = if canvas_width.is_finite() {
            canvas_width.clamp(crate::state::OVERLAY_WIDTH, 4096.) as f32
        } else {
            crate::state::OVERLAY_WIDTH as f32
        };
        let contour = ShapeContour::new(width, height, radius, form);
        let scale = optical_scale(contour.total_width(), contour.height, capture_scale);
        let w = (contour.total_width() as f32 * scale).ceil().max(1.) as u32;
        let h = (height as f32 * scale).ceil().max(1.) as u32;
        let r = radius as f32 * scale;
        let key = GlassCacheKey {
            width: w,
            height: h,
            radius_subpixels: (r * 64.).round() as u32,
            scale_bits: scale.to_bits(),
            capture_scale_bits: capture_scale.to_bits(),
            canvas_width_bits: canvas_width.to_bits(),
            backdrop_revision: self.revision,
            form,
            profile,
        };
        if let Some((old, image)) = &self.cache
            && *old == key
        {
            return image.clone();
        }
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        let left = (canvas_width * scale - w as f32) * 0.5;
        let bezel = (12. * scale).min(h as f32 * 0.46);
        let output = buffer.make_mut_slice();
        for y in 0..h {
            for x in 0..w {
                let logical_x = (x as f64 + 0.5) / scale as f64;
                let logical_y = (y as f64 + 0.5) / scale as f64;
                let safe_inset = contour.shoulder() + contour.radius + 12.;
                let (distance, normal) = if logical_x >= safe_inset
                    && logical_x <= contour.total_width() - safe_inset
                    && logical_y >= contour.radius + 12.
                    && logical_y <= contour.height - contour.radius - 12.
                {
                    (-12., [0., -1.])
                } else {
                    lens_distance_normal(contour, logical_x, logical_y)
                };
                let d = distance as f32 * scale;
                let n = [normal[0] as f32, normal[1] as f32];
                let coverage = (0.5 - d).clamp(0., 1.);
                if coverage <= 0. {
                    continue;
                }
                let (dx, dy, reflection) = if profile == OpticalProfile::Frosted {
                    (0., 0., 0.)
                } else if -d >= bezel {
                    // A flat center has normal(0,0,1), no displacement, and
                    // constant Fresnel/light terms. Only the optical rim needs
                    // Snell square roots and specular powers per pixel.
                    (
                        0.,
                        0.,
                        (((1.45f32 - 1.) / (1.45 + 1.)).powi(2) * 0.55 + 0.61f32.powi(18) * 0.16)
                            * coverage,
                    )
                } else {
                    let depth = (-d).max(0.);
                    let t = (depth / bezel).clamp(0.0001, 1.);
                    let arc = (1. - (1. - t).powi(2)).sqrt();
                    let slope = ((8. * scale) * (1. - t) / (bezel * arc)).min(8.);
                    let len = (1. + slope * slope).sqrt();
                    let normal = [n[0] * slope / len, n[1] * slope / len, 1. / len];
                    let ray = refract(normal, 1. / 1.45);
                    let travel = 12. * scale;
                    let dx = ray[0] / (-ray[2]).max(0.05) * travel;
                    let dy = ray[1] / (-ray[2]).max(0.05) * travel;
                    let f0 = ((1.45f32 - 1.) / (1.45 + 1.)).powi(2);
                    let fresnel = f0 + (1. - f0) * (1. - normal[2]).powi(5);
                    let light = (-0.45 * normal[0] - 0.65 * normal[1] + 0.61 * normal[2]).max(0.);
                    let reflection = (fresnel * 0.55 + light.powi(18) * 0.16) * coverage;
                    (dx, dy, reflection)
                };
                let backdrop = &self.backdrop;
                let pixel = if let Some(frame) = backdrop {
                    // Map optical pixels through logical canvas space into the
                    // original capture, then into the reduced backdrop. DPI and
                    // user zoom must not shift the sampled desktop crop.
                    let source_x = (left + x as f32 + 0.5 + dx) / scale * capture_scale;
                    let source_y = (y as f32 + 0.5 + dy) / scale * capture_scale;
                    let mut c = sample(
                        frame,
                        source_x * frame.width as f32 / self.source_size.0.max(1) as f32 - 0.5,
                        source_y * frame.height as f32 / self.source_size.1.max(1) as f32 - 0.5,
                    );
                    for value in &mut c {
                        *value = (*value * (1. - reflection) + 255. * reflection).clamp(0., 255.);
                    }
                    Rgba8Pixel {
                        r: c[0] as u8,
                        g: c[1] as u8,
                        b: c[2] as u8,
                        a: (255. * coverage) as u8,
                    }
                } else {
                    Rgba8Pixel {
                        r: 255,
                        g: 255,
                        b: 255,
                        a: (255. * reflection) as u8,
                    }
                };
                output[(y * w + x) as usize] = pixel;
            }
        }
        let image = Image::from_rgba8(buffer);
        self.cache = Some((key, image.clone()));
        image
    }
}
// Optical content is a blurred image beneath vector-clipped UI, not text.
// Bound its work independently of monitor DPI and launcher size.
const MAX_OPTICAL_PIXELS: f64 = 128_000.;
fn optical_scale(width: f64, height: f64, requested: f32) -> f32 {
    let limit = (MAX_OPTICAL_PIXELS / (width.max(1.) * height.max(1.))).sqrt() as f32;
    requested.min(1.25).min(limit).max(0.01)
}
fn reduced_backdrop(frame: &Backdrop) -> Option<Backdrop> {
    let size = (frame.width as usize)
        .checked_mul(frame.height as usize)?
        .checked_mul(4)?;
    if frame.width == 0 || frame.height == 0 || frame.pixels.len() != size {
        return None;
    }
    let factor = (384. / frame.width as f32)
        .min(240. / frame.height as f32)
        .min(1.);
    let width = (frame.width as f32 * factor).floor().max(1.) as u32;
    let height = (frame.height as f32 * factor).floor().max(1.) as u32;
    if width == frame.width && height == frame.height {
        return Some(frame.clone());
    }
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let sx = (x as f32 + 0.5) * frame.width as f32 / width as f32 - 0.5;
            let sy = (y as f32 + 0.5) * frame.height as f32 / height as f32 - 0.5;
            let c = sample(frame, sx, sy);
            let alpha = frame.pixels[((sy.round().clamp(0., (frame.height - 1) as f32) as u32
                * frame.width
                + sx.round().clamp(0., (frame.width - 1) as f32) as u32)
                * 4
                + 3) as usize];
            pixels.extend_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, alpha]);
        }
    }
    Some(Backdrop {
        width,
        height,
        pixels,
    })
}
// Closed-form distances for the same axis-aligned contour. The generic shared
// contour enumerates eight lines/arcs (with trigonometry) per pixel; the optical
// renderer only needs the top shoulder or the lower rounded box at each point.
fn lens_distance_normal(c: ShapeContour, x: f64, y: f64) -> (f64, [f64; 2]) {
    let w = c.total_width();
    let h = c.height;
    let r = c.radius;
    if c.form == FormFactor::Island {
        let (d, n) = rounded_distance_normal(
            [(x - w / 2.) as f32, (y - h / 2.) as f32],
            [(w / 2.) as f32, (h / 2.) as f32],
            r as f32,
        );
        return (d as f64, [n[0] as f64, n[1] as f64]);
    }
    let s = c.shoulder();
    if x < 0. || x > w || y < 0. || y > h {
        return c.distance_normal(x, y);
    }
    if y < s {
        let mut depth = y;
        let mut n = [0., -1.];
        for (dx, sign) in [(x, -1.), (w - x, 1.)] {
            let dy = y - s;
            let length = (dx * dx + dy * dy).sqrt();
            let inward = length - s;
            if inward < depth {
                depth = inward;
                n = if length > 1e-12 {
                    [sign * dx / length, -dy / length]
                } else {
                    [0., 1.]
                };
            }
        }
        return (-depth, n);
    }
    let mut depth = y;
    let mut normal = [0., -1.];
    let (edge, n) = if y > h - r && x < s + r {
        let dx = x - s - r;
        let dy = y - h + r;
        let length = (dx * dx + dy * dy).sqrt();
        (
            r - length,
            if length > 1e-12 {
                [dx / length, dy / length]
            } else {
                [-1., 0.]
            },
        )
    } else if y > h - r && x > w - s - r {
        let dx = x - w + s + r;
        let dy = y - h + r;
        let length = (dx * dx + dy * dy).sqrt();
        (
            r - length,
            if length > 1e-12 {
                [dx / length, dy / length]
            } else {
                [1., 0.]
            },
        )
    } else if x - s < w - s - x {
        (x - s, [-1., 0.])
    } else {
        (w - s - x, [1., 0.])
    };
    if edge < depth {
        depth = edge;
        normal = n;
    }
    // Inside the horizontal extent of the bottom segment it can be closest.
    if x >= s + r && x <= w - s - r && h - y < depth {
        depth = h - y;
        normal = [0., 1.];
    }
    (-depth, normal)
}
/// Exact rounded-box SDF with its outward unit gradient, including straight segments.
fn rounded_distance_normal(p: [f32; 2], half: [f32; 2], radius: f32) -> (f32, [f32; 2]) {
    let q = [p[0].abs() - half[0] + radius, p[1].abs() - half[1] + radius];
    let outside = [q[0].max(0.), q[1].max(0.)];
    let length = outside[0].hypot(outside[1]);
    let n = if length > 1e-6 {
        [
            outside[0] / length * p[0].signum(),
            outside[1] / length * p[1].signum(),
        ]
    } else if q[0] > q[1] {
        [p[0].signum(), 0.]
    } else {
        [0., p[1].signum()]
    };
    (length + q[0].max(q[1]).min(0.) - radius, n)
}
/// Snell's law for normalized incident ray (0,0,-1), following GLSL refract.
fn refract(n: [f32; 3], eta: f32) -> [f32; 3] {
    let dot = -n[2];
    let k = 1. - eta * eta * (1. - dot * dot);
    if k < 0. {
        return [0., 0., -1.];
    }
    let f = eta * dot + k.sqrt();
    [-f * n[0], -f * n[1], -eta - f * n[2]]
}
fn sample(frame: &Backdrop, x: f32, y: f32) -> [f32; 3] {
    if frame.width == 0
        || frame.height == 0
        || frame.pixels.len() < (frame.width * frame.height * 4) as usize
    {
        return [0.; 3];
    }
    let x = x.clamp(0., (frame.width - 1) as f32);
    let y = y.clamp(0., (frame.height - 1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(frame.width - 1);
    let y1 = (y0 + 1).min(frame.height - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let mut color = [0.; 3];
    for (c, value) in color.iter_mut().enumerate() {
        let pixel = |px, py| frame.pixels[((py * frame.width + px) * 4) as usize + c] as f32;
        *value = (pixel(x0, y0) * (1. - fx) + pixel(x1, y0) * fx) * (1. - fy)
            + (pixel(x0, y1) * (1. - fx) + pixel(x1, y1) * fx) * fy;
    }
    color
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distance_and_normals() {
        let (d, n) = rounded_distance_normal([50., 0.], [50., 25.], 12.);
        assert!(d.abs() < 1e-5);
        assert_eq!(n, [1., 0.]);
        assert!(rounded_distance_normal([0., 0.], [50., 25.], 12.).0 < 0.);
    }
    #[test]
    fn normal_incidence_is_undisplaced() {
        let ray = refract([0., 0., 1.], 1. / 1.45);
        assert_eq!(ray, [0., 0., -1.]);
    }
    #[test]
    fn refraction_is_finite_and_normalized() {
        for s in 0..100 {
            let x = s as f32 / 100.;
            let ray = refract([x, 0., (1. - x * x).sqrt()], 1. / 1.45);
            assert!(ray.iter().all(|v| v.is_finite()));
            assert!((ray.iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 1e-5);
        }
    }
    #[test]
    fn sample_interpolates_and_clamps() {
        let frame = Backdrop {
            width: 2,
            height: 1,
            pixels: vec![0, 0, 0, 255, 200, 100, 50, 255],
        };
        assert_eq!(sample(&frame, 0.5, 0.), [100., 50., 25.]);
        assert_eq!(sample(&frame, 8., 0.), [200., 100., 50.]);
    }
}

fn blur(frame: &mut Backdrop, radius: usize) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w == 0 || h == 0 || frame.pixels.len() < w * h * 4 {
        return;
    }
    let mut temporary = vec![0u8; w * h * 4];
    let window = (radius * 2 + 1) as u32;
    for y in 0..h {
        for c in 0..3 {
            let mut sum = 0u32;
            for i in 0..radius * 2 + 1 {
                let px = (i as isize - radius as isize).clamp(0, w as isize - 1) as usize;
                sum += frame.pixels[(y * w + px) * 4 + c] as u32;
            }
            for x in 0..w {
                temporary[(y * w + x) * 4 + c] = (sum / window) as u8;
                let remove = x.saturating_sub(radius);
                let add = (x + radius + 1).min(w - 1);
                sum = sum + frame.pixels[(y * w + add) * 4 + c] as u32
                    - frame.pixels[(y * w + remove) * 4 + c] as u32;
            }
        }
    }
    for x in 0..w {
        for c in 0..3 {
            let mut sum = 0u32;
            for i in 0..radius * 2 + 1 {
                let py = (i as isize - radius as isize).clamp(0, h as isize - 1) as usize;
                sum += temporary[(py * w + x) * 4 + c] as u32;
            }
            for y in 0..h {
                frame.pixels[(y * w + x) * 4 + c] = (sum / window) as u8;
                let remove = y.saturating_sub(radius);
                let add = (y + radius + 1).min(h - 1);
                sum = sum + temporary[(add * w + x) * 4 + c] as u32
                    - temporary[(remove * w + x) * 4 + c] as u32;
            }
        }
    }
}
#[cfg(test)]
mod compositor_tests {
    use super::*;

    #[test]
    fn changing_canvas_width_realigns_backdrop_and_invalidates_cached_image() {
        let mut pixels = Vec::new();
        for _ in 0..300 {
            for x in 0..1200 {
                pixels.extend_from_slice(&[(x / 5) as u8, 0, 0, 255]);
            }
        }
        let mut glass = Glass::default();
        glass.set_backdrop(Backdrop {
            width: 1200,
            height: 300,
            pixels,
        });
        let first = glass
            .image_in_canvas(360., 160., 8., 1., FormFactor::Notch, 800.)
            .to_rgba8()
            .unwrap();
        let second = glass
            .image_in_canvas(360., 160., 8., 1., FormFactor::Notch, 1200.)
            .to_rgba8()
            .unwrap();
        let index = (80 * first.width() + first.width() / 2) as usize;
        assert!(second.as_slice()[index].r > first.as_slice()[index].r + 30);
    }

    fn canvas_backdrop(color: [u8; 4]) -> Backdrop {
        let width = crate::state::OVERLAY_WIDTH as u32;
        let height = crate::state::OVERLAY_HEIGHT as u32;
        Backdrop {
            width,
            height,
            pixels: color.repeat((width * height) as usize),
        }
    }
    #[test]
    fn prefilter_preserves_constant_color_and_alpha() {
        let mut frame = Backdrop {
            width: 4,
            height: 3,
            pixels: [121u8, 81, 41, 255].repeat(12),
        };
        let expected = frame.pixels.clone();
        blur(&mut frame, 3);
        assert_eq!(frame.pixels, expected);
    }
    #[test]
    fn notch_lens_preserves_shoulder_cutouts_and_top_attachment() {
        let mut glass = Glass::default();
        let buffer = glass
            .image(220., 48., 24., 1., FormFactor::Notch)
            .to_rgba8()
            .unwrap();
        assert_eq!(buffer.width(), 236);
        // Without a backdrop alpha is the rim reflection, so supply an opaque
        // captured surface when checking coverage rather than reflectance.
        glass.set_backdrop(canvas_backdrop([80, 80, 80, 255]));
        let buffer = glass
            .image(220., 48., 24., 1., FormFactor::Notch)
            .to_rgba8()
            .unwrap();
        assert_eq!(buffer.as_slice()[4 * 236].a, 0);
        assert_eq!(buffer.as_slice()[118].a, 255);
        assert_eq!(buffer.as_slice()[4].a, 255);
    }
    #[test]
    fn lens_has_transparent_corners_and_samples_backdrop() {
        let mut glass = Glass::default();
        glass.set_backdrop(canvas_backdrop([100, 80, 60, 255]));
        let buffer = glass
            .image(220., 48., 24., 1., FormFactor::Island)
            .to_rgba8()
            .unwrap();
        assert_eq!(buffer.as_slice()[0].a, 0);
        let center = buffer.as_slice()[(24 * 220 + 110) as usize];
        assert_eq!(center.a, 255);
        assert!(center.r >= 100 && center.r < 150);
    }

    #[test]
    fn wide_notch_material_preserves_coverage_at_fractional_scale() {
        let mut glass = Glass::default();
        glass.set_backdrop(canvas_backdrop([80, 80, 80, 255]));
        let buffer = glass
            .image(640., 216., 8., 1.5, FormFactor::Notch)
            .to_rgba8()
            .unwrap();
        let scale = optical_scale(656., 216., 1.5);
        assert_eq!(
            (buffer.width(), buffer.height()),
            ((656. * scale).ceil() as u32, (216. * scale).ceil() as u32)
        );
        assert_eq!(
            buffer.as_slice()[((4. * scale) as u32 * buffer.width()) as usize].a,
            0
        );
        assert_eq!(
            buffer.as_slice()
                [((buffer.height() / 2) * buffer.width() + buffer.width() / 2) as usize]
                .a,
            255
        );
    }

    #[test]
    fn user_zoom_keeps_glass_pixels_and_captured_canvas_in_the_same_space() {
        for scale in [0.75, 1.5, 2.25] {
            let width = (crate::state::OVERLAY_WIDTH as f32 * scale) as u32;
            let height = (crate::state::OVERLAY_HEIGHT as f32 * scale) as u32;
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            for _ in 0..height {
                for x in 0..width {
                    pixels.extend_from_slice(&[(x * 255 / (width - 1)) as u8, 80, 60, 255]);
                }
            }
            let mut glass = Glass::default();
            glass.set_backdrop(Backdrop {
                width,
                height,
                pixels,
            });
            let image = glass
                .image(220., 48., 8., scale, FormFactor::Notch)
                .to_rgba8()
                .unwrap();
            let render_scale = optical_scale(236., 48., scale);
            assert_eq!(image.width(), (236. * render_scale).ceil() as u32);
            assert_eq!(image.height(), (48. * render_scale).ceil() as u32);
            let center = image.as_slice()
                [((image.height() / 2) * image.width() + image.width() / 2) as usize];
            assert_eq!(center.a, 255);
            assert!((120..=140).contains(&center.r), "scale {scale}: {center:?}");
        }
    }
}

#[cfg(test)]
mod bounded_work_tests {
    #[test]
    fn analytic_optical_contour_matches_shared_geometry() {
        for form in [super::FormFactor::Notch, super::FormFactor::Island] {
            for (w, h, r) in [
                (126., 32., 16.),
                (220., 48., 24.),
                (360., 160., 8.),
                (560., 400., 8.),
            ] {
                let c = super::ShapeContour::new(w, h, r, form);
                for yi in 0..40 {
                    for xi in 0..80 {
                        let x = (xi as f64 + 0.27) * c.total_width() / 80.;
                        let y = (yi as f64 + 0.31) * h / 40.;
                        let expected = c.distance_normal(x, y);
                        let got = super::lens_distance_normal(c, x, y);
                        assert!(
                            (expected.0 - got.0).abs() < 0.0001,
                            "{form:?} {w} {h} at {x},{y}: {expected:?} != {got:?}"
                        );
                        if expected.0.abs() > 0.0001 {
                            assert!(
                                expected
                                    .1
                                    .iter()
                                    .zip(got.1)
                                    .all(|(a, b)| (a - b).abs() < 0.0001),
                                "normal: {expected:?} != {got:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    use super::*;
    #[test]
    fn backdrop_and_optical_work_stay_bounded_at_high_dpi() {
        let mut glass = Glass::default();
        let frame = Backdrop {
            width: 2400,
            height: 1494,
            pixels: [80, 100, 120, 255].repeat(2400 * 1494),
        };
        assert!(glass.set_backdrop(frame.clone()));
        let revision = glass.revision;
        assert!(glass.backdrop.as_ref().unwrap().width <= 384);
        assert!(glass.backdrop.as_ref().unwrap().height <= 240);
        assert!(!glass.set_backdrop(frame));
        assert_eq!(glass.revision, revision);
        for scale in [1.3, 2., 3.] {
            let image = glass
                .image_in_canvas(560., 400., 8., scale, FormFactor::Notch, 800.)
                .to_rgba8()
                .unwrap();
            assert!(image.width() * image.height() <= 129_000);
        }
    }
    #[test]
    fn frosted_and_refractive_modes_have_distinct_cache_entries() {
        let mut glass = Glass::default();
        glass.set_backdrop(Backdrop {
            width: 800,
            height: 498,
            pixels: [90, 100, 110, 255].repeat(800 * 498),
        });
        let glass_pixels = glass
            .image_in_canvas(360., 160., 8., 1., FormFactor::Notch, 800.)
            .to_rgba8()
            .unwrap();
        let frost = glass
            .frosted_image_in_canvas(360., 160., 8., 1., FormFactor::Notch, 800.)
            .to_rgba8()
            .unwrap();
        assert_ne!(glass_pixels.as_slice(), frost.as_slice());
        assert_eq!(
            frost.as_slice()[80 * 376 + 188],
            Rgba8Pixel {
                r: 90,
                g: 100,
                b: 110,
                a: 255
            }
        );
    }
    #[test]
    fn invalid_capture_does_not_replace_a_valid_backdrop() {
        let mut glass = Glass::default();
        assert!(!glass.set_backdrop(Backdrop {
            width: u32::MAX,
            height: u32::MAX,
            pixels: vec![]
        }));
        assert!(glass.backdrop.is_none());
    }
}
