//! One-click screen setups.
//!
//! "Game Sack" presets recreate the 3DS views from Game Sack's videos
//! (recorded at 4K with an LCD grid): use them with cc3dsfs on a 4K display.
//! "Morph 4K" presets match the profiles in the morph4k-profile-test repo:
//! cc3dsfs outputs 1080p at a whole-number scale and the Morph adds the grid.

use crate::settings::Settings;

/// Layout indices, as in the UI's Screens list.
const TOP_ABOVE_BOTTOM: i32 = 0;
const TOP_ONLY: i32 = 4;
const BOTTOM_ONLY: i32 = 5;

/// Gap Game Sack leaves for the 3DS hinge, in 3DS pixels (measured at 4K).
pub const HINGE_GAP: i32 = 28;

const LCD_GRID: i32 = 1;
const FILTER_OFF: i32 = 0;

pub struct Preset {
    pub name: &'static str,
    pub hint: &'static str,
    layout: i32,
    separator: i32,
    integer_scaling: bool,
    filter_style: i32,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Game Sack: Top screen",
        hint: "9x on 4K, full height, LCD grid",
        layout: TOP_ONLY,
        separator: 0,
        integer_scaling: true,
        filter_style: LCD_GRID,
    },
    Preset {
        name: "Game Sack: Bottom screen",
        hint: "9x on 4K, full height, LCD grid",
        layout: BOTTOM_ONLY,
        separator: 0,
        integer_scaling: true,
        filter_style: LCD_GRID,
    },
    Preset {
        name: "Game Sack: Both screens",
        hint: "Stacked, full height (4.5x on 4K)",
        layout: TOP_ABOVE_BOTTOM,
        separator: 0,
        integer_scaling: false,
        filter_style: LCD_GRID,
    },
    Preset {
        name: "Game Sack: Hinge gap",
        hint: "Both, with a gap like the hinge",
        layout: TOP_ABOVE_BOTTOM,
        separator: HINGE_GAP,
        integer_scaling: false,
        filter_style: LCD_GRID,
    },
    Preset {
        name: "Morph 4K: Top 9x",
        hint: "1080p; Morph preset Top 9x",
        layout: TOP_ONLY,
        separator: 0,
        integer_scaling: true,
        filter_style: FILTER_OFF,
    },
    Preset {
        name: "Morph 4K: Bottom 9x",
        hint: "1080p; Morph preset Bottom 9x",
        layout: BOTTOM_ONLY,
        separator: 0,
        integer_scaling: true,
        filter_style: FILTER_OFF,
    },
    Preset {
        name: "Morph 4K: Both 4.5x",
        hint: "1080p; Morph preset Both Screens",
        layout: TOP_ABOVE_BOTTOM,
        separator: 0,
        integer_scaling: true,
        filter_style: FILTER_OFF,
    },
    Preset {
        name: "Morph 4K: Hinge 4.25x",
        hint: "1080p; Morph preset Hinge Gap",
        layout: TOP_ABOVE_BOTTOM,
        separator: HINGE_GAP,
        integer_scaling: true,
        filter_style: FILTER_OFF,
    },
];

impl Preset {
    /// Apply the preset's screen setup, keeping unrelated settings (audio, theme…).
    pub fn apply(&self, s: &mut Settings) {
        s.layout = self.layout;
        s.separator = self.separator;
        s.integer_scaling = self.integer_scaling;
        s.filter_style = self.filter_style;
        s.bottom_scale = 1.0;
        s.screen_offset = 0.5;
        s.smoothing = false;
        s.top_rotation = 0;
        s.bottom_rotation = 0;
        s.crop_3ds = 0;
        s.split_windows = false;
        s.mode_3d = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scale of the stacked layout for a given output height, as `Screens` computes it.
    fn stacked_scale(height: f32, gap: i32) -> f32 {
        height / (240.0 + 240.0 + gap as f32)
    }

    #[test]
    fn game_sack_scales_on_4k() {
        assert_eq!(2160.0 / 240.0, 9.0);
        assert_eq!(stacked_scale(2160.0, 0), 4.5);
        assert!((stacked_scale(2160.0, HINGE_GAP) - 4.25).abs() < 0.01);
    }

    #[test]
    fn presets_keep_unrelated_settings() {
        let mut s = Settings { volume: 37, theme: 3, ..Settings::default() };
        PRESETS[0].apply(&mut s);
        assert_eq!((s.volume, s.theme, s.layout), (37, 3, TOP_ONLY));
    }
}
