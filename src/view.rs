//! Crops and rotation applied to captured frames before display.
//! Crop table ported from `frontend.cpp`.

use slint::{Image, Rgb8Pixel, SharedPixelBuffer};

use crate::capture::frame::{BOT_WIDTH_3DS, Frame, HEIGHT_3DS, HEIGHT_DS, Plane, TOP_WIDTH_3DS, WIDTH_DS};

#[derive(Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

const fn r(x: usize, y: usize, w: usize, h: usize) -> Rect {
    Rect { x, y, w, h }
}

const HIDDEN: Rect = r(0, 0, 0, 0);

pub struct Crop {
    pub name: &'static str,
    pub top: Rect,
    pub bottom: Rect,
    pub game_specific: bool,
}

const fn centered(outer_w: usize, outer_h: usize, w: usize, h: usize) -> Rect {
    r((outer_w - w) / 2, (outer_h - h) / 2, w, h)
}

const FULL_3DS_BOTTOM: Rect = r(0, 0, BOT_WIDTH_3DS, HEIGHT_3DS);
const NATIVE_DS_BOTTOM_ON_3DS: Rect = r((BOT_WIDTH_3DS - WIDTH_DS) / 2, HEIGHT_3DS - HEIGHT_DS, WIDTH_DS, HEIGHT_DS);

pub const CROPS_3DS: &[Crop] = &[
    Crop { name: "3DS", top: r(0, 0, TOP_WIDTH_3DS, HEIGHT_3DS), bottom: FULL_3DS_BOTTOM, game_specific: false },
    Crop { name: "16:10", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 384, HEIGHT_3DS), bottom: FULL_3DS_BOTTOM, game_specific: false },
    Crop { name: "Scaled DS", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 320, HEIGHT_3DS), bottom: FULL_3DS_BOTTOM, game_specific: false },
    Crop {
        name: "Native DS",
        top: r((TOP_WIDTH_3DS - WIDTH_DS) / 2, 0, WIDTH_DS, HEIGHT_DS),
        bottom: NATIVE_DS_BOTTOM_ON_3DS,
        game_specific: false,
    },
    Crop { name: "Scaled GBA", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 360, 240), bottom: HIDDEN, game_specific: false },
    Crop { name: "Native GBA", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 240, 160), bottom: HIDDEN, game_specific: false },
    Crop { name: "Scaled VC GB", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 266, 240), bottom: HIDDEN, game_specific: false },
    Crop { name: "VC GB", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 160, 144), bottom: HIDDEN, game_specific: false },
    Crop { name: "Scaled SNES", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 284, 224), bottom: HIDDEN, game_specific: false },
    Crop { name: "VC SNES", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 256, 224), bottom: HIDDEN, game_specific: false },
    Crop { name: "VC NES", top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 256, 240), bottom: HIDDEN, game_specific: false },
    Crop {
        name: "Scaled Pokémon DS",
        top: r((TOP_WIDTH_3DS - 320 + 6) / 2, 3, 320 - 6, HEIGHT_3DS - 6),
        bottom: FULL_3DS_BOTTOM,
        game_specific: true,
    },
    Crop {
        name: "Native Pokémon DS",
        top: r((TOP_WIDTH_3DS - WIDTH_DS + 2) / 2, 1, WIDTH_DS - 2, HEIGHT_DS - 2),
        bottom: NATIVE_DS_BOTTOM_ON_3DS,
        game_specific: true,
    },
    Crop {
        name: "Scaled PMD:BRT",
        top: centered(TOP_WIDTH_3DS, HEIGHT_3DS, 320, HEIGHT_3DS),
        bottom: r(3, 0, BOT_WIDTH_3DS - 3, HEIGHT_3DS),
        game_specific: true,
    },
    Crop {
        name: "Native PMD:BRT",
        top: r((TOP_WIDTH_3DS - WIDTH_DS) / 2, 0, WIDTH_DS, HEIGHT_DS),
        bottom: r((BOT_WIDTH_3DS - WIDTH_DS + 2) / 2, HEIGHT_3DS - HEIGHT_DS, WIDTH_DS - 1, HEIGHT_DS),
        game_specific: true,
    },
    Crop {
        name: "Scaled PMD:RRT",
        top: r((TOP_WIDTH_3DS - 360 + 6) / 2, 0, 360 - 3, 240),
        bottom: HIDDEN,
        game_specific: true,
    },
    Crop {
        name: "Native PMD:RRT",
        top: r((TOP_WIDTH_3DS - 240 + 2) / 2, (HEIGHT_3DS - 160) / 2, 240 - 1, 160),
        bottom: HIDDEN,
        game_specific: true,
    },
];

