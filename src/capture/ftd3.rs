//! New 3DS XL Loopy boards (FTDI FT601).
//!
//! Works through FTDI's D3XX driver (the Windows default) or WinUSB.
//! For WinUSB, the FT601 command protocol is spoken directly, as in
//! `3dscapture_ftd3_libusb_comms.cpp`.

use std::time::{Duration, Instant};

use nusb::transfer::{Buffer, Bulk, ControlIn, ControlType, In, Out, Recipient};
use nusb::{Endpoint, Interface, MaybeFuture};

use super::frame::{self, VIDEO_SIZE_3DS_2D, VIDEO_SIZE_3DS_3D};
use super::ftdi_ffi::{self, D3xx, FT_FLAGS_SUPERSPEED, FT_IO_PENDING, FT_OK, FT_OPEN_BY_SERIAL_NUMBER, FtHandle, Overlapped};
use super::{BoardKind, DeviceInfo, Driver, Output, Session};

const BULK_OUT: u8 = 0x02;
const BULK_IN: u8 = 0x82;
const CFG_WAIT_MS: u64 = 200;
const CFG_3D_SCREEN_POS: usize = 3;
const CONCURRENT_BUFFERS: usize = 5;

const VALID_DESCRIPTIONS: [&str; 2] = ["N3DSXL", "N3DSXL.2"];

const SAMPLES_IN: usize = 1096 * 16;
const EXTRA_BUFFER: usize = 1024;
const ERROR_BUFFER: usize = 1024;

fn video_in_size(is_3d: bool) -> usize {
    if is_3d { VIDEO_SIZE_3DS_3D } else { VIDEO_SIZE_3DS_2D }
}

/// Mirrors `sizeof(FTD3_3DSCaptureReceived[_3D]) & ~(1024 - 1)`.
fn capture_size(is_3d: bool) -> usize {
    let full = video_in_size(is_3d) + SAMPLES_IN * 2 + EXTRA_BUFFER + ERROR_BUFFER;
    let aligned = full.div_ceil(16) * 16;
    aligned & !(EXTRA_BUFFER - 1)
}

pub fn list_devices() -> Vec<DeviceInfo> {
    let mut out = Vec::new();
    let mut extra_id = 0;
    let mut serial_or_id = |serial: &str| {
        if serial.is_empty() {
            extra_id += 1;
            (extra_id - 1).to_string()
        } else {
            serial.to_string()
        }
    };

    if let Some(d3xx) = D3xx::get() {
        for (flags, _ty, serial, desc) in ftdi_ffi::list_ftdi_devices(d3xx.FT_CreateDeviceInfoList, d3xx.FT_GetDeviceInfoDetail) {
            if VALID_DESCRIPTIONS.contains(&desc.as_str()) {
                out.push(DeviceInfo {
                    board: BoardKind::New3dsXl,
                    driver: Driver::FtdiD3xx,
                    serial: serial_or_id(&serial),
                    superspeed: flags & FT_FLAGS_SUPERSPEED != 0,
                    firmware_id: 0,
                });
            }
        }
    }

    for dev in winusb_candidates() {
        out.push(DeviceInfo {
            board: BoardKind::New3dsXl,
            driver: Driver::WinUsb,
            serial: serial_or_id(dev.serial_number().unwrap_or("")),
            superspeed: matches!(dev.speed(), Some(nusb::Speed::Super | nusb::Speed::SuperPlus)) || dev.usb_version() >= 0x300,
            firmware_id: 0,
        });
    }
    out
}

const FTDI_VID: u16 = 0x0403;
const VALID_PIDS: [u16; 7] = [0x601e, 0x601f, 0x602a, 0x602b, 0x602c, 0x602d, 0x602f];

fn winusb_candidates() -> Vec<nusb::DeviceInfo> {
    let Ok(devices) = nusb::list_devices().wait() else { return Vec::new() };
    devices
        .filter(|d| d.vendor_id() == FTDI_VID && VALID_PIDS.contains(&d.product_id()))
        .filter(|d| d.driver().is_some_and(|drv| drv.eq_ignore_ascii_case("winusb")))
        .filter(|d| d.product_string().is_some_and(|p| VALID_DESCRIPTIONS.contains(&p)))
        .collect()
}

