//! Context-owned, event-driven Liquid Glass optics. No capture or animation clock.
use crate::{material::Backdrop, shape::ShapeContour};
use glow::HasContext;
use std::{num::NonZeroU32, time::Instant};

const MAX_PIXELS: f64 = 1_048_576.;
const VERTEX: &str = "#version 330 core\nvoid main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(p*2.-1.,0,1);}";

#[derive(Clone, Copy, PartialEq)]
pub struct Geometry {
    pub canvas: [f32; 2],
    pub shape: [f32; 4],
    pub scale: f32,
    pub tint: f32,
    pub dark: bool,
}
impl Geometry {
    pub fn new(canvas: [f32; 2], contour: ShapeContour, scale: f32, tint: f32, dark: bool) -> Self {
        Self {
            canvas,
            shape: [
                contour.total_width() as f32,
                contour.height as f32,
                contour.radius as f32,
                contour.shoulder() as f32,
            ],
            scale,
            tint,
            dark,
        }
    }
    fn size(self) -> [i32; 2] {
        let scale = (self.scale as f64)
            .min((MAX_PIXELS / (self.canvas[0] as f64 * self.canvas[1] as f64).max(1.)).sqrt());
        self.canvas
            .map(|v| (v as f64 * scale).floor().max(1.) as i32)
    }
}

