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
    stop: Sender<bool>,
    worker: Option<JoinHandle<()>>,
}
impl CaptureService {
    pub fn start(
        rect: (i32, i32, u32, u32),
        monitor_handle: isize,
        on_frame: impl Fn(BackdropFrame) + Send + Sync + 'static,
        on_error: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        if rect.2 == 0 || rect.3 == 0 || rect.2 > 4096 || rect.3 > 4096 {
            return Err("Invalid glass backdrop capture size".into());
        }
        let (stop, stopped) = mpsc::channel();
        let wake = stop.clone();

        let callback: Arc<dyn Fn(BackdropFrame) + Send + Sync> = Arc::new(on_frame);
        let worker = thread::Builder::new()
            .name("island-backdrop".into())
            .spawn(move || {
                // SAFETY: this new worker has no COM apartment. Every WinRT resource
                // is released before the matching RoUninitialize on this thread.
                if let Err(error) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                    on_error(format!("Backdrop WinRT initialization failed: {error}"));
                    return;
                }
                match Session::new(rect, monitor_handle, callback, wake) {
                    Ok(session) => {
                        loop {
                            let pending = session.readback.as_ref().is_some_and(|r| {
                                r.lock()
                                    .is_ok_and(|r| r.slots.iter().any(|s| s.pending.is_some()))
                            });
                            let stopped_now = if pending {
                                matches!(
                                    stopped.recv_timeout(Duration::from_millis(2)),
                                    Ok(true) | Err(mpsc::RecvTimeoutError::Disconnected)
                                )
                            } else {
                                !matches!(stopped.recv(), Ok(false))
                            };
                            if stopped_now {
                                break;
                            }
                            if let Some(readback) = &session.readback
                                && let Ok(mut state) = readback.lock()
                            {
                                match state.poll() {
                                    Ok(Some(image)) => {
                                        if let Some(notify) = &session.notify {
                                            notify(image);
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        on_error(format!("Backdrop device interrupted: {error}"));
                                        break;
                                    }
                                }
                            }
                        }
                        drop(session);
                    }
                    Err(error) => {
                        on_error(error);
                    }
                }
                // SAFETY: paired with successful RoInitialize on this exact thread.
                unsafe {
                    RoUninitialize();
                }
            })
            .map_err(|e| format!("Could not start backdrop capture: {e}"))?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for CaptureService {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(worker) = self.worker.take() {
            // The capture worker owns teardown; the UI never waits for its GPU.
            drop(worker);
        }
    }
}

struct Session {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    handler: Option<i64>,
    readback: Option<Arc<Mutex<Readback>>>,
    notify: Option<Arc<dyn Fn(BackdropFrame) + Send + Sync>>,
}
impl Session {
    fn new(
        rect: (i32, i32, u32, u32),
        monitor_handle: isize,
        notify: Arc<dyn Fn(BackdropFrame) + Send + Sync>,
        wake: Sender<bool>,
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
            // Hardware capture is preferred; software is an event-driven fallback.
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
                let preferred = D3D_DRIVER_TYPE_HARDWARE;
                let fallback = D3D_DRIVER_TYPE_WARP;
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
                readback: None,
                notify: None,
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
            let mut mip = 0u32;
            while (rect.2 >> mip).max(1) as u64 * (rect.3 >> mip).max(1) as u64 > 262_144 {
                mip += 1;
            }
            let width = (rect.2 >> mip).max(1);
            let height = (rect.3 >> mip).max(1);
            let mut roi = None;
            let description = D3D11_TEXTURE2D_DESC {
                Width: rect.2,
                Height: rect.3,
                MipLevels: mip + 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
                ..Default::default()
            };
            unsafe {
                device.CreateTexture2D(&description, None, Some(&mut roi))?;
            }
            let roi = roi.ok_or_else(windows::core::Error::empty)?;
            let mut view = None;
            unsafe {
                device.CreateShaderResourceView(&roi, None, Some(&mut view))?;
            }
            let view = view.ok_or_else(windows::core::Error::empty)?;
            let mut slots = Vec::with_capacity(3);
            for _ in 0..3 {
                let mut texture = None;
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    ..Default::default()
                };
                unsafe {
                    device.CreateTexture2D(&desc, None, Some(&mut texture))?;
                }
                slots.push(Slot {
                    texture: texture.ok_or_else(windows::core::Error::empty)?,
                    pending: None,
                });
            }
            let readback = Arc::new(Mutex::new(Readback {
                _device: device,
                context,
                roi,
                view,
                mip,
                slots,
                left: left as u32,
                top: top as u32,
                crop_width: rect.2,
                crop_height: rect.3,
                width,
                height,
                last: Vec::new(),
                last_read: None,
                serial: 0,
                delivered: 0,
                failure: None,
            }));
            resources.readback = Some(readback.clone());
            resources.notify = Some(notify.clone());
            let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                move |pool, _| {
                    let Some(pool) = pool.as_ref() else {
                        return Ok(());
                    };
                    let Ok(frame) = pool.TryGetNextFrame() else {
                        return Ok(());
                    };
                    let frame = FrameLease(frame);
                    let image =
                        readback
                            .lock()
                            .ok()
                            .and_then(|mut state| match state.read(&frame.0) {
                                Ok(image) => image,
                                Err(error) => {
                                    state.failure = Some(error);
                                    None
                                }
                            });
                    let _ = wake.send(false);
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
struct Slot {
    texture: ID3D11Texture2D,
    pending: Option<(u64, Instant)>,
}
struct Readback {
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    roi: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    mip: u32,
    slots: Vec<Slot>,
    left: u32,
    top: u32,
    crop_width: u32,
    crop_height: u32,
    width: u32,
    height: u32,
    last: Vec<u8>,
    last_read: Option<Instant>,
    serial: u64,
    delivered: u64,
    failure: Option<windows::core::Error>,
}
impl Readback {
    /// Nonblocking staging maps are confined to the capture worker. A final queued
    /// frame is drained by a finite worker wakeup even when WGC stops raising events.
    fn poll(&mut self) -> windows::core::Result<Option<BackdropFrame>> {
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        let mut newest = None;
        for slot in &mut self.slots {
            let Some((serial, queued)) = slot.pending else {
                continue;
            };
            if queued.elapsed() > Duration::from_millis(500) {
                slot.pending = None;
                return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x800705B4u32 as i32,
                )));
            }
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            let result = unsafe {
                self.context.Map(
                    &slot.texture,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                )
            };
            if let Err(error) = result {
                if error.code() == windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING {
                    continue;
                }
                slot.pending = None;
                return Err(error);
            }
            slot.pending = None;
            if serial > self.delivered
                && !mapped.pData.is_null()
                && mapped.RowPitch >= self.width * 4
            {
                let mut pixels = vec![0; self.width as usize * self.height as usize * 4];
                for y in 0..self.height as usize {
                    let row = unsafe {
                        std::slice::from_raw_parts(
                            (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                            self.width as usize * 4,
                        )
                    };
                    for (bgra, rgba) in row.as_chunks::<4>().0.iter().zip(
                        pixels[y * self.width as usize * 4..(y + 1) * self.width as usize * 4]
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut(),
                    ) {
                        rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
                    }
                }
                self.delivered = serial;
                newest = Some(pixels);
            }
            unsafe {
                self.context.Unmap(&slot.texture, 0);
            }
        }
        let Some(pixels) = newest else {
            return Ok(None);
        };
        // A bounded digest on the capture worker avoids any UI-thread pixel comparison.
        if self.last.len() == pixels.len()
            && self
                .last
                .as_chunks::<4>()
                .0
                .iter()
                .step_by(7)
                .zip(pixels.as_chunks::<4>().0.iter().step_by(7))
                .all(|(a, b)| (0..3).all(|i| a[i].abs_diff(b[i]) <= 2))
        {
            return Ok(None);
        }
        self.last.clone_from(&pixels);
        Ok(Some(BackdropFrame {
            width: self.width,
            height: self.height,
            pixels,
        }))
    }
    fn read(
        &mut self,
        frame: &Direct3D11CaptureFrame,
    ) -> windows::core::Result<Option<BackdropFrame>> {
        let completed = self.poll()?;
        if self
            .last_read
            .is_some_and(|last| last.elapsed() < Duration::from_millis(33))
        {
            return Ok(completed);
        }
        if self.serial > 0
            && let Ok(regions) = frame.DirtyRegions()
        {
            let mut overlap = false;
            for i in 0..regions.Size()? {
                let r = regions.GetAt(i)?;
                overlap |= r.X < (self.left + self.crop_width) as i32
                    && r.Y < (self.top + self.crop_height) as i32
                    && r.X + r.Width > self.left as i32
                    && r.Y + r.Height > self.top as i32;
            }
            if !overlap {
                return Ok(completed);
            }
        }
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.pending.is_none()) else {
            return Ok(completed);
        };
        let surface = frame.Surface()?;
        let interop: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let texture: ID3D11Texture2D = unsafe { interop.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut desc);
        }
        if self.left + self.crop_width > desc.Width || self.top + self.crop_height > desc.Height {
            return Ok(completed);
        }
        let source = D3D11_BOX {
            left: self.left,
            top: self.top,
            front: 0,
            right: self.left + self.crop_width,
            bottom: self.top + self.crop_height,
            back: 1,
        };
        unsafe {
            self.context
                .CopySubresourceRegion(&self.roi, 0, 0, 0, 0, &texture, 0, Some(&source));
            if self.mip > 0 {
                self.context.GenerateMips(&self.view);
            }
            self.context.CopySubresourceRegion(
                &slot.texture,
                0,
                0,
                0,
                0,
                &self.roi,
                self.mip,
                None,
            );
            self.context.Flush();
        }
        self.serial += 1;
        slot.pending = Some((self.serial, Instant::now()));
        self.last_read = Some(Instant::now());
        Ok(completed)
    }
}
