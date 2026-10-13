//! Runtime bindings to FTDI's D3XX (FT601, New 3DS XL boards, Windows only)
//! and D2XX (FT232H, new DS boards; `libftd2xx.so` on Linux) drivers.
//!
//! The DLLs are loaded lazily so the application still starts (and the
//! WinUSB backends keep working) when a driver is not installed.

use std::ffi::c_void;
use std::sync::OnceLock;

use libloading::Library;

pub type FtHandle = *mut c_void;
pub type FtStatus = u32;

pub const FT_OK: FtStatus = 0;
#[cfg(windows)]
pub const FT_IO_PENDING: FtStatus = 24;
pub const FT_OPEN_BY_SERIAL_NUMBER: u32 = 1;

pub const FT_FLAGS_HISPEED: u32 = 2;
#[cfg(windows)]
pub const FT_FLAGS_SUPERSPEED: u32 = 4;

pub const FT_DEVICE_232H: u32 = 8;
pub const FT_FLOW_RTS_CTS: u16 = 0x0100;
pub const FT_PURGE_RX: u32 = 1;
pub const FT_PURGE_TX: u32 = 2;
pub const FT_BITMODE_RESET: u8 = 0x00;
pub const FT_BITMODE_MPSSE: u8 = 0x02;
pub const FT_BITMODE_SYNC_FIFO: u8 = 0x40;

/// Win32 `OVERLAPPED`. Initialised by `FT_InitializeOverlapped`.
#[cfg(windows)]
#[repr(C)]
pub struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    h_event: *mut c_void,
}

#[cfg(windows)]
impl Default for Overlapped {
    fn default() -> Self {
        Self { internal: 0, internal_high: 0, offset: 0, offset_high: 0, h_event: std::ptr::null_mut() }
    }
}

macro_rules! ftdi_lib {
    ($name:ident, $dll:expr, { $($fn_name:ident : fn($($arg:ty),*);)* }) => {
        #[allow(non_snake_case)]
        pub struct $name {
            _lib: Library,
            $(pub $fn_name: unsafe extern "system" fn($($arg),*) -> FtStatus,)*
        }

        impl $name {
            fn load() -> Option<Self> {
                let lib = load_library($dll)?;
                unsafe {
                    Some(Self {
                        $($fn_name: *lib.get(concat!(stringify!($fn_name), "\0").as_bytes()).ok()?,)*
                        _lib: lib,
                    })
                }
            }

            pub fn get() -> Option<&'static Self> {
                static LIB: OnceLock<Option<$name>> = OnceLock::new();
                LIB.get_or_init(Self::load).as_ref()
            }
        }

        unsafe impl Send for $name {}
        unsafe impl Sync for $name {}
    };
}

/// Prefer a DLL shipped next to the executable, then fall back to the system search path.
fn load_library(names: &[&str]) -> Option<Library> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
    for name in names {
        if let Some(dir) = &exe_dir {
            if let Ok(lib) = unsafe { Library::new(dir.join(name)) } {
                return Some(lib);
            }
        }
        if let Ok(lib) = unsafe { Library::new(*name) } {
            return Some(lib);
        }
    }
    None
}

#[cfg(windows)]
ftdi_lib!(D3xx, &["FTD3XX.dll"], {
    FT_CreateDeviceInfoList: fn(*mut u32);
    FT_GetDeviceInfoDetail: fn(u32, *mut u32, *mut u32, *mut u32, *mut u32, *mut c_void, *mut c_void, *mut FtHandle);
    FT_Create: fn(*mut c_void, u32, *mut FtHandle);
    FT_Close: fn(FtHandle);
    FT_WritePipe: fn(FtHandle, u8, *mut u8, u32, *mut u32, *mut Overlapped);
    FT_ReadPipe: fn(FtHandle, u8, *mut u8, u32, *mut u32, *mut Overlapped);
    FT_ReadPipeEx: fn(FtHandle, u8, *mut u8, u32, *mut u32, *mut Overlapped);
    FT_GetOverlappedResult: fn(FtHandle, *mut Overlapped, *mut u32, i32);
    FT_InitializeOverlapped: fn(FtHandle, *mut Overlapped);
    FT_ReleaseOverlapped: fn(FtHandle, *mut Overlapped);
    FT_SetStreamPipe: fn(FtHandle, u8, u8, u8, u32);
    FT_AbortPipe: fn(FtHandle, u8);
    FT_SetPipeTimeout: fn(FtHandle, u8, u32);
    FT_GetPipeTimeout: fn(FtHandle, u8, *mut u32);
});