/// Pipe-level operations shared by both drivers.
trait Ftd3Io {
    fn write_pipe(&mut self, pipe: u8, data: &[u8]) -> Result<usize, String>;
    fn read_pipe(&mut self, pipe: u8, buf: &mut [u8], timeout_ms: Option<u32>) -> Result<usize, String>;
    fn abort_pipe(&mut self, pipe: u8) -> Result<(), String>;
    fn set_stream_pipe(&mut self, pipe: u8, len: usize) -> Result<(), String>;
}

enum Io {
    D3xx(D3xxIo),
    WinUsb(WinUsbIo),
}

impl Io {
    fn is_driver(&self) -> bool {
        matches!(self, Io::D3xx(_))
    }

    fn inner(&mut self) -> &mut dyn Ftd3Io {
        match self {
            Io::D3xx(io) => io,
            Io::WinUsb(io) => io,
        }
    }
}

impl Ftd3Io for Io {
    fn write_pipe(&mut self, pipe: u8, data: &[u8]) -> Result<usize, String> {
        self.inner().write_pipe(pipe, data)
    }

    fn read_pipe(&mut self, pipe: u8, buf: &mut [u8], timeout_ms: Option<u32>) -> Result<usize, String> {
        self.inner().read_pipe(pipe, buf, timeout_ms)
    }

    fn abort_pipe(&mut self, pipe: u8) -> Result<(), String> {
        self.inner().abort_pipe(pipe)
    }

    fn set_stream_pipe(&mut self, pipe: u8, len: usize) -> Result<(), String> {
        self.inner().set_stream_pipe(pipe, len)
    }
}

pub fn open(info: &DeviceInfo) -> Result<Box<dyn Session>, String> {
    match info.driver {
        Driver::FtdiD3xx => {
            // The board sometimes stops answering; drain it on a throwaway handle first.
            {
                let mut io = D3xxIo::open(&info.serial).map_err(|e| format!("Create failed ({e})"))?;
                drain_data(&mut io);
                let _ = io.abort_pipe(BULK_IN);
            }
            let mut io = Io::D3xx(D3xxIo::open(&info.serial).map_err(|e| format!("Second create failed ({e})"))?);
            let has_3d = configure(&mut io)?;
            Ok(Box::new(Ftd3Session { io, has_3d, superspeed: info.superspeed }))
        }
        _ => {
            {
                let mut io = WinUsbIo::open(&info.serial)?;
                drain_data(&mut io);
                let _ = io.abort_pipe(BULK_IN);
            }
            let mut io = Io::WinUsb(WinUsbIo::open(&info.serial)?);
            let has_3d = configure(&mut io)?;
            Ok(Box::new(Ftd3Session { io, has_3d, superspeed: info.superspeed }))
        }
    }
}

fn set_spi_access(io: &mut dyn Ftd3Io, enable: bool) -> Result<(), String> {
    let buf = [0x40, if enable { 0x80 } else { 0x00 }, 0x00, 0x00];
    io.write_pipe(BULK_OUT, &buf).map(|_| ()).map_err(|e| format!("Setting SPI access failed ({e})"))
}

/// Try draining frames without safety checks. Fixes boards that stop answering.
fn drain_data(io: &mut dyn Ftd3Io) {
    let _ = set_spi_access(io, true);
    let mut buf = vec![0u8; 0x100000];
    let start = Instant::now();
    let _ = io.read_pipe(BULK_IN, &mut buf, Some(CFG_WAIT_MS as u32));
    let elapsed = start.elapsed();
    if elapsed < Duration::from_millis(CFG_WAIT_MS) {
        std::thread::sleep(Duration::from_millis(CFG_WAIT_MS + 1) - elapsed);
    }
}

