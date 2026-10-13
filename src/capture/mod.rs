//! Device discovery and the capture thread for Loopy capture boards.

#[cfg(debug_assertions)]
mod demo;
pub mod frame;
mod ftd2;
mod ftd3;
mod ftdi_ffi;
mod legacy_usb;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::audio::AudioQueue;
use frame::Frame;

/// Every Loopy board family supported on Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardKind {
    /// 2020+ New 3DS XL boards (FTDI FT601).
    New3dsXl,
    /// Older 3DS boards (Cypress, 16D0:06A3).
    Old3ds,
    /// Older DS boards (Cypress, 16D0:0647).
    OldDs,
    /// 2024 DS boards (FTDI FT232H, firmware uploaded on connect).
    NewDs,
}

impl BoardKind {
    pub fn is_3ds(self) -> bool {
        matches!(self, BoardKind::New3dsXl | BoardKind::Old3ds)
    }

    pub fn label(self) -> &'static str {
        match self {
            BoardKind::New3dsXl => "New 3DS XL",
            BoardKind::Old3ds => "3DS",
            BoardKind::OldDs => "DS",
            BoardKind::NewDs => "DS (2024)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    /// FTDI's D3XX driver (Windows only).
    #[cfg_attr(not(windows), allow(dead_code))]
    FtdiD3xx,
    FtdiD2xx,
    /// Direct USB access: WinUSB on Windows, usbfs on Linux.
    Usb,
}

impl Driver {
    pub fn label(self) -> &'static str {
        match self {
            Driver::FtdiD3xx => "FTDI D3XX",
            Driver::FtdiD2xx => "FTDI D2XX",
            Driver::Usb if cfg!(windows) => "WinUSB",
            Driver::Usb => "USB",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub board: BoardKind,
    pub driver: Driver,
    pub serial: String,
    /// USB 3 link, required for 3D capture on New 3DS XL boards.
    pub superspeed: bool,
    /// Which FPGA bitstream a new DS board needs (1 or 2).
    pub firmware_id: u8,
}

impl DeviceInfo {
    pub fn display_name(&self) -> String {
        format!("{} · {}", self.board.label(), self.serial)
    }
}

/// Claim a USB interface, detaching any kernel driver first on Linux.
fn claim_interface(device: &nusb::Device, interface: u8) -> Result<nusb::Interface, nusb::Error> {
    use nusb::MaybeFuture;
    if cfg!(target_os = "linux") {
        device.detach_and_claim_interface(interface).wait()
    } else {
        device.claim_interface(interface).wait()
    }
}

pub fn list_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::new();
    #[cfg(debug_assertions)]
    if std::env::var_os("CC3DSFS_DEMO").is_some() {
        devices.push(demo::info());
    }
    devices.extend(ftd3::list_devices());
    devices.extend(ftd2::list_devices());
    devices.extend(legacy_usb::list_devices());
    devices
}

/// What a running capture session needs from the outside world.
pub struct Output {
    shared: Arc<Shared>,
    audio: Arc<AudioQueue>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Output {
    pub fn keep_running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    pub fn wants_3d(&self) -> bool {
        self.shared.request_3d.load(Ordering::Relaxed)
    }

    pub fn push(&self, frame: Frame, audio: &[i16]) {
        if !audio.is_empty() {
            self.audio.push(audio);
        }
        *self.shared.latest.lock().unwrap() = Some(frame);
        if !self.shared.frame_pending.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }
}

/// A connected board. `run` blocks until stopped or the device errors out.
pub trait Session: Send {
    fn run(&mut self, out: &Output) -> Result<(), String>;
    /// Whether this board/link can switch to 3D capture.
    fn can_3d(&self) -> bool {
        false
    }
}

fn open(info: &DeviceInfo) -> Result<Box<dyn Session>, String> {
    #[cfg(debug_assertions)]
    if info.serial == demo::SERIAL {
        return Ok(Box::new(demo::DemoSession));
    }
    match info.board {
        BoardKind::New3dsXl => ftd3::open(info),
        BoardKind::NewDs => ftd2::open(info),
        BoardKind::Old3ds | BoardKind::OldDs => legacy_usb::open(info),
    }
}

pub enum Event {
    Devices(Vec<DeviceInfo>),
    Connecting(DeviceInfo),
    Connected { info: DeviceInfo, can_3d: bool },
    Disconnected(Option<String>),
    /// A new frame is ready; fetch it with [`CaptureManager::take_frame`].
    Frame,
}

struct Shared {
    running: AtomicBool,
    request_3d: AtomicBool,
    want_connected: AtomicBool,
    wanted_serial: Mutex<Option<String>>,
    latest: Mutex<Option<Frame>>,
    frame_pending: AtomicBool,
}

enum Command {
    Wake,
    Shutdown,
}

pub struct CaptureManager {
    shared: Arc<Shared>,
    tx: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureManager {
    pub fn new(
        audio: Arc<AudioQueue>,
        auto_connect: bool,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            running: AtomicBool::new(false),
            request_3d: AtomicBool::new(false),
            want_connected: AtomicBool::new(auto_connect),
            wanted_serial: Mutex::new(None),
            latest: Mutex::new(None),
            frame_pending: AtomicBool::new(false),
        });
        let (tx, rx) = mpsc::channel();
        let events: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(events);
        let worker_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("capture".into())
            .spawn(move || worker(worker_shared, rx, audio, events))
            .expect("failed to spawn capture thread");
        Self { shared, tx, thread: Some(thread) }
    }

