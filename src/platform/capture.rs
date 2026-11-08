//! Opt-in, event-driven desktop backdrop capture for physically refracted glass.
//!
//! Frames stay in memory and are cropped to the overlay canvas before CPU readback.
//! The caller must exclude the overlay HWND with WDA_EXCLUDEFROMCAPTURE before
//! starting, otherwise the captured island would recursively refract itself.
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
            GraphicsCaptureSession,
        },
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::{HMODULE, POINT},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
            Direct3D11::*,
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
                IDXGIDevice,
            },
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint},
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
            RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize,
        },
    },
    core::{IInspectable, Interface, factory},
};

#[derive(Clone, Debug)]
pub struct BackdropFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed, unpremultiplied RGBA8 in physical-pixel canvas coordinates.
    pub pixels: Vec<u8>,
}

/// A live capture session exists only while Glass is selected. Windows supplies
/// FrameArrived events; no timer, desktop polling, files, or network are used.
pub struct CaptureService {
    stop: Sender<()>,
    worker: Option<JoinHandle<()>>,
}
impl CaptureService {
    pub fn start(
        rect: (i32, i32, u32, u32),
        monitor_handle: isize,
        on_frame: impl Fn(BackdropFrame) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        if rect.2 == 0 || rect.3 == 0 || rect.2 > 4096 || rect.3 > 4096 {
            return Err("Invalid glass backdrop capture size".into());
        }
        let (stop, stopped) = mpsc::channel();
        let (ready, initialization) = mpsc::sync_channel(1);
        let callback: Arc<dyn Fn(BackdropFrame) + Send + Sync> = Arc::new(on_frame);
        let worker = thread::Builder::new()
            .name("island-backdrop".into())
            .spawn(move || {
                // SAFETY: this new worker has no COM apartment. Every WinRT resource
                // is released before the matching RoUninitialize on this thread.
                if let Err(error) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                    let _ = ready.send(Err(format!(
                        "Backdrop WinRT initialization failed: {error}"
                    )));
                    return;
                }
                match Session::new(rect, monitor_handle, callback) {
                    Ok(session) => {
                        let _ = ready.send(Ok(()));
                        let _ = stopped.recv();
                        drop(session);
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
                // SAFETY: paired with successful RoInitialize on this exact thread.
                unsafe {
                    RoUninitialize();
                }
            })
            .map_err(|e| format!("Could not start backdrop capture: {e}"))?;
        match initialization.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = stop.send(());
                drop(worker);
                Err(format!(
                    "Backdrop capture initialization timed out: {error}"
                ))
            }
        }
    }
}
impl Drop for CaptureService {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Session {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    handler: Option<i64>,
}
impl Session {
    fn new(
        rect: (i32, i32, u32, u32),
        monitor_handle: isize,
        notify: Arc<dyn Fn(BackdropFrame) + Send + Sync>,
    ) -> Result<Self, String> {
        let setup = || -> windows::core::Result<Self> {
            if !GraphicsCaptureSession::IsSupported()? {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004001u32 as i32,
                )));
            }
            let mut monitor_info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: initialized monitor buffer and an actual primary-monitor handle.
            let monitor = if monitor_handle == 0 {
                unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
            } else {
                windows::Win32::Graphics::Gdi::HMONITOR(monitor_handle as *mut _)
            };
            unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.ok()?;
            let left = rect.0 - monitor_info.rcMonitor.left;
            let top = rect.1 - monitor_info.rcMonitor.top;
            if left < 0
                || top < 0
                || left as u64 + rect.2 as u64
                    > (monitor_info.rcMonitor.right - monitor_info.rcMonitor.left) as u64
                || top as u64 + rect.3 as u64
                    > (monitor_info.rcMonitor.bottom - monitor_info.rcMonitor.top) as u64
            {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80070057u32 as i32,
                )));
            }
            let mut device = None;
            let mut context = None;
            // SAFETY: D3D output pointers refer to initialized stack Options.
            // BGRA support is required by the WinRT capture-surface bridge.
            // Only cropped copies use this device; optical rendering is bounded
            // separately. Prefer the lower-memory device by default and retain
            // a hardware override for measured driver comparisons.
            unsafe {
                let create =
                    |driver,
                     device: &mut Option<ID3D11Device>,
                     context: &mut Option<ID3D11DeviceContext>| {
                        D3D11CreateDevice(
                            None,
                            driver,
                            HMODULE::default(),
                            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                            None,
                            D3D11_SDK_VERSION,
                            Some(device),
                            None,
                            Some(context),
                        )
                    };
                let hardware = std::env::args().any(|arg| arg == "--capture-hardware");
                let preferred = if hardware {
                    D3D_DRIVER_TYPE_HARDWARE
                } else {
                    D3D_DRIVER_TYPE_WARP
                };
                let fallback = if hardware {
                    D3D_DRIVER_TYPE_WARP
                } else {
                    D3D_DRIVER_TYPE_HARDWARE
                };
                if create(preferred, &mut device, &mut context).is_err() {
                    device = None;
                    context = None;
                    create(fallback, &mut device, &mut context)?;
                }
            }
            let device = device.ok_or_else(windows::core::Error::empty)?;
            let context = context.ok_or_else(windows::core::Error::empty)?;
            let dxgi: IDXGIDevice = device.cast()?;
            // SAFETY: dxgi belongs to the live BGRA-compatible D3D11 device.
            let winrt_device: IDirect3DDevice =
                unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
            let interop: IGraphicsCaptureItemInterop =
                factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            // SAFETY: CreateForMonitor receives the primary monitor handle. This
            // Win32 interop API requires no picker or permission dialog.
            let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor)? };
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                1,
                windows::Graphics::SizeInt32 {
                    Width: left + rect.2 as i32,
                    Height: top + rect.3 as i32,
                },
            )?;
            let session = pool.CreateCaptureSession(&item)?;
            let mut resources = Self {
                pool,
                session,
                handler: None,
            };
            resources.session.SetIsCursorCaptureEnabled(false)?;
            // No consent request or security setting is changed. If system policy
            // requires a capture border, this feature is unavailable and falls back.
            resources.session.SetIsBorderRequired(false)?;
            if resources.session.IsBorderRequired()? {
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80070005u32 as i32,
                )));
            }
            // Capture only the upper-left monitor extent containing the canvas;
            // a complete monitor-sized backing texture is unnecessary for this ROI.
            // Optional Windows 11 throttle: at most 30 backdrop frames/s, only
            // when Windows raises a frame event. Older builds may lack this property.
            let _ = resources
                .session
                .SetMinUpdateInterval(TimeSpan { Duration: 333_333 });
            let _ = resources.session.SetDirtyRegionMode(
                windows::Graphics::Capture::GraphicsCaptureDirtyRegionMode::ReportOnly,
            );
            let mut staging = None;
            let description = D3D11_TEXTURE2D_DESC {
                Width: rect.2,
                Height: rect.3,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            // SAFETY: valid single-subresource CPU-read staging texture descriptor.
            unsafe {
                device.CreateTexture2D(&description, None, Some(&mut staging))?;
            }
            let readback = Arc::new(Mutex::new(Readback {
                _device: device,
                context,
                staging: staging.ok_or_else(windows::core::Error::empty)?,
                left: left as u32,
                top: top as u32,
                width: rect.2,
                height: rect.3,
                scratch: vec![0; rect.2 as usize * rect.3 as usize * 4],
                last: Vec::new(),
                last_read: None,
            }));
            let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                move |pool, _| {
                    let Some(pool) = pool.as_ref() else {
                        return Ok(());
                    };
                    let Ok(frame) = pool.TryGetNextFrame() else {
                        return Ok(());
                    };
                    let frame = FrameLease(frame);
                    let image = readback
                        .lock()
                        .ok()
                        .and_then(|mut state| state.read(&frame.0).ok().flatten());
                    // Return the frame before invoking application code. The callback
                    // may enqueue a GUI event but must not block this WGC worker.
                    drop(frame);
                    if let Some(image) = image {
                        notify(image);
                    }
                    Ok(())
                },
            );
            resources.handler = Some(resources.pool.FrameArrived(&handler)?);
            resources.session.StartCapture()?;
            Ok(resources)
        };
        setup().map_err(|error| format!("Live glass backdrop capture unavailable (borderless capture/graphics support required): {error}"))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            let _ = self.pool.RemoveFrameArrived(handler);
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
struct FrameLease(Direct3D11CaptureFrame);
impl Drop for FrameLease {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
struct Readback {
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    staging: ID3D11Texture2D,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    scratch: Vec<u8>,
    last: Vec<u8>,
    last_read: Option<Instant>,
}
impl Readback {
    fn read(
        &mut self,
        frame: &Direct3D11CaptureFrame,
    ) -> windows::core::Result<Option<BackdropFrame>> {
        let size = frame.ContentSize()?;
        if self.left + self.width > size.Width.max(0) as u32
            || self.top + self.height > size.Height.max(0) as u32
        {
            return Ok(None);
        }
        if !self.last.is_empty()
            && let Ok(regions) = frame.DirtyRegions()
        {
            let mut overlaps = false;
            for i in 0..regions.Size()? {
                let region = regions.GetAt(i)?;
                if region.X < (self.left + self.width) as i32
                    && region.Y < (self.top + self.height) as i32
                    && region.X + region.Width > self.left as i32
                    && region.Y + region.Height > self.top as i32
                {
                    overlaps = true;
                    break;
                }
            }
            if !overlaps {
                return Ok(None);
            }
        }
        // Maintain the same 30fps ceiling on older Windows builds without the
        // optional MinUpdateInterval property. This checks frame events; it never
        // schedules a wakeup and does not affect the independent morph timer.
        if self
            .last_read
            .is_some_and(|last| last.elapsed() < Duration::from_millis(33))
        {
            return Ok(None);
        }
        self.last_read = Some(Instant::now());
        let surface = frame.Surface()?;
        let interop: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        // SAFETY: the capture surface's DXGI interface is a texture valid only
        // until FrameLease closes it. No reference escapes this synchronous read.
        let texture: ID3D11Texture2D = unsafe { interop.GetInterface()? };
        let mut texture_description = D3D11_TEXTURE2D_DESC::default();
        // ContentSize can lead the backing texture during display reconfiguration.
        // Check the actual resource too, so CopySubresourceRegion stays in bounds.
        unsafe {
            texture.GetDesc(&mut texture_description);
        }
        if self.left + self.width > texture_description.Width
            || self.top + self.height > texture_description.Height
        {
            return Ok(None);
        }
        let source_box = D3D11_BOX {
            left: self.left,
            top: self.top,
            front: 0,
            right: self.left + self.width,
            bottom: self.top + self.height,
            back: 1,
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: a mutex serializes this immediate context, the source box is
        // within ContentSize, and staging dimensions exactly match the crop.
        unsafe {
            self.context.CopySubresourceRegion(
                &self.staging,
                0,
                0,
                0,
                0,
                &texture,
                0,
                Some(&source_box),
            );
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let row_bytes = self.width as usize * 4;
            if mapped.pData.is_null() || mapped.RowPitch < row_bytes as u32 {
                self.context.Unmap(&self.staging, 0);
                return Ok(None);
            }
            // A successful Map guarantees RowPitch*Height readable bytes until
            // Unmap. Only Width*4 bytes of each padded row are inspected.
            for y in 0..self.height as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    row_bytes,
                );
                let output = &mut self.scratch[y * row_bytes..(y + 1) * row_bytes];
                for (bgra, rgba) in row
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(output.as_chunks_mut::<4>().0)
                {
                    rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
                }
            }
            self.context.Unmap(&self.staging, 0);
        }
        // Ignore tiny compositor/noise changes instead of rebuilding the entire
        // optical image for a handful of pixels. Compare with the last accepted
        // frame, so larger slow changes accumulate until they become visible.
        if self.last.len() == self.scratch.len() {
            let substantial_pixels = self
                .last
                .as_chunks::<4>()
                .0
                .iter()
                .zip(self.scratch.as_chunks::<4>().0)
                .filter(|(old, new)| (0..3).any(|channel| old[channel].abs_diff(new[channel]) > 3))
                .take(32)
                .count();
            if substantial_pixels < 32 {
                return Ok(None);
            }
        }
        self.last.clone_from(&self.scratch);
        Ok(Some(BackdropFrame {
            width: self.width,
            height: self.height,
            pixels: self.scratch.clone(),
        }))
    }
}