/// Connection setup from `connect_ftd3`. Returns whether the board reports 3D support.
fn configure(io: &mut dyn Ftd3Io) -> Result<bool, String> {
    // Firmware info query. Not needed, but the original sequence does it.
    set_spi_access(io, true)?;
    io.write_pipe(BULK_OUT, &[0x80, 0x01, 0xAB, 0x00])?;
    io.write_pipe(BULK_OUT, &[0x90, 0x08, 0x03, 0x02, 0x00, 0x00, 0x00, 0x00])?;
    let mut in_buf = [0u8; 0x10];
    io.read_pipe(BULK_IN, &mut in_buf, None)?;
    io.write_pipe(BULK_OUT, &[0x80, 0x01, 0xAB, 0x00])?;
    set_spi_access(io, false)?;

    load_firmware(io, 1, false)?;

    // Read the 3D capability flag.
    set_spi_access(io, true)?;
    io.write_pipe(BULK_OUT, &[0x98, 0x05, 0x9F, 0x00])?;
    let mut cfg = [0u8; 0x10];
    let read = io.read_pipe(BULK_IN, &mut cfg, None)?;
    let has_3d = !(read >= CFG_3D_SCREEN_POS && cfg[CFG_3D_SCREEN_POS] != 0xc1);
    set_spi_access(io, false)?;
    Ok(has_3d)
}

fn load_firmware(io: &mut dyn Ftd3Io, firmware_id: u8, is_switch: bool) -> Result<(), String> {
    set_spi_access(io, true)?;
    io.write_pipe(BULK_OUT, &[0x42 + firmware_id.min(1), 0x00, 0x00, 0x00])?;
    std::thread::sleep(Duration::from_millis(CFG_WAIT_MS));
    if is_switch {
        // Needed for audio to come back after switching firmware.
        set_spi_access(io, true)?;
    }
    set_spi_access(io, false)
}

struct Ftd3Session {
    io: Io,
    has_3d: bool,
    superspeed: bool,
}

impl Ftd3Session {
    fn can_3d_now(&self) -> bool {
        self.has_3d && self.superspeed
    }

    /// Port of `ftd3_capture_3d_setup`.
    fn setup_3d(&mut self, first_pass: bool, stored_3d: &mut bool, requested: bool) -> Result<(), String> {
        let can_3d = self.can_3d_now();
        if first_pass {
            *stored_3d = true;
        }
        if !first_pass && (*stored_3d == requested || !can_3d) {
            *stored_3d = requested;
            return Ok(());
        }
        if !first_pass {
            self.io.abort_pipe(BULK_IN).map_err(|e| format!("Abort failed ({e})"))?;
        }
        let enabled = can_3d && requested;
        if self.has_3d && *stored_3d != enabled {
            load_firmware(&mut self.io, enabled as u8, true)?;
        }
        let size = capture_size(enabled || (first_pass && self.io.is_driver()));
        self.io.set_stream_pipe(BULK_IN, size).map_err(|e| format!("Stream failed ({e})"))?;
        self.io.abort_pipe(BULK_IN).map_err(|e| format!("Abort failed ({e})"))?;
        self.io.set_stream_pipe(BULK_IN, size).map_err(|e| format!("Stream failed ({e})"))?;
        *stored_3d = requested;
        Ok(())
    }
}

/// Validates a transfer and hands the decoded frame to the UI. Port of `data_output_update`.
fn output_buffer(out: &Output, data: &[u8], mut is_3d: bool) {
    let read = data.len();
    if is_3d && read < video_in_size(true) && read >= video_in_size(false) {
        is_3d = false;
    }
    // Some games (Kirby Planet Robobot, M&L: Dream Team) produce bad frames.
    if read > capture_size(is_3d) - ERROR_BUFFER || read < video_in_size(is_3d) {
        return;
    }
    let video_size = video_in_size(is_3d);
    let frame = if is_3d { frame::ftd3_3d(data) } else { frame::ftd3_2d(data) };
    let audio = frame::audio_after_video(data, video_size, SAMPLES_IN);
    out.push(frame, &audio);
}

impl Session for Ftd3Session {
    fn can_3d(&self) -> bool {
        self.can_3d_now()
    }

