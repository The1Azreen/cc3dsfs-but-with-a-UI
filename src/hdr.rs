//! HDR output (Windows): a fullscreen window drawn with Direct3D 11 into a
//! linear scRGB (FP16) swap chain, so the game can be shown brighter than SDR
//! white. This makes up for the darkening of the screen filter, like the HDR
//! injection of the Morph 4K and RetroTINK 4K.
//!
//! Slint only draws SDR, so this window replaces the Slint capture view while
//! it is shown. It runs on its own thread with its own message loop.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCSTR, s, w};

use crate::capture::frame::Plane;
use crate::layout;
use crate::settings::Settings;

/// scRGB: 1.0 is 80 nits.
const SCRGB_NITS: f32 = 80.0;

pub enum HdrEvent {
    /// Esc or right click in the HDR window.
    OpenMenu,
    /// The window was closed (Alt+F4).
    Closed,
    Error(String),
}

#[derive(Default)]
pub struct HdrFrame {
    pub top: Option<Plane>,
    pub bottom: Option<Plane>,
}

enum Command {
    /// Show fullscreen on the monitor containing this point.
    Show(i32, i32),
    Hide,
    Frame(Arc<HdrFrame>),
    Settings(Box<Settings>),
    Quit,
}

pub struct HdrOutput {
    tx: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl HdrOutput {
    pub fn new(events: impl Fn(HdrEvent) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("hdr-output".into())
            .spawn(move || {
                if let Err(e) = run(rx, &events) {
                    events(HdrEvent::Error(e));
                }
            })
            .expect("failed to spawn HDR thread");
        Self { tx, thread: Some(thread) }
    }

    pub fn show(&self, x: i32, y: i32) {
        let _ = self.tx.send(Command::Show(x, y));
    }

    pub fn hide(&self) {
        let _ = self.tx.send(Command::Hide);
    }

    pub fn frame(&self, frame: Arc<HdrFrame>) {
        let _ = self.tx.send(Command::Frame(frame));
    }

    pub fn settings(&self, settings: &Settings) {
        let _ = self.tx.send(Command::Settings(Box::new(settings.clone())));
    }
}

impl Drop for HdrOutput {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct DisplayInfo {
    pub hdr_enabled: bool,
    pub max_nits: f32,
}

/// HDR state of the display showing the point (x, y), in desktop pixels.
pub fn display_at(x: i32, y: i32) -> Option<DisplayInfo> {
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    display_for_monitor(monitor)
}

fn display_for_monitor(monitor: HMONITOR) -> Option<DisplayInfo> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut a = 0;
        while let Ok(adapter) = factory.EnumAdapters1(a) {
            let mut o = 0;
            while let Ok(output) = adapter.EnumOutputs(o) {
                if let Ok(desc) = output.cast::<IDXGIOutput6>().and_then(|o6| o6.GetDesc1()) {
                    if desc.Monitor == monitor {
                        return Some(DisplayInfo {
                            hdr_enabled: desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
                            max_nits: desc.MaxLuminance,
                        });
                    }
                }
                o += 1;
            }
            a += 1;
        }
    }
    None
}

/// How much light a filter lets through, averaged over a source pixel, so
/// the brightness can be raised by the inverse. Integrates the same profiles
/// as `hdr.hlsl`, including its gamma-space darkening.
pub fn filter_mean(style: i32, edge: f32) -> f32 {
    if style == 0 || edge <= 0.0 {
        return 1.0;
    }
    let soft = |t: f32| 0.5 + 0.5 * (std::f32::consts::TAU * t).cos();
    let fine = |t: f32| if !(0.1..=0.9).contains(&t) { 1.0 } else { 0.0 };
    const N: usize = 64;
    let mut total = 0.0;
    for i in 0..N {
        for j in 0..N {
            let (tx, ty) = ((i as f32 + 0.5) / N as f32, (j as f32 + 0.5) / N as f32);
            let (dx, dy) = match style {
                1 => (soft(tx), soft(ty)),
                2 => (fine(tx), fine(ty)),
                _ => (0.0, soft(ty)),
            };
            total += ((1.0 - edge * dx) * (1.0 - edge * dy)).powf(2.2);
        }
    }
    total / (N * N) as f32
}

