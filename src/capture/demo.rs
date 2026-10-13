//! Debug-only fake 3DS board (set `CC3DSFS_DEMO=1`), used to check layouts without hardware.
//! Frames go through the real FTD3 decoder.

use std::time::Duration;

use super::frame::{self, COLUMNS_3DS_2D, HEIGHT_3DS};
use super::{BoardKind, DeviceInfo, Driver, Output, Session};

pub const SERIAL: &str = "DEMO";

pub fn info() -> DeviceInfo {
    DeviceInfo { board: BoardKind::New3dsXl, driver: Driver::WinUsb, serial: SERIAL.into(), superspeed: false, firmware_id: 0 }
}

pub struct DemoSession;

impl Session for DemoSession {
    fn run(&mut self, out: &Output) -> Result<(), String> {
        let mut t = 0usize;
        let mut data = vec![0u8; COLUMNS_3DS_2D * HEIGHT_3DS * 3];
        while out.keep_running() {
            for c in 0..COLUMNS_3DS_2D {
                for p in 0..HEIGHT_3DS {
                    let i = (c * HEIGHT_3DS + p) * 3;
                    let checker = ((c / 20) + (p / 20) + t / 8) % 2 == 0;
                    data[i] = if checker { 230 } else { (c * 255 / COLUMNS_3DS_2D) as u8 };
                    data[i + 1] = (p * 255 / HEIGHT_3DS) as u8;
                    data[i + 2] = if c % 2 == 0 { 200 } else { 60 };
                }
            }
            out.push(frame::ftd3_2d(&data), &[]);
            t += 1;
            std::thread::sleep(Duration::from_millis(16));
        }
        Ok(())
    }
}