    fn run(&mut self, out: &Output) -> Result<(), String> {
        let could_use_3d = self.can_3d_now();
        let mut stored_3d = true;
        self.setup_3d(true, &mut stored_3d, out.wants_3d())?;
        let result = match &self.io {
            Io::D3xx(io) => {
                let (lib, handle) = (io.lib, io.handle);
                self.run_d3xx(lib, handle, out, could_use_3d, &mut stored_3d)
            }
            Io::WinUsb(_) => self.run_winusb(out, could_use_3d, &mut stored_3d),
        };
        let _ = self.io.abort_pipe(BULK_IN);
        result
    }
}

// ---------------------------------------------------------------------------
// D3XX driver

struct D3xxIo {
    lib: &'static D3xx,
    handle: FtHandle,
}

unsafe impl Send for D3xxIo {}

impl D3xxIo {
    fn open(serial: &str) -> Result<Self, String> {
        let lib = D3xx::get().ok_or("FTD3XX.dll not found")?;
        let mut arg = ftdi_ffi::serial_arg(serial);
        let mut handle: FtHandle = std::ptr::null_mut();
        let status = unsafe { (lib.FT_Create)(arg.as_mut_ptr().cast(), FT_OPEN_BY_SERIAL_NUMBER, &mut handle) };
        if status != FT_OK || handle.is_null() {
            return Err(format!("FT_Create error {status}"));
        }
        Ok(Self { lib, handle })
    }

    fn with_timeout<T>(&mut self, pipe: u8, timeout_ms: Option<u32>, f: impl FnOnce(&mut Self) -> T) -> T {
        let mut old = 0u32;
        if let Some(t) = timeout_ms {
            unsafe {
                (self.lib.FT_GetPipeTimeout)(self.handle, pipe, &mut old);
                (self.lib.FT_SetPipeTimeout)(self.handle, pipe, t);
            }
        }
        let result = f(self);
        if timeout_ms.is_some() {
            unsafe { (self.lib.FT_SetPipeTimeout)(self.handle, pipe, old) };
        }
        result
    }
}

impl Drop for D3xxIo {
    fn drop(&mut self) {
        unsafe { (self.lib.FT_Close)(self.handle) };
    }
}

impl Ftd3Io for D3xxIo {
    fn write_pipe(&mut self, pipe: u8, data: &[u8]) -> Result<usize, String> {
        let mut transferred = 0u32;
        let mut copy = data.to_vec();
        let status = unsafe {
            (self.lib.FT_WritePipe)(self.handle, pipe, copy.as_mut_ptr(), copy.len() as u32, &mut transferred, std::ptr::null_mut())
        };
        if status != FT_OK {
            return Err(format!("write error {status}"));
        }
        Ok(transferred as usize)
    }

    fn read_pipe(&mut self, pipe: u8, buf: &mut [u8], timeout_ms: Option<u32>) -> Result<usize, String> {
        self.with_timeout(pipe, timeout_ms, |io| {
            let mut transferred = 0u32;
            let status = unsafe {
                (io.lib.FT_ReadPipe)(io.handle, pipe, buf.as_mut_ptr(), buf.len() as u32, &mut transferred, std::ptr::null_mut())
            };
            if status != FT_OK {
                return Err(format!("read error {status}"));
            }
            Ok(transferred as usize)
        })
    }

    fn abort_pipe(&mut self, pipe: u8) -> Result<(), String> {
        match unsafe { (self.lib.FT_AbortPipe)(self.handle, pipe) } {
            FT_OK => Ok(()),
            s => Err(format!("error {s}")),
        }
    }

    fn set_stream_pipe(&mut self, pipe: u8, len: usize) -> Result<(), String> {
        match unsafe { (self.lib.FT_SetStreamPipe)(self.handle, 0, 0, pipe, len as u32) } {
            FT_OK => Ok(()),
            s => Err(format!("error {s}")),
        }
    }
}

struct OverlappedReads {
    lib: &'static D3xx,
    handle: FtHandle,
    overlapped: Box<[Overlapped; CONCURRENT_BUFFERS]>,
    buffers: Vec<Vec<u8>>,
    read: [u32; CONCURRENT_BUFFERS],
    is_3d: [bool; CONCURRENT_BUFFERS],
    pending: [bool; CONCURRENT_BUFFERS],
}