/// Filter only when a capture arrives, including while the spring changes geometry.
#[derive(Default)]
pub struct Palette {
    luminance: Option<f32>,
    sampled: Option<Instant>,
    pub dark: bool,
    pub mixed: bool,
}
impl Palette {
    pub fn sample(&mut self, frame: &Backdrop, region: [f32; 4], now: Instant) {
        let [left, top, right, bottom] = region;
        let mut sum = 0.;
        let mut count = 0.;
        let mut lo = 1f32;
        let mut hi = 0f32;
        for y in (top.max(0.) as u32..(bottom as u32).min(frame.height)).step_by(8) {
            for x in (left.max(0.) as u32..(right as u32).min(frame.width)).step_by(8) {
                let i = ((y * frame.width + x) * 4) as usize;
                if let Some(p) = frame.pixels.get(i..i + 3) {
                    let l = linear(p[0]) * 0.2126 + linear(p[1]) * 0.7152 + linear(p[2]) * 0.0722;
                    sum += l;
                    count += 1.;
                    lo = lo.min(l);
                    hi = hi.max(l);
                }
            }
        }
        if count == 0. {
            return;
        }
        let measured = sum / count;
        let weight = self.sampled.map_or(1., |last| {
            1. - (-now.duration_since(last).as_secs_f32() / 0.15).exp()
        });
        let l = self
            .luminance
            .map_or(measured, |old| old + weight * (measured - old));
        self.luminance = Some(l);
        self.sampled = Some(now);
        if l < 0.35 {
            self.dark = true;
        } else if l > 0.55 {
            self.dark = false;
        }
        self.mixed = hi - lo > 0.35;
    }
}
fn linear(v: u8) -> f32 {
    let c = v as f32 / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub struct Renderer {
    gl: glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    fbo: glow::Framebuffer,
    textures: [glow::Texture; 4], // source, horizontal, vertical, stable output image
    uniforms: [Option<glow::UniformLocation>; 10],
    source_size: [i32; 2],
    output_size: [i32; 2],
    uploaded: Option<u64>,
    rendered: Option<(u64, Geometry, bool)>,
    output: usize,
    pub submissions: u64,
    pub submission_ms: f64,
}
impl Renderer {
    pub fn invalidate(&mut self) {
        self.rendered = None;
    }
    /// Call exclusively from a Slint rendering notifier with its current context.
    pub unsafe fn new(gl: glow::Context) -> Result<Self, String> {
        unsafe {
            let _state = State::save(&gl);
            let program = gl.create_program()?;
            for (kind, source) in [
                (glow::VERTEX_SHADER, VERTEX),
                (glow::FRAGMENT_SHADER, include_str!("liquid.frag")),
            ] {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    let error = gl.get_shader_info_log(shader);
                    gl.delete_shader(shader);
                    gl.delete_program(program);
                    return Err(error);
                }
                gl.attach_shader(program, shader);
                gl.delete_shader(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let error = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(error);
            }
            let vao = gl.create_vertex_array()?;
            let fbo = gl.create_framebuffer()?;
            let textures = [
                gl.create_texture()?,
                gl.create_texture()?,
                gl.create_texture()?,
                gl.create_texture()?,
            ];
            gl.active_texture(glow::TEXTURE0);
            for texture in textures {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                for (key, value) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, key, value as i32);
                }
            }
            let uniforms = [
                "backdrop",
                "blurred",
                "canvas",
                "target_size",
                "shape",
                "edges[0]",
                "tint",
                "support",
                "pass",
                "unused",
            ]
            .map(|name| gl.get_uniform_location(program, name));
            // Restore before moving the context into the renderer.
            drop(_state);
            Ok(Self {
                gl,
                program,
                vao,
                fbo,
                textures,
                uniforms,
                source_size: [0; 2],
                output_size: [0; 2],
                uploaded: None,
                rendered: None,
                output: 3,
                submissions: 0,
                submission_ms: 0.,
            })
        }
    }
    pub fn render(
        &mut self,
        frame: &Backdrop,
        revision: u64,
        geometry: Geometry,
        contour: ShapeContour,
        mixed: bool,
    ) -> Result<Option<slint::Image>, String> {
        if self.rendered == Some((revision, geometry, mixed)) {
            return Ok(None);
        }
        let started = Instant::now();
        // SAFETY: caller is the notifier; all names belong to its current context.
        unsafe {
            let gl = &self.gl;
            let _state = State::save(gl);
            gl.bind_vertex_array(Some(self.vao));
            gl.use_program(Some(self.program));
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            for flag in FLAGS {
                gl.disable(flag);
            }
            gl.color_mask(true, true, true, true);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
            gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
            gl.active_texture(glow::TEXTURE0);
            let size = [frame.width as i32, frame.height as i32];
            if self.source_size != size {
                for i in 0..3 {
                    storage(gl, self.textures[i], size);
                }
                self.source_size = size;
                self.uploaded = None;
            }
            let output_size = geometry.size();
            let publish = self.rendered.is_none() || output_size != self.output_size;
            if output_size != self.output_size {
                // Ordered GL commands retain old storage until prior uses finish.
                for i in 3..4 {
                    storage(gl, self.textures[i], output_size);
                }
                self.output_size = output_size;
            }
            gl.uniform_1_i32(self.uniforms[0].as_ref(), 0);
            gl.uniform_1_i32(self.uniforms[1].as_ref(), 1);
            gl.uniform_2_f32(
                self.uniforms[2].as_ref(),
                geometry.canvas[0],
                geometry.canvas[1],
            );
            if self.uploaded != Some(revision)
                || self
                    .rendered
                    .is_some_and(|(_, old, _)| old.canvas != geometry.canvas)
            {
                gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[0]));
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    size[0],
                    size[1],
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&frame.pixels)),
                );
                for (pass, source, destination) in [(1, 0, 1), (2, 1, 2)] {
                    gl.uniform_1_i32(self.uniforms[8].as_ref(), pass);
                    self.draw(source, destination, size)?;
                }
                self.uploaded = Some(revision);
            }
            gl.uniform_1_i32(self.uniforms[8].as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[2]));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_4_f32_slice(self.uniforms[4].as_ref(), &geometry.shape);
            gl.uniform_4_f32_slice(self.uniforms[5].as_ref(), &contour.shader_edges());
            gl.uniform_1_f32(self.uniforms[6].as_ref(), geometry.tint);
            gl.uniform_1_f32(
                self.uniforms[7].as_ref(),
                if mixed {
                    if geometry.dark { -1. } else { 1. }
                } else {
                    0.
                },
            );
            // Slint and this pass submit on the same context. Command ordering
            // protects earlier reads; replacing the Image every spring tick only
            // invalidates Slint's scene and schedules a redundant second frame.
            self.draw(0, self.output, output_size)?;
            self.rendered = Some((revision, geometry, mixed));
            self.submissions += 1;
            self.submission_ms += started.elapsed().as_secs_f64() * 1000.;
            Ok(publish.then(|| {
                slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                    self.textures[self.output].0,
                    [output_size[0] as u32, output_size[1] as u32].into(),
                )
                .build()
            }))
        }
    }
    unsafe fn draw(&self, source: usize, destination: usize, size: [i32; 2]) -> Result<(), String> {
        unsafe {
            let gl = &self.gl;
            gl.bind_texture(glow::TEXTURE_2D, Some(self.textures[source]));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(self.textures[destination]),
                0,
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                return Err("Incomplete Liquid Glass framebuffer".into());
            }
            gl.viewport(0, 0, size[0], size[1]);
            gl.uniform_2_f32(self.uniforms[3].as_ref(), size[0] as f32, size[1] as f32);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            Ok(())
        }
    }
    /// Release every borrowed Image first. The notifier guarantees a current context.
    pub unsafe fn destroy(self) {
        unsafe {
            for texture in self.textures {
                self.gl.delete_texture(texture);
            }
            self.gl.delete_framebuffer(self.fbo);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_program(self.program);
        }
    }
}
unsafe fn storage(gl: &glow::Context, texture: glow::Texture, size: [i32; 2]) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            size[0],
            size[1],
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
    }
}
const FLAGS: [u32; 7] = [
    glow::BLEND,
    glow::DEPTH_TEST,
    glow::STENCIL_TEST,
    glow::SCISSOR_TEST,
    glow::CULL_FACE,
    glow::FRAMEBUFFER_SRGB,
    glow::RASTERIZER_DISCARD,
];
struct State<'a> {
    gl: &'a glow::Context,
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    draw: Option<glow::Framebuffer>,
    read: Option<glow::Framebuffer>,
    active: u32,
    texture: Option<glow::Texture>,
    sampler: Option<glow::Sampler>,
    texture1: Option<glow::Texture>,
    sampler1: Option<glow::Sampler>,
    unpack: Option<glow::Buffer>,
    pixel: [i32; 4],
    viewport: [i32; 4],
    mask: [bool; 4],
    flags: [bool; 7],
}
impl<'a> State<'a> {
    unsafe fn save(gl: &'a glow::Context) -> Self {
        unsafe {
            let active = gl.get_parameter_i32(glow::ACTIVE_TEXTURE) as u32;
            gl.active_texture(glow::TEXTURE1);
            let texture1 = NonZeroU32::new(gl.get_parameter_i32(glow::TEXTURE_BINDING_2D) as u32)
                .map(glow::NativeTexture);
            let sampler1 = NonZeroU32::new(gl.get_parameter_i32(glow::SAMPLER_BINDING) as u32)
                .map(glow::NativeSampler);
            gl.bind_sampler(1, None);
            gl.active_texture(glow::TEXTURE0);
            let texture = NonZeroU32::new(gl.get_parameter_i32(glow::TEXTURE_BINDING_2D) as u32)
                .map(glow::NativeTexture);
            let sampler = NonZeroU32::new(gl.get_parameter_i32(glow::SAMPLER_BINDING) as u32)
                .map(glow::NativeSampler);
            gl.bind_sampler(0, None);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let mut mask = [0; 4];
            gl.get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut mask);
            Self {
                gl,
                active,
                texture,
                sampler,
                texture1,
                sampler1,
                viewport,
                mask: mask.map(|v| v != 0),
                flags: FLAGS.map(|f| gl.is_enabled(f)),
                program: NonZeroU32::new(gl.get_parameter_i32(glow::CURRENT_PROGRAM) as u32)
                    .map(glow::NativeProgram),
                vao: NonZeroU32::new(gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING) as u32)
                    .map(glow::NativeVertexArray),
                draw: NonZeroU32::new(gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) as u32)
                    .map(glow::NativeFramebuffer),
                read: NonZeroU32::new(gl.get_parameter_i32(glow::READ_FRAMEBUFFER_BINDING) as u32)
                    .map(glow::NativeFramebuffer),
                unpack: NonZeroU32::new(
                    gl.get_parameter_i32(glow::PIXEL_UNPACK_BUFFER_BINDING) as u32
                )
                .map(glow::NativeBuffer),
                pixel: [
                    glow::UNPACK_ALIGNMENT,
                    glow::UNPACK_ROW_LENGTH,
                    glow::UNPACK_SKIP_ROWS,
                    glow::UNPACK_SKIP_PIXELS,
                ]
                .map(|p| gl.get_parameter_i32(p)),
            }
        }
    }
}
impl Drop for State<'_> {
    fn drop(&mut self) {
        // SAFETY: guard never escapes the current-context notifier invocation.
        unsafe {
            let gl = self.gl;
            gl.use_program(self.program);
            gl.bind_vertex_array(self.vao);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.draw);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, self.read);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, self.texture1);
            gl.bind_sampler(1, self.sampler1);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, self.texture);
            gl.bind_sampler(0, self.sampler);
            gl.active_texture(self.active);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, self.unpack);
            for (p, v) in [
                glow::UNPACK_ALIGNMENT,
                glow::UNPACK_ROW_LENGTH,
                glow::UNPACK_SKIP_ROWS,
                glow::UNPACK_SKIP_PIXELS,
            ]
            .into_iter()
            .zip(self.pixel)
            {
                gl.pixel_store_i32(p, v);
            }
            gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            gl.color_mask(self.mask[0], self.mask[1], self.mask[2], self.mask[3]);
            for (flag, enabled) in FLAGS.into_iter().zip(self.flags) {
                if enabled {
                    gl.enable(flag)
                } else {
                    gl.disable(flag)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn primary_texture_is_bounded_at_large_dpi() {
        let g = Geometry::new(
            [1600., 900.],
            ShapeContour::new(640., 216., 16., crate::shape::FormFactor::Notch),
            4.,
            0.55,
            true,
        );
        let [w, h] = g.size();
        assert!(w * h <= 1_048_576);
    }
    #[test]
    fn palette_measures_content_and_holds_the_dead_band() {
        let mut f = Backdrop {
            width: 64,
            height: 32,
            pixels: [255, 255, 255, 255].repeat(64 * 32),
        };
        for y in 8..24 {
            for x in 16..48 {
                let i = (y * 64 + x) * 4;
                f.pixels[i..i + 3].fill(10);
            }
        }
        let mut p = Palette::default();
        let now = Instant::now();
        p.sample(&f, [16., 8., 48., 24.], now);
        assert!(p.dark);
        f.pixels = [180, 180, 180, 255].repeat(64 * 32);
        p.sample(
            &f,
            [0., 0., 64., 32.],
            now + std::time::Duration::from_secs(1),
        );
        assert!(p.dark);
        f.pixels.fill(255);
        p.sample(
            &f,
            [0., 0., 64., 32.],
            now + std::time::Duration::from_secs(2),
        );
        assert!(!p.dark);
    }
    #[test]
    fn srgb_reference_values() {
        assert_eq!(linear(0), 0.);
        assert_eq!(linear(255), 1.);
        assert!((linear(128) - 0.21586).abs() < 0.0001);
    }
}
