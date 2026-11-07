//! A small CPU lens compositor for the optional captured-desktop glass material.
//! No time uniform, animation loop, screen capture, or global state lives here.
//! The rounded-box distance gradient defines a beveled normal; Snell refraction
//! displaces backdrop samples and Schlick Fresnel supplies restrained rim reflection.
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
    cache: Option<((u32, u32, u32, u32, u64), Image)>,
}
impl Glass {
    pub fn set_backdrop(&mut self, mut frame: Backdrop) -> bool {
        // Small separable box passes approximate a soft Gaussian prefilter. Text
        // behind the lens becomes restrained rather than competing with controls.
        let radius = (frame.width / 140).clamp(1, 12) as usize;
        blur(&mut frame, radius);
        blur(&mut frame, radius);
        if self.backdrop.as_ref().is_some_and(|old| {
            old.width == frame.width && old.height == frame.height && old.pixels == frame.pixels
        }) {
            return false;
        }
        self.backdrop = Some(frame);
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn clear(&mut self) {
        self.backdrop = None;
        self.cache = None;
    }
    pub fn image(&mut self, width: f64, height: f64, radius: f64, scale: f32) -> Image {
        let scale = scale.clamp(1., 4.);
        let w = (width as f32 * scale).ceil().max(1.) as u32;
        let h = (height as f32 * scale).ceil().max(1.) as u32;
        let r = radius as f32 * scale;
        let key = (
            w,
            h,
            (r * 64.).round() as u32,
            scale.to_bits(),
            self.revision,
        );
        if let Some((old, image)) = &self.cache {
            if *old == key {
                return image.clone();
            }
        }
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        let left = (420. * scale - w as f32) * 0.5;
        let bezel = (12. * scale).min(h as f32 * 0.46);
        for y in 0..h {
            for x in 0..w {
                let p = [
                    x as f32 + 0.5 - w as f32 / 2.,
                    y as f32 + 0.5 - h as f32 / 2.,
                ];
                let (d, n) = rounded_distance_normal(
                    p,
                    [w as f32 / 2., h as f32 / 2.],
                    r.min(h as f32 / 2.),
                );
                let coverage = (0.5 - d).clamp(0., 1.);
                if coverage <= 0. {
                    continue;
                }
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
                let pixel = if let Some(frame) = &self.backdrop {
                    let mut c = sample(frame, left + x as f32 + dx, y as f32 + dy);
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
                buffer.make_mut_slice()[(y * w + x) as usize] = pixel;
            }
        }
        let image = Image::from_rgba8(buffer);
        self.cache = Some((key, image.clone()));
        image
    }
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
    fn lens_has_transparent_corners_and_samples_backdrop() {
        let mut glass = Glass::default();
        glass.set_backdrop(Backdrop {
            width: 420,
            height: 220,
            pixels: [100u8, 80, 60, 255].repeat(420 * 220),
        });
        let buffer = glass.image(220., 48., 24., 1.).to_rgba8().unwrap();
        assert_eq!(buffer.as_slice()[0].a, 0);
        let center = buffer.as_slice()[(24 * 220 + 110) as usize];
        assert_eq!(center.a, 255);
        assert!(center.r >= 100 && center.r < 150);
    }
}