impl OverlappedReads {
    fn new(lib: &'static D3xx, handle: FtHandle) -> Result<Self, String> {
        let mut overlapped: Box<[Overlapped; CONCURRENT_BUFFERS]> = Box::new(Default::default());
        for ov in overlapped.iter_mut() {
            if unsafe { (lib.FT_InitializeOverlapped)(handle, ov) } != FT_OK {
                return Err("Disconnected: Initialize failed".into());
            }
        }
        Ok(Self {
            lib,
            handle,
            overlapped,
            buffers: (0..CONCURRENT_BUFFERS).map(|_| vec![0u8; capture_size(true)]).collect(),
            read: [0; CONCURRENT_BUFFERS],
            is_3d: [false; CONCURRENT_BUFFERS],
            pending: [false; CONCURRENT_BUFFERS],
        })
    }

    fn submit(&mut self, i: usize, is_3d: bool) -> Result<(), String> {
        self.is_3d[i] = is_3d;
        let status = unsafe {
            (self.lib.FT_ReadPipeEx)(
                self.handle,
                BULK_IN,
                self.buffers[i].as_mut_ptr(),
                capture_size(is_3d) as u32,
                &mut self.read[i],
                &mut self.overlapped[i],
            )
        };
        if status != FT_IO_PENDING {
            return Err("Disconnected: Read failed".into());
        }
        self.pending[i] = true;
        Ok(())
    }

    fn wait(&mut self, i: usize) -> Result<(), String> {
        if !self.pending[i] {
            return Ok(());
        }
        self.pending[i] = false;
        let status = unsafe { (self.lib.FT_GetOverlappedResult)(self.handle, &mut self.overlapped[i], &mut self.read[i], 1) };
        if status != FT_OK {
            return Err("Disconnected: USB error".into());
        }
        Ok(())
    }

    fn wait_all(&mut self) -> Result<(), String> {
        let mut result = Ok(());
        for i in 0..CONCURRENT_BUFFERS {
            if let Err(e) = self.wait(i) {
                result = Err(e);
            }
        }
        result
    }
}

impl Drop for OverlappedReads {
    fn drop(&mut self) {
        unsafe { (self.lib.FT_AbortPipe)(self.handle, BULK_IN) };
        let _ = self.wait_all();
        for ov in self.overlapped.iter_mut() {
            unsafe { (self.lib.FT_ReleaseOverlapped)(self.handle, ov) };
        }
    }
}

impl Ftd3Session {
    /// Port of `fast_capture_call`: keeps 4 overlapped reads in flight.
    fn run_d3xx(&mut self, lib: &'static D3xx, handle: FtHandle, out: &Output, could_use_3d: bool, stored_3d: &mut bool) -> Result<(), String> {
        let mut reads = OverlappedReads::new(lib, handle)?;
        for i in 0..CONCURRENT_BUFFERS - 1 {
            reads.submit(i, could_use_3d && *stored_3d)?;
        }
        let mut curr = CONCURRENT_BUFFERS - 1;

        while out.keep_running() {
            if could_use_3d && *stored_3d != out.wants_3d() {
                reads.wait_all()?;
                self.setup_3d(false, stored_3d, out.wants_3d())?;
                for i in 0..CONCURRENT_BUFFERS - 1 {
                    reads.submit(i, could_use_3d && *stored_3d)?;
                }
                curr = CONCURRENT_BUFFERS - 1;
            }
            reads.submit(curr, could_use_3d && *stored_3d)?;
            curr = (curr + 1) % CONCURRENT_BUFFERS;
            reads.wait(curr)?;
            let len = reads.read[curr] as usize;
            output_buffer(out, &reads.buffers[curr][..len], reads.is_3d[curr]);
        }
        Ok(())
    }

    fn ep_in(&mut self) -> &mut Endpoint<Bulk, In> {
        match &mut self.io {
            Io::WinUsb(io) => &mut io.ep_in,
            Io::D3xx(_) => unreachable!("WinUSB capture loop on a D3XX handle"),
        }
    }

