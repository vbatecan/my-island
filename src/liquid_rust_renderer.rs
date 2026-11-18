//! Offscreen Liquid Rust rendering. GPU submission and readback stay off Slint's UI thread.
use crate::{
    GlassMaterial,
    material::{Backdrop, lens_distance_normal},
    settings::LiquidGlassSettings,
    shape::ShapeContour,
};
use liquid_rust::{
    Color, Frame, Glass, GlassId, Material, Rect, Renderer, Scene, Shadow, Shape, Tone,
};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use wgpu::{BufferUsages, TextureUsages};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const MAX_PIXELS: f32 = 1_048_576.;
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Lens {
    pub id: i32,
    pub rect: [f32; 4],
    pub radius: f32,
    pub exposure: f32,
    pub layer: i32,
    pub pressed: bool,
    pub material: GlassMaterial,
}

#[derive(Clone, PartialEq)]
pub struct Layout {
    pub canvas: [f32; 2],
    pub contour: ShapeContour,
    pub scale: f32,
    pub tint: f32,
    pub optics: LiquidGlassSettings,
    pub sample_y: f32,
    pub controls: Arc<[Lens]>,
    pub motion_speed: f32,
}

impl Layout {
    fn pixel_scale(&self) -> f32 {
        self.scale
            .min((MAX_PIXELS / (self.canvas[0] * self.canvas[1])).sqrt())
    }

    fn compatible(&self, other: &Self) -> bool {
        self.contour.form == other.contour.form
            && self.canvas == other.canvas
            && self.scale == other.scale
            && self.tint == other.tint
            && self.optics == other.optics
    }

    fn same_controls(&self, other: &Self) -> bool {
        self.controls.len() == other.controls.len()
            && self
                .controls
                .iter()
                .zip(&*other.controls)
                .all(|(a, b)| a.id == b.id && a.material == b.material)
    }
}

type Pixels = SharedPixelBuffer<Rgba8Pixel>;
struct FramePixels {
    shell: Pixels,
    controls: Option<Pixels>,
    moving: bool,
}
pub struct RenderedFrame {
    pub shell: Image,
    pub controls: Option<Image>,
}
type RenderResult = Result<FramePixels, String>;
type Completed = Arc<Mutex<Option<RenderResult>>>;

struct Job {
    backdrop: Arc<Backdrop>,
    revision: u64,
    layout: Layout,
}

pub struct Worker {
    send: mpsc::Sender<Job>,
    completed: Completed,
    last: Option<(u64, Layout)>,
    published_controls: Option<Layout>,
    busy: bool,
    moving: bool,
}