#[cfg(windows)]
const D2XX_NAMES: &[&str] = &["ftd2xx.dll", "ftd2xx64.dll"];
#[cfg(not(windows))]
const D2XX_NAMES: &[&str] = &["libftd2xx.so", "/usr/local/lib/libftd2xx.so"];

ftdi_lib!(D2xx, D2XX_NAMES, {
    FT_CreateDeviceInfoList: fn(*mut u32);
    FT_GetDeviceInfoDetail: fn(u32, *mut u32, *mut u32, *mut u32, *mut u32, *mut c_void, *mut c_void, *mut FtHandle);
    FT_OpenEx: fn(*mut c_void, u32, *mut FtHandle);
    FT_Close: fn(FtHandle);
    FT_Read: fn(FtHandle, *mut c_void, u32, *mut u32);
    FT_Write: fn(FtHandle, *mut c_void, u32, *mut u32);
    FT_SetTimeouts: fn(FtHandle, u32, u32);
    FT_ResetDevice: fn(FtHandle);
    FT_SetUSBParameters: fn(FtHandle, u32, u32);
    FT_SetChars: fn(FtHandle, u8, u8, u8, u8);
    FT_SetLatencyTimer: fn(FtHandle, u8);
    FT_SetFlowControl: fn(FtHandle, u16, u8, u8);
    FT_SetBitMode: fn(FtHandle, u8, u8);
    FT_Purge: fn(FtHandle, u32);
    FT_ReadEE: fn(FtHandle, u32, *mut u16);
    FT_GetQueueStatus: fn(FtHandle, *mut u32);
});

/// Shared device-list walk for both drivers: returns (flags, type, serial, description).
pub fn list_ftdi_devices(
    create_list: unsafe extern "system" fn(*mut u32) -> FtStatus,
    get_detail: unsafe extern "system" fn(u32, *mut u32, *mut u32, *mut u32, *mut u32, *mut c_void, *mut c_void, *mut FtHandle) -> FtStatus,
) -> Vec<(u32, u32, String, String)> {
    let mut out = Vec::new();
    let mut num = 0u32;
    unsafe {
        if create_list(&mut num) != FT_OK {
            return out;
        }
        for i in 0..num {
            let (mut flags, mut ty, mut id, mut loc) = (0u32, 0u32, 0u32, 0u32);
            let mut serial = [0u8; 32];
            let mut desc = [0u8; 65];
            let mut handle: FtHandle = std::ptr::null_mut();
            let status = get_detail(
                i,
                &mut flags,
                &mut ty,
                &mut id,
                &mut loc,
                serial.as_mut_ptr().cast(),
                desc.as_mut_ptr().cast(),
                &mut handle,
            );
            if status != FT_OK {
                continue;
            }
            out.push((flags, ty, c_str(&serial), c_str(&desc)));
        }
    }
    out
}

fn c_str(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// NUL-terminated serial number buffer for `FT_Create` / `FT_OpenEx` (max 16 chars).
pub fn serial_arg(serial: &str) -> [u8; 17] {
    let mut buf = [0u8; 17];
    let bytes = serial.as_bytes();
    let n = bytes.len().min(16);
    buf[..n].copy_from_slice(&bytes[..n]);
    buf
}
