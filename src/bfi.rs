//! Black frame insertion: on a fast display, show each game frame for one
//! refresh and black for the next ones, so motion looks sharper (less
//! sample-and-hold blur), like the BFI of RetroArch and the RetroTINK 4K.
//!
//! The windows redraw on every refresh while it is on. Each redraw calls
//! `Bfi::next`, which says whether that refresh shows the game or black. The
//! refresh rate is measured from the redraws themselves, which also catches
//! outputs that are not vsync-paced (they would flicker at random).

use std::time::{Duration, Instant};

/// Highest number of black frames per game frame.
pub const MAX_BLACK_FRAMES: i32 = 3;

/// Game frame rate of the 3DS and DS, rounded down so 119.88 Hz still counts as 2x.
const GAME_HZ: f32 = 59.0;

/// Faster than this the redraws are not waiting for vsync.
const MAX_VSYNC_HZ: f32 = 500.0;

/// Black frames per game frame for a setting (0 off, 1 auto, 2.. = 1..)
/// at a refresh rate.
pub fn black_frames(setting: i32, refresh_hz: f32) -> u32 {
    match setting {
        0 => 0,
        1 => ((refresh_hz / GAME_HZ).floor() as i32 - 1).clamp(0, MAX_BLACK_FRAMES) as u32,
        n => (n - 1).clamp(1, MAX_BLACK_FRAMES) as u32,
    }
}

/// Text for the Video page.
pub fn status(setting: i32, refresh_hz: Option<f32>, supported: bool) -> String {
    if !supported {
        return "Not available with this renderer".into();
    }
    if setting == 0 {
        return "Sharper motion on 120 Hz+ displays; flickers".into();
    }
    let Some(hz) = refresh_hz else {
        return "Measures the display once the menu is closed".into();
    };
    if hz > MAX_VSYNC_HZ {
        return "Display is not vsync-paced, so BFI is paused".into();
    }
    let black = black_frames(setting, hz);
    let cycle = (black + 1) as f32;
    if black == 0 {
        format!("{hz:.0} Hz display: needs 120 Hz or more")
    } else if hz / cycle < GAME_HZ {
        format!("{hz:.0} Hz display: too slow for {black} black, frames get dropped")
    } else {
        format!("{hz:.0} Hz display, {black} black per game frame")
    }
}

#[derive(Default)]
pub struct Bfi {
    phase: u32,
    last: Option<Instant>,
    /// Smoothed time between redraws, in seconds.
    interval: Option<f32>,
}

impl Bfi {
    /// Call once per redraw while BFI is on. Returns true when this refresh
    /// should be black.
    pub fn next(&mut self, setting: i32, now: Instant) -> bool {
        if let Some(last) = self.last {
            let dt = now.duration_since(last);
            // Ignore stalls (window moved, menu closed) when measuring.
            if dt < Duration::from_millis(50) {
                let dt = dt.as_secs_f32();
                self.interval = Some(match self.interval {
                    Some(i) => i + (dt - i) * 0.05,
                    None => dt,
                });
            }
        }
        self.last = Some(now);
        let Some(hz) = self.paced_hz() else {
            self.phase = 0;
            return false;
        };
        let cycle = black_frames(setting, hz) + 1;
        self.phase = (self.phase + 1) % cycle;
        self.phase != 0
    }

    /// Call when redraws stop, so the gap is not measured.
    pub fn pause(&mut self) {
        self.last = None;
        self.phase = 0;
    }

    pub fn refresh_hz(&self) -> Option<f32> {
        self.interval.filter(|&i| i > 0.0).map(|i| 1.0 / i)
    }

    fn paced_hz(&self) -> Option<f32> {
        self.refresh_hz().filter(|&hz| hz <= MAX_VSYNC_HZ)
    }

    /// How much brighter the game must be to look as bright as without BFI.
    pub fn brightness_gain(&self, setting: i32) -> f32 {
        self.paced_hz().map_or(1.0, |hz| (black_frames(setting, hz) + 1) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_picks_from_refresh_rate() {
        assert_eq!(black_frames(1, 60.0), 0);
        assert_eq!(black_frames(1, 119.88), 1);
        assert_eq!(black_frames(1, 144.0), 1);
        assert_eq!(black_frames(1, 180.0), 2);
        assert_eq!(black_frames(1, 240.0), 3);
        assert_eq!(black_frames(1, 480.0), 3);
        assert_eq!(black_frames(2, 60.0), 1);
        assert_eq!(black_frames(4, 60.0), 3);
    }

    #[test]
    fn alternates_at_120hz() {
        let mut bfi = Bfi::default();
        let start = Instant::now();
        let step = Duration::from_secs_f64(1.0 / 120.0);
        let pattern: Vec<bool> = (0..8).map(|i| bfi.next(1, start + step * i)).collect();
        // The first redraw has no measurement yet, then game and black alternate.
        assert!(!pattern[0]);
        assert_eq!(&pattern[1..7], &[true, false, true, false, true, false]);
        assert_eq!(bfi.brightness_gain(1), 2.0);
    }

    #[test]
    fn unpaced_output_never_goes_black() {
        let mut bfi = Bfi::default();
        let start = Instant::now();
        assert!((0..20).all(|i| !bfi.next(1, start + Duration::from_micros(500) * i)));
    }
}