    /// Port of `ftd3_libusb_capture_main_loop`, using nusb's transfer queue.
    fn run_winusb(&mut self, out: &Output, could_use_3d: bool, stored_3d: &mut bool) -> Result<(), String> {
        let submit_all = |ep: &mut Endpoint<Bulk, In>, is_3d: bool| {
            for _ in 0..CONCURRENT_BUFFERS {
                ep.submit(Buffer::new(capture_size(is_3d)));
            }
        };
        let mut is_3d = could_use_3d && *stored_3d;
        submit_all(self.ep_in(), is_3d);

        let result = loop {
            if !out.keep_running() {
                break Ok(());
            }
            if could_use_3d && *stored_3d != out.wants_3d() {
                drain_pending(self.ep_in());
                self.setup_3d(false, stored_3d, out.wants_3d())?;
                is_3d = could_use_3d && *stored_3d;
                submit_all(self.ep_in(), is_3d);
            }
            let ep = self.ep_in();
            let Some(done) = ep.wait_next_complete(Duration::from_millis(500)) else {
                break Err("Disconnected: Read timed out".into());
            };
            if done.status.is_err() {
                break Err("Disconnected: Read failed".into());
            }
            output_buffer(out, &done.buffer[..done.actual_len], is_3d);
            let mut buf = done.buffer;
            buf.clear();
            buf.set_requested_len(capture_size(is_3d));
            ep.submit(buf);
        };
        drain_pending(self.ep_in());
        result
    }
}