pub const CROPS_DS: &[Crop] = &[
    Crop { name: "DS", top: r(0, 0, WIDTH_DS, HEIGHT_DS), bottom: r(0, 0, WIDTH_DS, HEIGHT_DS), game_specific: false },
    Crop { name: "Top GBA", top: centered(WIDTH_DS, HEIGHT_DS, 240, 160), bottom: HIDDEN, game_specific: false },
    Crop { name: "Bottom GBA", top: HIDDEN, bottom: centered(WIDTH_DS, HEIGHT_DS, 240, 160), game_specific: false },
    Crop {
        name: "Pokémon DS",
        top: r(1, 1, WIDTH_DS - 2, HEIGHT_DS - 2),
        bottom: r(0, 0, WIDTH_DS, HEIGHT_DS),
        game_specific: true,
    },
    Crop { name: "PMD:BRT", top: r(0, 0, WIDTH_DS, HEIGHT_DS), bottom: r(1, 0, WIDTH_DS - 1, HEIGHT_DS), game_specific: true },
    Crop {
        name: "Top PMD:RRT",
        top: r((WIDTH_DS - 240 + 2) / 2, (HEIGHT_DS - 160) / 2, 240 - 1, 160),
        bottom: HIDDEN,
        game_specific: true,
    },
    Crop {
        name: "Bottom PMD:RRT",
        top: HIDDEN,
        bottom: r((WIDTH_DS - 240 + 2) / 2, (HEIGHT_DS - 160) / 2, 240 - 1, 160),
        game_specific: true,
    },
];

/// The crops offered for a console, honouring the "game crops" toggle.
pub fn crops(is_3ds: bool, game_crops: bool) -> Vec<&'static Crop> {
    let table = if is_3ds { CROPS_3DS } else { CROPS_DS };
    table.iter().filter(|c| game_crops || !c.game_specific).collect()
}

pub fn crop_for(is_3ds: bool, game_crops: bool, index: i32) -> &'static Crop {
    let list = crops(is_3ds, game_crops);
    list.get(index.max(0) as usize).copied().unwrap_or(list[0])
}

/// Crop then rotate (clockwise, in degrees) into a Slint image.
pub fn plane_to_image(plane: &Plane, crop: Rect, rotation: i32) -> Option<Image> {
    if crop.w == 0 || crop.h == 0 {
        return None;
    }
    let rot = rotation.rem_euclid(360) / 90;
    let (out_w, out_h) = if rot % 2 == 1 { (crop.h, crop.w) } else { (crop.w, crop.h) };
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(out_w as u32, out_h as u32);
    let dst = buf.make_mut_bytes();
    for y in 0..crop.h {
        let src_row = ((crop.y + y) * plane.width + crop.x) * 3;
        for x in 0..crop.w {
            let (dx, dy) = match rot {
                0 => (x, y),
                1 => (crop.h - 1 - y, x),
                2 => (crop.w - 1 - x, crop.h - 1 - y),
                _ => (y, crop.w - 1 - x),
            };
            let s = src_row + x * 3;
            let d = (dy * out_w + dx) * 3;
            dst[d..d + 3].copy_from_slice(&plane.rgb[s..s + 3]);
        }
    }
    Some(Image::from_rgb8(buf))
}

/// Left and right eye next to each other, for side-by-side 3D output.
pub fn side_by_side(left: &Plane, right: &Plane) -> Plane {
    let w = left.width + right.width;
    let mut rgb = vec![0u8; w * left.height * 3];
    for y in 0..left.height {
        let row = y * w * 3;
        rgb[row..row + left.width * 3].copy_from_slice(&left.rgb[y * left.width * 3..(y + 1) * left.width * 3]);
        rgb[row + left.width * 3..row + w * 3].copy_from_slice(&right.rgb[y * right.width * 3..(y + 1) * right.width * 3]);
    }
    Plane { width: w, height: left.height, rgb }
}

pub struct Screens {
    pub top: Option<Image>,
    pub bottom: Option<Image>,
}

pub fn render(frame: &Frame, crop: &Crop, top_rotation: i32, bottom_rotation: i32, side_by_side_3d: bool) -> Screens {
    let top = match &frame.top_right {
        Some(right) if side_by_side_3d && crop.top.w > 0 => {
            let stitched = side_by_side(&crop_plane(&frame.top, crop.top), &crop_plane(right, crop.top));
            let full = r(0, 0, stitched.width, stitched.height);
            plane_to_image(&stitched, full, top_rotation)
        }
        _ => plane_to_image(&frame.top, crop.top, top_rotation),
    };
    Screens { top, bottom: plane_to_image(&frame.bottom, crop.bottom, bottom_rotation) }
}

fn crop_plane(plane: &Plane, crop: Rect) -> Plane {
    let mut rgb = Vec::with_capacity(crop.w * crop.h * 3);
    for y in crop.y..crop.y + crop.h {
        let s = (y * plane.width + crop.x) * 3;
        rgb.extend_from_slice(&plane.rgb[s..s + crop.w * 3]);
    }
    Plane { width: crop.w, height: crop.h, rgb }
}