impl Worker {
    pub fn start(notify: impl Fn() + Send + 'static) -> Result<Self, String> {
        let (send, receive) = mpsc::channel::<Job>();
        let completed = Completed::default();
        let output = completed.clone();
        std::thread::Builder::new()
            .name("liquid-rust".into())
            .spawn(move || {
                let mut gpu = None;
                while let Ok(job) = receive.recv() {
                    let result = (|| {
                        if gpu.is_none() {
                            gpu = Some(pollster::block_on(Gpu::new())?);
                        }
                        gpu.as_mut().unwrap().render(job)
                    })();
                    let failed = result.is_err();
                    *output.lock().unwrap() = Some(result);
                    notify();
                    if failed {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            send,
            completed,
            last: None,
            published_controls: None,
            busy: false,
            moving: false,
        })
    }

    pub fn update(
        &mut self,
        backdrop: &Arc<Backdrop>,
        revision: u64,
        layout: &Layout,
    ) -> Result<Option<RenderedFrame>, String> {
        let mut image = None;
        if !self.controls_match(layout) {
            self.published_controls = None;
        }
        if let Some(result) = self.completed.lock().unwrap().take() {
            self.busy = false;
            match result {
                Ok(frame) => {
                    self.moving = frame.moving;
                    if let Some((_, previous)) = self.last.as_ref()
                        && previous.compatible(layout)
                    {
                        let controls = frame.controls.filter(|_| previous.same_controls(layout));
                        if controls.is_some() {
                            self.published_controls = Some(previous.clone());
                        }
                        image = Some(RenderedFrame {
                            shell: Image::from_rgba8(frame.shell),
                            controls: controls.map(Image::from_rgba8),
                        });
                    }
                }
                Err(error) => return Err(error),
            }
        }
        // One job at a time. The completion redraw pulls the most recent capture and shape.
        if !self.busy
            && (self.moving
                || self
                    .last
                    .as_ref()
                    .is_none_or(|(old, previous)| *old != revision || previous != layout))
        {
            self.send
                .send(Job {
                    backdrop: backdrop.clone(),
                    revision,
                    layout: layout.clone(),
                })
                .map_err(|error| error.to_string())?;
            self.last = Some((revision, layout.clone()));
            self.busy = true;
        }
        Ok(image)
    }

    pub fn controls_match(&self, current: &Layout) -> bool {
        !current.controls.is_empty()
            && self.published_controls.as_ref().is_some_and(|previous| {
                previous.compatible(current) && previous.same_controls(current)
            })
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    scene: Scene,
    glass: liquid_rust::GlassId,
    controls: Option<ControlPass>,
    surface: Option<Surface>,
    upload: Vec<u8>,
}

impl Gpu {
    async fn new() -> Result<Self, String> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12 | wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|error| error.to_string())?;
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let renderer = Renderer::new(&device, FORMAT);
        if let Some(error) = errors.pop().await {
            return Err(error.to_string());
        }

        let mut scene = Scene::new();
        scene.physics = false;
        let glass = scene.add(Glass::new(Rect::new(0., 0., 1., 1.)));
        Ok(Self {
            device,
            queue,
            renderer,
            scene,
            glass,
            controls: None,
            surface: None,
            upload: Vec::new(),
        })
    }

    fn render(&mut self, job: Job) -> RenderResult {
        let Job {
            backdrop,
            revision,
            layout,
        } = job;
        let scale = layout.pixel_scale();
        let [width, height] = layout
            .canvas
            .map(|length| (length * scale).floor().max(1.) as u32);
        if self
            .surface
            .as_ref()
            .is_none_or(|surface| surface.size != (width, height))
        {
            self.surface = Some(Surface::new(&self.device, width, height));
        }
        let surface = self.surface.as_mut().unwrap();
        let offset = (layout.sample_y * backdrop.height as f32 / layout.canvas[1]).round() as i32;
        let content_changed = surface.content_key != Some((revision, offset));
        if content_changed {
            let pixels = resized_backdrop(&backdrop, width, height, offset, &mut self.upload);
            self.queue.write_texture(
                surface.content.as_image_copy(),
                pixels,
                rows(4 * width),
                surface.content.size(),
            );
            surface.content_key = Some((revision, offset));
        }

        let contour = layout.contour;
        let radius = contour.radius as f32;
        // Put the lens's top bevel above the visible panel. The native contour supplies
        // the notch shoulders and the Island's upper corners when we copy it back.
        let above = radius + (layout.optics.depth as f32 + 8.).max(16.);
        self.scene.place(
            self.glass,
            Rect::new(
                (width as f32 / scale - contour.width as f32) / 2.,
                -above,
                contour.width as f32,
                contour.height as f32 + above,
            ),
        );
        self.scene.set_shape(self.glass, Shape::Circular(radius));
        self.scene.set_material(self.glass, material(layout.optics));
        self.scene.update(0.);

        let errors = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &Frame {
                content: &surface.content_view,
                target: &surface.target_view,
                size: surface.size,
                scale,
                content_changed,
            },
            &self.scene,
        );
        let has_controls = !layout.controls.is_empty();
        let mut moving = false;
        if has_controls {
            let target = surface.controls.get_or_insert_with(|| {
                let texture = Surface::texture(
                    &self.device,
                    surface.size,
                    TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                );
                let view = texture.create_view(&Default::default());
                (texture, view)
            });
            let pass = self
                .controls
                .get_or_insert_with(|| ControlPass::new(&self.device));
            moving = pass.place(&layout.controls, layout.optics, layout.motion_speed);
            pass.renderer.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &Frame {
                    content: &surface.target_view,
                    target: &target.1,
                    size: surface.size,
                    scale,
                    content_changed: true,
                },
                &pass.scene,
            );
        } else {
            if let Some(pass) = &mut self.controls {
                pass.clear();
            }
        }