fn drain_pending(ep: &mut Endpoint<Bulk, In>) {
    ep.cancel_all();
    while ep.pending() > 0 {
        if ep.wait_next_complete(Duration::from_secs(1)).is_none() {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// WinUSB: speak the FT601 command protocol directly.

const COMMAND_PIPE: u8 = 0x01;
const CREATE_PIPE_ID: u8 = 0x82;
const COMMAND_TIMEOUT: Duration = Duration::from_millis(500);
const CMD_ABORT: u8 = 0;
const CMD_RW_PREPARE: u8 = 1;
const CMD_SET_STREAM_PIPE: u8 = 2;
const CMD_CREATE: u8 = 3;
const CMD_DESTROY: u8 = 3;

struct WinUsbIo {
    _interfaces: Vec<Interface>,
    ep_cmd: Endpoint<Bulk, Out>,
    ep_out: Endpoint<Bulk, Out>,
    ep_in: Endpoint<Bulk, In>,
    cmd_id: u32,
}

fn find_endpoint<D: nusb::transfer::EndpointDirection>(interfaces: &[Interface], addr: u8) -> Result<Endpoint<Bulk, D>, String> {
    interfaces
        .iter()
        .find_map(|i| i.endpoint::<Bulk, D>(addr).ok())
        .ok_or_else(|| format!("endpoint {addr:#04x} not found"))
}

impl WinUsbIo {
    fn open(serial: &str) -> Result<Self, String> {
        let candidates = winusb_candidates();
        let mut extra_id = 0;
        let dev_info = candidates
            .into_iter()
            .find(|d| {
                let s = match d.serial_number() {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => {
                        extra_id += 1;
                        (extra_id - 1).to_string()
                    }
                };
                s == serial
            })
            .ok_or("Device not found")?;
        let device = dev_info.open().wait().map_err(|e| format!("Open failed ({e})"))?;
        let cmd_intf = device.claim_interface(0).wait().map_err(|e| format!("Claim failed ({e})"))?;

        for index in [0x8000u16, 0x8400] {
            let data = cmd_intf
                .control_in(
                    ControlIn { control_type: ControlType::Vendor, recipient: Recipient::Device, request: 3, value: 1, index, length: 4 },
                    COMMAND_TIMEOUT,
                )
                .wait()
                .map_err(|e| format!("Setup failed ({e})"))?;
            if data.len() != 4 {
                return Err("Setup failed (short read)".into());
            }
        }

        let bulk_intf = device.claim_interface(1).wait().map_err(|e| format!("Claim failed ({e})"))?;
        let interfaces = vec![cmd_intf, bulk_intf];
        let mut io = Self {
            ep_cmd: find_endpoint(&interfaces, COMMAND_PIPE)?,
            ep_out: find_endpoint(&interfaces, BULK_OUT)?,
            ep_in: find_endpoint(&interfaces, BULK_IN)?,
            _interfaces: interfaces,
            cmd_id: 0,
        };
        io.send_ptr_command(CREATE_PIPE_ID, CMD_ABORT)?;
        io.send_ptr_command(CREATE_PIPE_ID, CMD_CREATE)?;
        Ok(io)
    }

    fn send_command(&mut self, data: Vec<u8>, timeout: Duration) -> Result<(), String> {
        let len = data.len();
        let done = self.ep_cmd.transfer_blocking(data.into(), timeout);
        done.status.map_err(|e| e.to_string())?;
        if done.actual_len != len {
            return Err("short command write".into());
        }
        Ok(())
    }

    fn preamble(&mut self, pipe: u8, command: u8) -> Vec<u8> {
        let mut data = Vec::with_capacity(20);
        data.extend_from_slice(&self.cmd_id.to_le_bytes());
        self.cmd_id = self.cmd_id.wrapping_add(1);
        data.extend_from_slice(&[pipe, command, 0, 0]);
        data
    }

    fn send_ptr_command(&mut self, pipe: u8, command: u8) -> Result<(), String> {
        let mut data = self.preamble(pipe, command);
        data.extend_from_slice(&[0u8; 12]);
        self.send_command(data, COMMAND_TIMEOUT)
    }

    fn send_len_command(&mut self, pipe: u8, command: u8, length: u32, timeout: Duration) -> Result<(), String> {
        let mut data = self.preamble(pipe, command);
        data.extend_from_slice(&length.to_le_bytes());
        data.extend_from_slice(&[0u8; 8]);
        self.send_command(data, timeout)
    }
}

impl Ftd3Io for WinUsbIo {
    fn write_pipe(&mut self, pipe: u8, data: &[u8]) -> Result<usize, String> {
        self.send_len_command(pipe, CMD_RW_PREPARE, data.len() as u32, COMMAND_TIMEOUT)?;
        let done = self.ep_out.transfer_blocking(data.to_vec().into(), COMMAND_TIMEOUT);
        done.status.map_err(|e| e.to_string())?;
        Ok(done.actual_len)
    }

    fn read_pipe(&mut self, pipe: u8, buf: &mut [u8], timeout_ms: Option<u32>) -> Result<usize, String> {
        let timeout = timeout_ms.map(|t| Duration::from_millis(t as u64)).unwrap_or(COMMAND_TIMEOUT);
        self.send_len_command(pipe, CMD_RW_PREPARE, buf.len() as u32, timeout)?;
        // WinUSB wants whole packets; the device still stops at the length we announced.
        let mps = self.ep_in.max_packet_size();
        let done = self.ep_in.transfer_blocking(Buffer::new(buf.len().div_ceil(mps) * mps), timeout);
        done.status.map_err(|e| e.to_string())?;
        let n = done.actual_len.min(buf.len());
        buf[..n].copy_from_slice(&done.buffer[..n]);
        Ok(n)
    }

    fn abort_pipe(&mut self, pipe: u8) -> Result<(), String> {
        self.send_ptr_command(pipe, CMD_ABORT)?;
        self.send_ptr_command(pipe, CMD_ABORT)?;
        self.send_ptr_command(pipe, CMD_DESTROY)
    }

    fn set_stream_pipe(&mut self, pipe: u8, len: usize) -> Result<(), String> {
        self.send_len_command(pipe, CMD_SET_STREAM_PIPE, len as u32, COMMAND_TIMEOUT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_sizes_match_original_structs() {
        // sizeof(FTD3_3DSCaptureReceived[_3D]) & ~1023 in the C++ code.
        assert_eq!(capture_size(false), 555_008);
        assert_eq!(capture_size(true), 842_752);
    }
}
