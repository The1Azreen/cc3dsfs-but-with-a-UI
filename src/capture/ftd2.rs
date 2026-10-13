//! 2024 Loopy DS boards (FTDI FT232H) via FTDI's D2XX driver.
//! Port of `dscapture_ftd2_shared.cpp` and the driver acquisition loop.
//! Based on sample code provided by Loopy.

use std::ffi::c_void;
use std::time::Duration;

use super::frame::{self, VIDEO_SIZE_DS};
use super::ftdi_ffi::{self, D2xx, FT_DEVICE_232H, FT_FLAGS_HISPEED, FT_OK, FT_OPEN_BY_SERIAL_NUMBER, FtHandle, FtStatus};
use super::{BoardKind, DeviceInfo, Driver, Output, Session};

static FIRMWARE: [&[u8]; 2] = [
    include_bytes!("../../firmware/ftd2_ds2_fw_1.bin"),
    include_bytes!("../../firmware/ftd2_ds2_fw_2.bin"),
];
const VALID_DESCRIPTIONS: [(&str, u8); 2] = [("NDS.1", 1), ("NDS.2", 2)];

const SYNCH: u16 = 0x4321;
const LIMIT_SIZE_TRANSFER: u32 = 0x10000;
const CLKRATE: u16 = 6;
const CLKDIV: u16 = (30 / CLKRATE) - 1;
const RESETDELAY: u16 = (CLKRATE) / 8;
const INITDELAY: u16 = (1200 * CLKRATE) / 8;
const TX_SPI_SIZE: usize = 1 << 16;

const DS_SAMPLES_IN: usize = 1096;
const MAX_PACKET_SIZE_FTD2: usize = 512 - 2;
const EXTRA_DATA_BUFFER_USB_SIZE: usize = 512;
/// sizeof(FTD2OldDSCaptureReceived)
const RECEIVED_SIZE: usize = (VIDEO_SIZE_DS + DS_SAMPLES_IN * 2 + EXTRA_DATA_BUFFER_USB_SIZE).div_ceil(16) * 16;
/// Bytes per read: a whole number of FTDI packets.
const CAPTURE_SIZE: usize = (RECEIVED_SIZE - EXTRA_DATA_BUFFER_USB_SIZE).div_ceil(MAX_PACKET_SIZE_FTD2) * MAX_PACKET_SIZE_FTD2;
/// The last 4 bytes should always be the sync pattern.
const MAX_SAMPLES: usize = ((CAPTURE_SIZE - VIDEO_SIZE_DS) / 2) - 4;
/// sizeof(FTD2OldDSCaptureReceivedRaw), used to size the driver's USB transfers.
const RAW_SIZE: usize = (RECEIVED_SIZE + RECEIVED_SIZE.div_ceil(MAX_PACKET_SIZE_FTD2) * 2).div_ceil(16) * 16;

const fn le16(x: u16) -> [u8; 2] {
    x.to_le_bytes()
}

pub fn list_devices() -> Vec<DeviceInfo> {
    let Some(d2xx) = D2xx::get() else { return Vec::new() };
    ftdi_ffi::list_ftdi_devices(d2xx.FT_CreateDeviceInfoList, d2xx.FT_GetDeviceInfoDetail)
        .into_iter()
        .filter(|(flags, ty, _, _)| flags & FT_FLAGS_HISPEED != 0 && *ty == FT_DEVICE_232H)
        .filter_map(|(_, _, serial, desc)| {
            let (_, fw) = VALID_DESCRIPTIONS.iter().find(|(d, _)| *d == desc)?;
            Some(DeviceInfo { board: BoardKind::NewDs, driver: Driver::FtdiD2xx, serial, superspeed: false, firmware_id: *fw })
        })
        .collect()
}

struct Ftd2 {
    lib: &'static D2xx,
    handle: FtHandle,
}

unsafe impl Send for Ftd2 {}

fn check(status: FtStatus, what: &str) -> Result<(), String> {
    if status == FT_OK { Ok(()) } else { Err(format!("{what} (error {status})")) }
}

