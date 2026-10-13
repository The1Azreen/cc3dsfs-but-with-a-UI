//! Older Loopy 3DS and DS boards (Cypress based), via WinUSB.
//! Port of `usb_ds_3ds_capture.cpp`.

use std::time::Duration;

use nusb::transfer::{Buffer, Bulk, ControlOut, ControlType, In, Recipient, TransferError};
use nusb::{Endpoint, Interface, MaybeFuture};

use super::frame::{self, VIDEO_SIZE_3DS_2D, VIDEO_SIZE_DS};
use super::{BoardKind, DeviceInfo, Driver, Output, Session};

const VID: u16 = 0x16D0;
const EP2_IN: u8 = 0x82;
const CAPTURE_INTERFACE: u8 = 0;
const MAX_PACKET: usize = 512;

struct Desc {
    board: BoardKind,
    pid: u16,
    control_timeout: Duration,
    bulk_timeout: Duration,
    cmdout_capture_start: u8,
}

const DESC_3DS: Desc = Desc {
    board: BoardKind::Old3ds,
    pid: 0x06A3,
    control_timeout: Duration::from_millis(30),
    bulk_timeout: Duration::from_millis(50),
    cmdout_capture_start: 0x40,
};

const DESC_DS: Desc = Desc {
    board: BoardKind::OldDs,
    pid: 0x0647,
    control_timeout: Duration::from_millis(500),
    bulk_timeout: Duration::from_millis(500),
    cmdout_capture_start: 0x30,
};

const O3DS_SAMPLES_IN: usize = 1096 * 16;

fn video_size(board: BoardKind) -> usize {
    if board == BoardKind::Old3ds { VIDEO_SIZE_3DS_2D } else { VIDEO_SIZE_DS }
}

/// Bytes requested per frame: video plus (on 3DS) the audio block.
fn capture_size(board: BoardKind) -> usize {
    if board == BoardKind::Old3ds {
        // sizeof(USB3DSCaptureReceived) - EXTRA_DATA_BUFFER_USB_SIZE
        VIDEO_SIZE_3DS_2D + O3DS_SAMPLES_IN * 2
    } else {
        VIDEO_SIZE_DS
    }
}

fn devices() -> Vec<(nusb::DeviceInfo, &'static Desc, String)> {
    let Ok(list) = nusb::list_devices().wait() else { return Vec::new() };
    let mut extra_ids = [0usize; 2];
    let mut out = Vec::new();
    for dev in list {
        if dev.vendor_id() != VID {
            continue;
        }
        let (desc, slot) = match dev.product_id() {
            p if p == DESC_3DS.pid => (&DESC_3DS, 0),
            p if p == DESC_DS.pid => (&DESC_DS, 1),
            _ => continue,
        };
        let serial = match dev.serial_number() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                extra_ids[slot] += 1;
                (extra_ids[slot] - 1).to_string()
            }
        };
        out.push((dev, desc, serial));
    }
    out
}

pub fn list_devices() -> Vec<DeviceInfo> {
    devices()
        .into_iter()
        .map(|(dev, desc, serial)| DeviceInfo {
            board: desc.board,
            driver: Driver::WinUsb,
            serial,
            superspeed: matches!(dev.speed(), Some(nusb::Speed::Super | nusb::Speed::SuperPlus)),
            firmware_id: 0,
        })
        .collect()
}

pub fn open(info: &DeviceInfo) -> Result<Box<dyn Session>, String> {
    let (dev, desc, _) = devices()
        .into_iter()
        .find(|(_, desc, serial)| desc.board == info.board && *serial == info.serial)
        .ok_or("Device not found")?;
    let device = dev.open().wait().map_err(|e| format!("Open failed ({e}). Is the WinUSB driver installed?"))?;
    let interface = device.claim_interface(CAPTURE_INTERFACE).wait().map_err(|e| format!("Interface claim failed ({e})"))?;
    let ep_in = interface.endpoint::<Bulk, In>(EP2_IN).map_err(|e| format!("Endpoint open failed ({e})"))?;
    let session = LegacySession { interface, ep_in, desc };
    if desc.board == BoardKind::Old3ds {
        // Firmware workaround: the first read sometimes times out.
        let _ = session.start_capture();
        std::thread::sleep(desc.bulk_timeout);
    }
    Ok(Box::new(session))
}

struct LegacySession {
    interface: Interface,
    ep_in: Endpoint<Bulk, In>,
    desc: &'static Desc,
}

enum ReadResult {
    Frame(Vec<u8>),
    Skip,
}

impl LegacySession {
    fn start_capture(&self) -> Result<(), TransferError> {
        self.interface
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: self.desc.cmdout_capture_start,
                    value: 0,
                    index: 0,
                    data: &[],
                },
                self.desc.control_timeout,
            )
            .wait()
    }

    /// Port of `capture_read_oldds_3ds`.
    fn read_frame(&mut self) -> Result<ReadResult, String> {
        match self.start_capture() {
            Ok(()) => {}
            Err(TransferError::Cancelled) => return Ok(ReadResult::Skip),
            Err(e) => return Err(format!("Disconnected: {e}")),
        }

        let full = capture_size(self.desc.board);
        let mut data = Vec::with_capacity(full);
        // The board sends until the end of the frame, then zero-length packets.
        loop {
            let want = (full - data.len()).div_ceil(MAX_PACKET) * MAX_PACKET;
            let done = self.ep_in.transfer_blocking(Buffer::new(want), self.desc.bulk_timeout);
            let take = done.actual_len.min(full - data.len());
            data.extend_from_slice(&done.buffer[..take]);
            match done.status {
                Ok(()) => {}
                Err(TransferError::Stall) => {
                    let _ = self.ep_in.clear_halt().wait();
                    return Err("Disconnected: Pipe error".into());
                }
                Err(TransferError::Cancelled) => return Ok(ReadResult::Skip),
                Err(e) => return Err(format!("Disconnected: {e}")),
            }
            if data.len() >= full || done.actual_len == 0 {
                break;
            }
        }
        // Incomplete frames are skipped (SIMPLE_DS_FRAME_SKIP).
        if data.len() < video_size(self.desc.board) {
            return Ok(ReadResult::Skip);
        }
        Ok(ReadResult::Frame(data))
    }
}

impl Session for LegacySession {
    fn run(&mut self, out: &Output) -> Result<(), String> {
        while out.keep_running() {
            let ReadResult::Frame(data) = self.read_frame()? else { continue };
            if self.desc.board == BoardKind::Old3ds {
                let audio = frame::audio_after_video(&data, VIDEO_SIZE_3DS_2D, O3DS_SAMPLES_IN);
                out.push(frame::old_3ds(&data), &audio);
            } else {
                out.push(frame::ds(&data), &[]);
            }
        }
        Ok(())
    }
}