    /// Connect to `serial`, or to the first board found when `None`.
    /// Keeps retrying (and reconnects after unplugging) until [`Self::disconnect`].
    pub fn connect(&self, serial: Option<String>) {
        *self.shared.wanted_serial.lock().unwrap() = serial;
        self.shared.want_connected.store(true, Ordering::Relaxed);
        self.shared.running.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Command::Wake);
    }

    pub fn disconnect(&self) {
        self.shared.want_connected.store(false, Ordering::Relaxed);
        self.shared.running.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Command::Wake);
    }

    pub fn set_3d(&self, enabled: bool) {
        self.shared.request_3d.store(enabled, Ordering::Relaxed);
    }

    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.frame_pending.store(false, Ordering::Release);
        self.shared.latest.lock().unwrap().take()
    }
}

impl Drop for CaptureManager {
    fn drop(&mut self) {
        self.shared.want_connected.store(false, Ordering::Relaxed);
        self.shared.running.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn worker(shared: Arc<Shared>, rx: Receiver<Command>, audio: Arc<AudioQueue>, events: Arc<dyn Fn(Event) + Send + Sync>) {
    let notify_events = events.clone();
    let output = Output { shared: shared.clone(), audio, notify: Arc::new(move || notify_events(Event::Frame)) };
    let mut last_listed: Option<Vec<DeviceInfo>> = None;

    loop {
        let devices = list_devices();
        if last_listed.as_ref() != Some(&devices) {
            events(Event::Devices(devices.clone()));
            last_listed = Some(devices.clone());
        }

        if shared.want_connected.load(Ordering::Relaxed) {
            let wanted = shared.wanted_serial.lock().unwrap().clone();
            let target = match &wanted {
                Some(serial) => devices.iter().find(|d| &d.serial == serial),
                None => devices.first(),
            };
            if let Some(info) = target.cloned() {
                events(Event::Connecting(info.clone()));
                match open(&info) {
                    Ok(mut session) => {
                        shared.running.store(true, Ordering::Relaxed);
                        events(Event::Connected { info: info.clone(), can_3d: session.can_3d() });
                        let result = session.run(&output);
                        shared.running.store(false, Ordering::Relaxed);
                        drop(session);
                        output.audio.clear();
                        events(Event::Disconnected(result.err()));
                        // Re-list right away so the UI reflects unplugged boards.
                        last_listed = None;
                        continue;
                    }
                    Err(err) => events(Event::Disconnected(Some(format!("{}: {err}", info.display_name())))),
                }
            }
        }

        match rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Command::Wake) | Err(RecvTimeoutError::Timeout) => {}
        }
    }
}