// ---------------------------------------------------------------------------
// Window thread

#[derive(Default)]
struct WindowEvents {
    open_menu: bool,
    closed: bool,
    resized: bool,
}

thread_local! {
    static WINDOW_EVENTS: RefCell<WindowEvents> = RefCell::new(WindowEvents::default());
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let note = |f: fn(&mut WindowEvents)| WINDOW_EVENTS.with(|e| f(&mut e.borrow_mut()));
    match msg {
        WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
            note(|e| e.open_menu = true);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            note(|e| e.open_menu = true);
            LRESULT(0)
        }
        WM_CLOSE => {
            note(|e| e.closed = true);
            LRESULT(0)
        }
        WM_SIZE => {
            note(|e| e.resized = true);
            LRESULT(0)
        }
        // Hide the mouse pointer over the game.
        WM_SETCURSOR => {
            unsafe { SetCursor(None) };
            LRESULT(1)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_window() -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("cc3dsfs_hdr"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_APPWINDOW,
            w!("cc3dsfs_hdr"),
            w!("cc3dsfs (HDR)"),
            WS_POPUP,
            0,
            0,
            640,
            480,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| format!("Could not create the HDR window: {e}"))
    }
}

fn run(rx: Receiver<Command>, events: &dyn Fn(HdrEvent)) -> Result<(), String> {
    let hwnd = create_window()?;
    let mut renderer = Renderer::new(hwnd)?;
    let mut settings = Settings::default();
    let mut frame: Arc<HdrFrame> = Arc::new(HdrFrame::default());
    let mut visible = false;
    let mut peak_nits = 1000.0;
    let mut dirty = false;

    loop {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let window_events = WINDOW_EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));
        if window_events.open_menu {
            events(HdrEvent::OpenMenu);
        }
        if window_events.closed {
            events(HdrEvent::Closed);
        }
        if window_events.resized {
            renderer.resize()?;
            dirty = true;
        }

        // Wait briefly for work; presenting with vsync paces the loop while visible.
        let first = rx.recv_timeout(Duration::from_millis(if visible { 2 } else { 30 }));
        let mut commands: Vec<Command> = match first {
            Ok(c) => vec![c],
            Err(RecvTimeoutError::Timeout) => Vec::new(),
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        commands.extend(rx.try_iter());
        for command in commands {
            match command {
                Command::Show(x, y) => unsafe {
                    let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                    let _ = GetMonitorInfoW(monitor, &mut info);
                    let r: RECT = info.rcMonitor;
                    match display_for_monitor(monitor) {
                        Some(d) if d.hdr_enabled => peak_nits = d.max_nits.max(100.0),
                        _ => events(HdrEvent::Error("This display is not in HDR mode. Turn on \"Use HDR\" in Windows display settings.".into())),
                    }
                    let _ = SetWindowPos(hwnd, Some(HWND_TOP), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_SHOWWINDOW);
                    let _ = SetForegroundWindow(hwnd);
                    visible = true;
                    dirty = true;
                },
                Command::Hide => unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    visible = false;
                },
                Command::Frame(f) => {
                    frame = f;
                    dirty = true;
                }
                Command::Settings(s) => {
                    settings = *s;
                    dirty = true;
                }
                Command::Quit => unsafe {
                    let _ = DestroyWindow(hwnd);
                    return Ok(());
                },
            }
        }

        if visible && dirty {
            renderer.render(&frame, &settings, peak_nits)?;
            dirty = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Direct3D 11

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Params {
    top_rect: [f32; 4],
    bottom_rect: [f32; 4],
    top_size: [f32; 2],
    bottom_size: [f32; 2],
    filter_style: f32,
    edge: f32,
    gain: f32,
    peak: f32,
}

struct ScreenTexture {
    width: usize,
    height: usize,
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
}

struct Renderer {
    hwnd: HWND,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    target: Option<ID3D11RenderTargetView>,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    params: ID3D11Buffer,
    textures: [Option<ScreenTexture>; 2],
    rgba: Vec<u8>,
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob, String> {
    let source = include_str!("hdr.hlsl");
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    let result = unsafe {
        D3DCompile(source.as_ptr().cast(), source.len(), s!("hdr.hlsl"), None, None, entry, target, 0, 0, &mut code, Some(&mut errors))
    };
    if let Err(e) = result {
        let detail = errors
            .map(|b| unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())).into_owned()
            })
            .unwrap_or_default();
        return Err(format!("HDR shader failed to compile: {e} {detail}"));
    }
    code.ok_or_else(|| "HDR shader produced no code".into())
}

fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()) }
}

fn err(what: &str) -> impl Fn(windows::core::Error) -> String + '_ {
    move |e| format!("HDR: {what} failed ({e})")
}

impl Renderer {
    fn new(hwnd: HWND) -> Result<Self, String> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(err("creating the Direct3D device"))?;
            let device: ID3D11Device = device.ok_or("HDR: no Direct3D device")?;
            let context: ID3D11DeviceContext = context.ok_or("HDR: no Direct3D context")?;

            let factory: IDXGIFactory2 = device
                .cast::<IDXGIDevice>()
                .and_then(|d| d.GetAdapter())
                .and_then(|a| a.GetParent())
                .map_err(err("finding the DXGI factory"))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap_chain = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None).map_err(err("creating the swap chain"))?;
            // Linear scRGB: lets Windows show values above SDR white on an HDR display.
            swap_chain
                .cast::<IDXGISwapChain3>()
                .and_then(|s| s.SetColorSpace1(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709))
                .map_err(err("setting the scRGB colour space"))?;
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);

            let vs_code = compile(s!("vs_main"), s!("vs_5_0"))?;
            let ps_code = compile(s!("ps_main"), s!("ps_5_0"))?;
            let mut vertex_shader = None;
            device.CreateVertexShader(blob_bytes(&vs_code), None, Some(&mut vertex_shader)).map_err(err("creating the vertex shader"))?;
            let mut pixel_shader = None;
            device.CreatePixelShader(blob_bytes(&ps_code), None, Some(&mut pixel_shader)).map_err(err("creating the pixel shader"))?;

            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            device.CreateSamplerState(&sampler_desc, Some(&mut sampler)).map_err(err("creating the sampler"))?;

            let buffer_desc = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<Params>() as u32,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                ..Default::default()
            };
            let mut params = None;
            device.CreateBuffer(&buffer_desc, None, Some(&mut params)).map_err(err("creating the constant buffer"))?;

            Ok(Self {
                hwnd,
                device,
                context,
                swap_chain,
                target: None,
                vertex_shader: vertex_shader.unwrap(),
                pixel_shader: pixel_shader.unwrap(),
                sampler: sampler.unwrap(),
                params: params.unwrap(),
                textures: [None, None],
                rgba: Vec::new(),
            })
        }
    }

    fn resize(&mut self) -> Result<(), String> {
        self.target = None;
        unsafe {
            self.swap_chain
                .ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
                .map_err(err("resizing the swap chain"))
        }
    }

    fn output_size(&self) -> (f32, f32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut r);
        }
        ((r.right - r.left).max(1) as f32, (r.bottom - r.top).max(1) as f32)
    }

    /// Upload a screen as an sRGB texture (so sampling returns linear light).
    fn upload(&mut self, slot: usize, plane: &Plane) -> Result<(), String> {
        let reuse = matches!(&self.textures[slot], Some(t) if t.width == plane.width && t.height == plane.height);
        if !reuse {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: plane.width as u32,
                Height: plane.height as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            };
            unsafe {
                let mut texture = None;
                self.device.CreateTexture2D(&desc, None, Some(&mut texture)).map_err(err("creating a screen texture"))?;
                let texture = texture.unwrap();
                let mut view = None;
                self.device.CreateShaderResourceView(&texture, None, Some(&mut view)).map_err(err("creating a texture view"))?;
                self.textures[slot] = Some(ScreenTexture { width: plane.width, height: plane.height, texture, view: view.unwrap() });
            }
        }
        self.rgba.clear();
        self.rgba.reserve(plane.width * plane.height * 4);
        for px in plane.rgb.chunks_exact(3) {
            self.rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        let texture = &self.textures[slot].as_ref().unwrap().texture;
        unsafe {
            self.context.UpdateSubresource(texture, 0, None, self.rgba.as_ptr().cast(), (plane.width * 4) as u32, 0);
        }
        Ok(())
    }

    fn render(&mut self, frame: &HdrFrame, settings: &Settings, peak_nits: f32) -> Result<(), String> {
        let (out_w, out_h) = self.output_size();
        let size = |p: &Option<Plane>| p.as_ref().map(|p| (p.width as f32, p.height as f32));
        let placement = layout::compute(settings, out_w, out_h, size(&frame.top), size(&frame.bottom), false);

        if let Some(top) = &frame.top {
            self.upload(0, top)?;
        }
        if let Some(bottom) = &frame.bottom {
            self.upload(1, bottom)?;
        }

        // Same rule as the Slint filter: no grid below 3 output pixels per source pixel.
        let cell = |p: Option<layout::Placement>, s: Option<(f32, f32)>| match (p, s) {
            (Some(p), Some((w, h))) => (p.w / w).min(p.h / h),
            _ => f32::MAX,
        };
        let smallest_cell = cell(placement.top, size(&frame.top)).min(cell(placement.bottom, size(&frame.bottom)));
        let style = if smallest_cell >= 3.0 { settings.filter_style } else { 0 };
        let edge = settings.filter_strength.clamp(0.0, 100.0) / 100.0 * 0.6;
        let gain = settings.hdr_brightness / SCRGB_NITS / filter_mean(style, edge);

        let rect = |p: Option<layout::Placement>| p.map(|p| [p.x, p.y, p.w, p.h]).unwrap_or_default();
        let params = Params {
            top_rect: rect(placement.top),
            bottom_rect: rect(placement.bottom),
            top_size: size(&frame.top).map(|(w, h)| [w, h]).unwrap_or_default(),
            bottom_size: size(&frame.bottom).map(|(w, h)| [w, h]).unwrap_or_default(),
            filter_style: style as f32,
            edge,
            gain,
            peak: peak_nits / SCRGB_NITS,
        };

        unsafe {
            if self.target.is_none() {
                let buffer: ID3D11Texture2D = self.swap_chain.GetBuffer(0).map_err(err("getting the back buffer"))?;
                let mut target = None;
                self.device.CreateRenderTargetView(&buffer, None, Some(&mut target)).map_err(err("creating the render target"))?;
                self.target = target;
            }
            let target = self.target.clone().unwrap();
            let ctx = &self.context;
            ctx.UpdateSubresource(&self.params, 0, None, (&params as *const Params).cast(), 0, 0);
            ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: out_w, Height: out_h, MinDepth: 0.0, MaxDepth: 1.0 }]));
            ctx.ClearRenderTargetView(&target, &[0.0, 0.0, 0.0, 1.0]);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None);
            ctx.VSSetShader(&self.vertex_shader, None);
            ctx.PSSetShader(&self.pixel_shader, None);
            let views = [
                frame.top.as_ref().and(self.textures[0].as_ref()).map(|t| t.view.clone()),
                frame.bottom.as_ref().and(self.textures[1].as_ref()).map(|t| t.view.clone()),
            ];
            ctx.PSSetShaderResources(0, Some(&views));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));
            ctx.Draw(3, 0);
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok().map_err(err("presenting"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::filter_mean;

    #[test]
    fn filter_mean_bounds() {
        assert_eq!(filter_mean(0, 0.5), 1.0);
        let lcd = filter_mean(1, 0.18);
        assert!(lcd > 0.6 && lcd < 0.9, "{lcd}");
        assert!(filter_mean(1, 0.36) < lcd);
        assert!(filter_mean(3, 0.18) > lcd);
    }
}