        let panel_width = ((contour.total_width() as f32 * scale).round() as u32).clamp(1, width);
        let panel_height = ((contour.height as f32 * scale).round() as u32).clamp(1, height);
        let stride = (4 * panel_width).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let panel_bytes = u64::from(stride) * u64::from(panel_height);
        let readback_bytes = if has_controls {
            panel_bytes * 2
        } else {
            panel_bytes
        };
        if surface.readback.size() < readback_bytes {
            surface.readback = Surface::readback(&self.device, readback_bytes);
        }
        let mut copy_panel = |texture: &wgpu::Texture, offset| {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    origin: wgpu::Origin3d {
                        x: (width - panel_width) / 2,
                        y: 0,
                        z: 0,
                    },
                    ..texture.as_image_copy()
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &surface.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        ..rows(stride)
                    },
                },
                extent(panel_width, panel_height),
            );
        };
        copy_panel(&surface.target, 0);
        if has_controls {
            copy_panel(&surface.controls.as_ref().unwrap().0, panel_bytes);
        }
        let submission = self.queue.submit([encoder.finish()]);
        if let Some(error) = pollster::block_on(errors.pop()) {
            return Err(error.to_string());
        }
        let slice = surface.readback.slice(..readback_bytes);
        let (send, receive) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(2)),
            })
            .map_err(|error| error.to_string())?;
        receive
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let data = slice
            .get_mapped_range()
            .map_err(|error| error.to_string())?;
        let (shell, controls) = data.split_at(panel_bytes as usize);
        let shell = mask_panel(shell, stride as usize, panel_width, panel_height, &layout);
        let controls = (!controls.is_empty()).then(|| {
            panel_pixels(
                controls,
                stride as usize,
                (panel_width, panel_height),
                layout.tint,
                |x, y| shell.as_slice()[y * panel_width as usize + x].a,
            )
        });
        drop(data);
        surface.readback.unmap();
        Ok(FramePixels {
            shell,
            controls,
            moving,
        })
    }
}

// The upper pass samples the refracted shell. Slint paints labels and values
// afterward, so the glass can bend the backdrop without distorting the controls.
struct ControlPass {
    renderer: Renderer,
    scene: Scene,
    ids: Vec<(i32, GlassId)>,
    pressing: Option<i32>,
    last: Option<Instant>,
}

impl ControlPass {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            renderer: Renderer::new(device, FORMAT),
            scene: Scene::new(),
            ids: Vec::new(),
            pressing: None,
            last: None,
        }
    }

    fn clear(&mut self) {
        if !self.ids.is_empty() {
            self.scene = Scene::new();
            self.ids.clear();
            self.pressing = None;
            self.last = None;
        }
    }

    fn place(&mut self, lenses: &[Lens], optics: LiquidGlassSettings, speed: f32) -> bool {
        if self.ids.len() != lenses.len()
            || self
                .ids
                .iter()
                .zip(lenses)
                .any(|((id, _), lens)| *id != lens.id)
        {
            self.clear();
            self.scene.physics = false;
            for lens in lenses {
                let [x, y, w, h] = lens.rect;
                let id = self.scene.add(
                    Glass::new(Rect::new(x, y, w, h))
                        .interactive()
                        .z(lens.layer),
                );
                self.ids.push((lens.id, id));
            }
            self.scene.update(0.);
        }
        let base = material(optics);
        for ((_, id), lens) in self.ids.iter().copied().zip(lenses) {
            let [x, y, width, height] = lens.rect;
            let mut face = base;
            let tint = match lens.material {
                GlassMaterial::Shared => None,
                GlassMaterial::ControlDark => Some(0xcbd8e6),
                GlassMaterial::ControlLight => Some(0x33475a),
            };
            if let Some(tint) = tint {
                face.refraction *= 0.75;
                face.depth = face.depth.min(5.);
                face.dispersion *= 0.5;
                face.frost = face.frost.clamp(2., 6.);
                face.tint = Some(Color::hex(tint).with_alpha(0.12 * lens.exposure));
            }
            face.depth = face.depth.min(width.min(height) * 0.32);
            face.refraction *= lens.exposure;
            face.frost *= lens.exposure;
            face.dispersion *= lens.exposure;
            face.specular *= lens.exposure;
            face.rim *= lens.exposure;
            face.rim_shade *= lens.exposure;
            self.scene.place(id, Rect::new(x, y, width, height));
            self.scene.set_shape(id, Shape::Circular(lens.radius));
            self.scene.set_material(id, face);
        }
        self.scene.physics = speed > 0.;
        let press = lenses.iter().rev().find(|lens| lens.pressed);
        let pressing = press.map(|lens| lens.id);
        if self.pressing != pressing {
            self.scene.pointer_up();
            if let Some(lens) = press {
                let [x, y, w, h] = lens.rect;
                self.scene
                    .pointer_down(liquid_rust::Vec2::new(x + w / 2., y + h / 2.));
            }
            self.pressing = pressing;
        }
        let now = Instant::now();
        let dt = self
            .last
            .replace(now)
            .map_or(1. / 60., |last| (now - last).as_secs_f32());
        self.scene.update(dt * speed)
    }
}

