//! Persistent settings and profile slots, stored as JSON in %APPDATA%\cc3dsfs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const NUM_PROFILES: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Index into `LAYOUTS` in the UI.
    pub layout: i32,
    /// Bottom screen size relative to the top screen.
    pub bottom_scale: f32,
    /// Where the smaller screen sits along the shared edge (0 = start, 0.5 = centre, 1 = end).
    pub screen_offset: f32,
    /// Gap between screens, in source pixels.
    pub separator: i32,
    pub integer_scaling: bool,
    pub smoothing: bool,
    pub top_rotation: i32,
    pub bottom_rotation: i32,
    pub crop_3ds: i32,
    pub crop_ds: i32,
    pub game_crops: bool,
    pub volume: i32,
    pub muted: bool,
    pub audio_device: String,
    pub enable_3d: bool,
    /// How 3D frames are shown: 0 = left eye, 1 = side by side.
    pub mode_3d: i32,
    pub fullscreen: bool,
    pub split_windows: bool,
    pub auto_connect: bool,
    pub theme: i32,
    /// Screen filter: 0 off, 1 LCD grid, 2 fine grid, 3 scanlines.
    pub filter_style: i32,
    /// Filter strength in percent.
    pub filter_strength: f32,
    /// Windows: show the game in an HDR window (see `hdr.rs`).
    pub hdr_output: bool,
    /// Brightness of SDR white in the HDR window, in nits, before the
    /// filter compensation.
    pub hdr_brightness: f32,
    /// Black frame insertion: 0 off, 1 auto, 2.. = 1.. black frames per
    /// game frame (see `bfi.rs`).
    pub bfi: i32,
    /// Wait for the display's refresh before showing a frame. Off lowers
    /// latency but can tear. BFI keeps it on.
    pub vsync: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            layout: 0,
            bottom_scale: 1.0,
            screen_offset: 0.5,
            separator: 0,
            integer_scaling: false,
            smoothing: false,
            top_rotation: 0,
            bottom_rotation: 0,
            crop_3ds: 0,
            crop_ds: 0,
            game_crops: false,
            volume: 100,
            muted: false,
            audio_device: String::new(),
            enable_3d: false,
            mode_3d: 0,
            fullscreen: false,
            split_windows: false,
            auto_connect: true,
            theme: 0,
            filter_style: 0,
            filter_strength: 30.0,
            hdr_output: false,
            hdr_brightness: 300.0,
            bfi: 0,
            vsync: true,
        }
    }
}

fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("cc3dsfs")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn profile_path(slot: usize) -> PathBuf {
    config_dir().join(format!("profile_{}.json", slot + 1))
}

fn read(path: PathBuf) -> Option<Settings> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write(path: PathBuf, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

impl Settings {
    pub fn load() -> Self {
        read(settings_path()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        write(settings_path(), self)
    }

    pub fn load_profile(slot: usize) -> Option<Self> {
        read(profile_path(slot))
    }

    pub fn save_profile(&self, slot: usize) -> Result<(), String> {
        write(profile_path(slot), self)
    }

    pub fn profile_exists(slot: usize) -> bool {
        profile_path(slot).exists()
    }
}