impl Ftd2 {
    fn queue_size(&self) -> Result<u32, String> {
        let mut n = 0u32;
        check(unsafe { (self.lib.FT_GetQueueStatus)(self.handle, &mut n) }, "Queue status failed")?;
        Ok(n)
    }

    fn write(&self, data: &[u8]) -> Result<usize, String> {
        let mut sent = 0u32;
        let status = unsafe { (self.lib.FT_Write)(self.handle, data.as_ptr() as *mut c_void, data.len() as u32, &mut sent) };
        check(status, "Write failed")?;
        Ok(sent as usize)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize, String> {
        let mut got = 0u32;
        let status = unsafe { (self.lib.FT_Read)(self.handle, buf.as_mut_ptr().cast(), buf.len() as u32, &mut got) };
        check(status, "Read failed")?;
        Ok(got as usize)
    }

    fn write_all(&self, data: &[u8]) -> Result<(), String> {
        if self.write(data)? != data.len() {
            return Err("Short write".into());
        }
        Ok(())
    }

    /// Write only when the read queue is empty (otherwise MPSSE is in a bad state).
    fn full_write(&self, data: &[u8]) -> Result<(), String> {
        if self.queue_size()? != 0 {
            return Err("Unexpected data in queue".into());
        }
        self.write_all(data)
    }

    fn set_timeouts(&self, read_ms: u32, write_ms: u32) -> Result<(), String> {
        check(unsafe { (self.lib.FT_SetTimeouts)(self.handle, read_ms, write_ms) }, "Timeouts setup failed")
    }

    fn set_usb_parameters(&self, size_in: u32, size_out: u32) -> Result<(), String> {
        check(unsafe { (self.lib.FT_SetUSBParameters)(self.handle, size_in, size_out) }, "USB parameters failed")
    }

    fn set_bitmode(&self, mode: u8) -> Result<(), String> {
        check(unsafe { (self.lib.FT_SetBitMode)(self.handle, 0x00, mode) }, "Bitmode setup failed")
    }

    /// Verify CDONE matches `want_active`.
    fn check_cdone(&self, want_active: bool) -> Result<(), String> {
        self.full_write(&[0x81, 0x83])?;
        let mut buf = [0u8; 2];
        if self.read(&mut buf)? != 2 || buf[0] == 0xFA {
            return Err("CDONE read failed".into());
        }
        if ((buf[0] & 0x40) != 0) != want_active {
            return Err("CDONE has the wrong state".into());
        }
        Ok(())
    }

    fn spi_tx(&self, data: &[u8]) -> Result<(), String> {
        if self.queue_size()? != 0 {
            return Err("Unexpected data in queue".into());
        }
        let mut tx = Vec::with_capacity(TX_SPI_SIZE + 3);
        for chunk in data.chunks(TX_SPI_SIZE) {
            tx.clear();
            let n = (chunk.len() - 1) as u16;
            tx.extend_from_slice(&[0x11, (n & 0xFF) as u8, (n >> 8) as u8]);
            tx.extend_from_slice(chunk);
            self.write_all(&tx)?;
            if self.queue_size()? != 0 {
                return Err("Unexpected data in queue".into());
            }
        }
        Ok(())
    }

    /// Lattice FPGA programming over MPSSE SPI.
    fn fpga_config(&self, bitstream: &[u8]) -> Result<(), String> {
        let mut cmd0 = vec![
            0x82, 0x7f, 0x80, // set Cx pins {val,dir:1=out}: reset=0
            0x80, 0xf0, 0x0b, // set Dx pins: SS=0, clk idle low, TDI out, TCK out
            0x8f,
        ];
        cmd0.extend_from_slice(&le16(RESETDELAY));
        cmd0.extend_from_slice(&[0x82, 0xff, 0x00, 0x8f]); // reset=PU
        cmd0.extend_from_slice(&le16(INITDELAY));
        cmd0.extend_from_slice(&[0x80, 0xf8, 0x0b, 0x8f]); // SS=1
        cmd0.extend_from_slice(&le16(1)); // 16 dummy clocks
        cmd0.extend_from_slice(&[0x80, 0xf0, 0x0b]); // SS=0

        self.set_timeouts(300, 300)?;
        self.full_write(&cmd0)?;
        self.check_cdone(false)?;
        self.spi_tx(bitstream)?;

        let mut cmd1 = vec![0x80, 0xf8, 0x0b, 0x8f]; // SS=1
        cmd1.extend_from_slice(&le16(20)); // >150 dummy clocks
        cmd1.extend_from_slice(&[0x80, 0x00, 0x00, 0x82, 0x00, 0x00]); // float Dx, Cx
        self.full_write(&cmd1)?;
        self.check_cdone(true)
    }

    fn init_mpsse(&self) -> Result<(), String> {
        let mut cmd = vec![0x85, 0x8d, 0x8a, 0x97, 0x86];
        cmd.extend_from_slice(&le16(CLKDIV));

        check(unsafe { (self.lib.FT_ResetDevice)(self.handle) }, "Reset failed")?;
        self.set_usb_parameters(LIMIT_SIZE_TRANSFER, LIMIT_SIZE_TRANSFER)?;
        check(unsafe { (self.lib.FT_SetChars)(self.handle, 0, 0, 0, 0) }, "SetChars failed")?;
        self.set_timeouts(300, 300)?;
        // Time before an incomplete packet is sent. 2 sometimes makes MPSSE reads fail.
        check(unsafe { (self.lib.FT_SetLatencyTimer)(self.handle, 3) }, "Latency timer failed")?;
        check(unsafe { (self.lib.FT_SetFlowControl)(self.handle, ftdi_ffi::FT_FLOW_RTS_CTS, 0, 0) }, "Flow control failed")?;
        self.set_bitmode(ftdi_ffi::FT_BITMODE_RESET)?;
        self.set_bitmode(ftdi_ffi::FT_BITMODE_MPSSE)?;

        // Bigger transfers improve the frame rate; fall back to 64k if unsupported.
        let size_in = (RAW_SIZE as u32).div_ceil(LIMIT_SIZE_TRANSFER) * LIMIT_SIZE_TRANSFER;
        if self.set_usb_parameters(size_in, LIMIT_SIZE_TRANSFER).is_err() {
            self.set_usb_parameters(LIMIT_SIZE_TRANSFER, LIMIT_SIZE_TRANSFER)?;
        }

        // MPSSE sometimes chokes on the first write: send, purge, resend.
        self.write(&cmd[..1])?;
        std::thread::sleep(Duration::from_millis(20));
        check(unsafe { (self.lib.FT_Purge)(self.handle, ftdi_ffi::FT_PURGE_RX | ftdi_ffi::FT_PURGE_TX) }, "Purge failed")?;
        self.full_write(&cmd)
    }
}

impl Drop for Ftd2 {
    fn drop(&mut self) {
        unsafe { (self.lib.FT_Close)(self.handle) };
    }
}

pub fn open(info: &DeviceInfo) -> Result<Box<dyn Session>, String> {
    let lib = D2xx::get().ok_or("ftd2xx.dll not found. Install FTDI's D2XX (CDM) driver.")?;
    let mut serial = ftdi_ffi::serial_arg(&info.serial);
    let mut handle: FtHandle = std::ptr::null_mut();
    check(unsafe { (lib.FT_OpenEx)(serial.as_mut_ptr().cast(), FT_OPEN_BY_SERIAL_NUMBER, &mut handle) }, "Create failed")?;
    let dev = Ftd2 { lib, handle };

    let setup = || -> Result<(), String> {
        dev.init_mpsse().map_err(|e| format!("MPSSE init failed: {e}"))?;
        let fw = FIRMWARE[(info.firmware_id.clamp(1, 2) - 1) as usize];
        dev.fpga_config(fw).map_err(|e| format!("FPGA config failed: {e}"))?;
        let mut val = 0u16;
        let status = unsafe { (lib.FT_ReadEE)(handle, 1, &mut val) };
        if status != FT_OK || val != 0x0403 {
            return Err("EEPROM read error".into());
        }
        // FIFO mode takes over port B.
        dev.set_bitmode(ftdi_ffi::FT_BITMODE_SYNC_FIFO)?;
        dev.set_timeouts(50, 50)?;
        if dev.queue_size()? != 0 {
            return Err("Intermediate error".into());
        }
        Ok(())
    };
    if let Err(e) = setup() {
        unsafe { (lib.FT_ResetDevice)(handle) };
        return Err(e);
    }
    Ok(Box::new(Ftd2Session { dev }))
}

struct Ftd2Session {
    dev: Ftd2,
}

fn u16_at(buf: &[u8], word: usize) -> u16 {
    u16::from_le_bytes([buf[word * 2], buf[word * 2 + 1]])
}

/// Returns `None` if the buffer is aligned, otherwise how many bytes of
/// `curr` (from the sync point on) belong to the next frame.
fn synchronization_check(curr: &[u8]) -> Option<usize> {
    let words = curr.len() / 2;
    // A frame always ends with sync padding.
    if u16_at(curr, words - 1) == SYNCH {
        return None;
    }
    let mut samples = 0;
    while samples < words && u16_at(curr, samples) != SYNCH {
        samples += 1;
    }
    while samples < words && u16_at(curr, samples) == SYNCH {
        samples += 1;
    }
    Some(samples * 2)
}

/// Length without the trailing sync padding.
fn remove_synch_from_final_length(buf: &[u8], mut len: usize) -> usize {
    while len >= 2 && u16_at(buf, len / 2 - 1) == SYNCH {
        len -= 2;
    }
    len
}

impl Session for Ftd2Session {
    fn run(&mut self, out: &Output) -> Result<(), String> {
        self.dev.write_all(&[0x80, 0x01]).map_err(|e| format!("Capture enable error: {e}"))?;
        let mut curr = vec![0u8; CAPTURE_SIZE];
        let mut next = vec![0u8; CAPTURE_SIZE];
        // Bytes still missing at the end of `curr`.
        let mut next_size = CAPTURE_SIZE;

        while out.keep_running() {
            let start = CAPTURE_SIZE - next_size;
            let got = self.dev.read(&mut curr[start..]).map_err(|e| format!("Disconnected: {e}"))?;
            if got < next_size {
                continue;
            }
            if let Some(sync_at) = synchronization_check(&curr) {
                // Misaligned: keep the tail (start of the next frame) and read the rest.
                next.fill(0);
                next[..CAPTURE_SIZE - sync_at].copy_from_slice(&curr[sync_at..]);
                std::mem::swap(&mut curr, &mut next);
                next_size = sync_at;
                continue;
            }
            next_size = CAPTURE_SIZE;

            let real_len = remove_synch_from_final_length(&curr, CAPTURE_SIZE);
            let mut offset = 0;
            while offset + 2 <= real_len && u16_at(&curr, offset / 2) == SYNCH {
                offset += 2;
            }
            let data = &curr[offset..real_len];
            if data.len() < VIDEO_SIZE_DS {
                continue;
            }
            let audio = frame::audio_after_video(data, VIDEO_SIZE_DS, MAX_SAMPLES);
            out.push(frame::ds(data), &audio);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_original_structs() {
        assert_eq!(RECEIVED_SIZE, 199_312);
        assert_eq!(CAPTURE_SIZE, 198_900);
        assert_eq!(MAX_SAMPLES, 1_142);
    }

    fn words(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn aligned_buffer_passes_sync_check() {
        assert_eq!(synchronization_check(&words(&[1, 2, 3, SYNCH])), None);
    }

    #[test]
    fn misaligned_buffer_reports_tail() {
        // Data, then padding, then the start of the next frame.
        assert_eq!(synchronization_check(&words(&[1, 2, SYNCH, SYNCH, 7, 8])), Some(8));
    }

    #[test]
    fn trailing_sync_is_trimmed() {
        let buf = words(&[5, 6, SYNCH, SYNCH]);
        assert_eq!(remove_synch_from_final_length(&buf, buf.len()), 4);
    }
}