fn material(optics: LiquidGlassSettings) -> Material {
    let preset = Material::clear();
    let specular = optics.rim_light as f32 / 100.;
    let rim_strength = specular / preset.specular;
    Material {
        refraction: optics.refraction as f32 / 100.,
        depth: optics.depth as f32,
        dispersion: optics.dispersion as f32 / 100.,
        frost: optics.frost as f32,
        specular,
        rim: preset.rim * rim_strength,
        rim_shade: preset.rim_shade * rim_strength,
        light_tone: Tone::NONE,
        dark_tone: Tone::NONE,
        shadow: Shadow {
            offset_y: 0.,
            blur: 0.,
            opacity: 0.,
        },
        ..preset
    }
}

struct Surface {
    size: (u32, u32),
    content: wgpu::Texture,
    content_view: wgpu::TextureView,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    controls: Option<(wgpu::Texture, wgpu::TextureView)>,
    content_key: Option<(u64, i32)>,
}

impl Surface {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = |usage| Self::texture(device, (width, height), usage);
        let content = texture(TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST);
        let target = texture(
            TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC
                | TextureUsages::TEXTURE_BINDING,
        );
        let stride = (4 * width).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        Self {
            size: (width, height),
            content_view: content.create_view(&Default::default()),
            content,
            target_view: target.create_view(&Default::default()),
            target,
            readback: Self::readback(device, u64::from(stride) * u64::from(height)),
            controls: None,
            content_key: None,
        }
    }

    fn texture(device: &wgpu::Device, size: (u32, u32), usage: TextureUsages) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent(size.0, size.1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage,
            view_formats: &[],
        })
    }

    fn readback(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

fn rows(stride: u32) -> wgpu::TexelCopyBufferLayout {
    wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(stride),
        rows_per_image: None,
    }
}

fn resized_backdrop<'a>(
    frame: &'a Backdrop,
    width: u32,
    height: u32,
    offset: i32,
    buffer: &'a mut Vec<u8>,
) -> &'a [u8] {
    if (width, height, offset) == (frame.width, frame.height, 0) {
        return &frame.pixels;
    }
    buffer.clear();
    buffer.reserve((4 * width * height) as usize);
    for y in 0..height {
        let source_y =
            ((y * frame.height / height) as i32 + offset).clamp(0, frame.height as i32 - 1) as u32;
        let start = (source_y * frame.width * 4) as usize;
        let source = &frame.pixels[start..start + (frame.width * 4) as usize];
        if width == frame.width {
            buffer.extend_from_slice(source);
        } else {
            for x in 0..width {
                let index = (x * frame.width / width * 4) as usize;
                buffer.extend_from_slice(&source[index..index + 4]);
            }
        }
    }
    buffer
}

fn mask_panel(data: &[u8], stride: usize, width: u32, height: u32, layout: &Layout) -> Pixels {
    let contour = layout.contour;
    let dx = contour.total_width() / f64::from(width);
    let dy = contour.height / f64::from(height);
    panel_pixels(data, stride, (width, height), layout.tint, |x, y| {
        let px = (x as f64 + 0.5) * dx;
        let py = (y as f64 + 0.5) * dy;
        let (distance, _) = lens_distance_normal(contour, px, py);
        (255. * (0.5 - distance / dx).clamp(0., 1.)).round() as u8
    })
}

fn panel_pixels(
    data: &[u8],
    stride: usize,
    (width, height): (u32, u32),
    tint: f32,
    alpha: impl Fn(usize, usize) -> u8,
) -> Pixels {
    let mut image = Pixels::new(width, height);
    let tint = 1. - tint.clamp(0., 1.);
    let channel = |value: u8| (f32::from(value) * tint).round() as u8;
    for (y, row) in image
        .make_mut_slice()
        .chunks_exact_mut(width as usize)
        .enumerate()
    {
        let source = &data[y * stride..][..4 * width as usize];
        for (x, (pixel, rgb)) in row.iter_mut().zip(source.chunks_exact(4)).enumerate() {
            *pixel = Rgba8Pixel {
                r: channel(rgb[0]),
                g: channel(rgb[1]),
                b: channel(rgb[2]),
                a: alpha(x, y),
            };
        }
    }
    image
}
