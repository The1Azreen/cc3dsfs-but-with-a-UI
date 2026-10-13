//! Turns raw capture-card buffers into upright RGB screens.
//! Ported from the Loopy paths of `conversions.cpp`.

pub const TOP_WIDTH_3DS: usize = 400;
pub const BOT_WIDTH_3DS: usize = 320;
pub const HEIGHT_3DS: usize = 240;
pub const WIDTH_DS: usize = 256;
pub const HEIGHT_DS: usize = 192;

/// 3DS boards send the screens column-major (the LCDs are mounted rotated),
/// 240 pixels per column. Top + bottom = 720 columns, 3D adds 400 more.
pub const COLUMNS_3DS_2D: usize = TOP_WIDTH_3DS + BOT_WIDTH_3DS;
pub const COLUMNS_3DS_3D: usize = TOP_WIDTH_3DS * 2 + BOT_WIDTH_3DS;
pub const VIDEO_SIZE_3DS_2D: usize = COLUMNS_3DS_2D * HEIGHT_3DS * 3;
pub const VIDEO_SIZE_3DS_3D: usize = COLUMNS_3DS_3D * HEIGHT_3DS * 3;
/// DS boards send top and bottom interleaved as RGB565 pairs.
pub const VIDEO_SIZE_DS: usize = WIDTH_DS * HEIGHT_DS * 2 * 2;

/// The first 80 top-screen columns arrive before the bottom screen starts.
const NO_BOTTOM_COLUMNS: usize = TOP_WIDTH_3DS - BOT_WIDTH_3DS;

#[derive(Clone)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    /// Tightly packed RGB8, row-major.
    pub rgb: Vec<u8>,
}

impl Plane {
    fn new(width: usize, height: usize) -> Self {
        Self { width, height, rgb: vec![0; width * height * 3] }
    }
}

#[derive(Clone)]
pub struct Frame {
    pub is_3ds: bool,
    pub top: Plane,
    /// Right-eye image when capturing in 3D.
    pub top_right: Option<Plane>,
    pub bottom: Plane,
}

/// Copies one 240-pixel input column into column `x` of an upright plane.
/// Within a column, pixel 0 is the bottom of the screen.
#[inline]
fn put_column(plane: &mut Plane, x: usize, column: &[u8]) {
    let w = plane.width;
    for y in 0..HEIGHT_3DS {
        let src = (HEIGHT_3DS - 1 - y) * 3;
        let dst = (y * w + x) * 3;
        plane.rgb[dst..dst + 3].copy_from_slice(&column[src..src + 3]);
    }
}

#[inline]
fn column(data: &[u8], index: usize) -> &[u8] {
    let start = index * HEIGHT_3DS * 3;
    &data[start..start + HEIGHT_3DS * 3]
}

/// New 3DS XL (FTD3) board, 2D mode.
pub fn ftd3_2d(data: &[u8]) -> Frame {
    let mut top = Plane::new(TOP_WIDTH_3DS, HEIGHT_3DS);
    let mut bottom = Plane::new(BOT_WIDTH_3DS, HEIGHT_3DS);
    for i in 0..NO_BOTTOM_COLUMNS {
        put_column(&mut top, i, column(data, i));
    }
    for i in 0..BOT_WIDTH_3DS {
        put_column(&mut bottom, i, column(data, NO_BOTTOM_COLUMNS + i * 2));
        put_column(&mut top, NO_BOTTOM_COLUMNS + i, column(data, NO_BOTTOM_COLUMNS + i * 2 + 1));
    }
    Frame { is_3ds: true, top, top_right: None, bottom }
}

/// New 3DS XL (FTD3) board, 3D mode.
pub fn ftd3_3d(data: &[u8]) -> Frame {
    let mut left = Plane::new(TOP_WIDTH_3DS, HEIGHT_3DS);
    let mut right = Plane::new(TOP_WIDTH_3DS, HEIGHT_3DS);
    let mut bottom = Plane::new(BOT_WIDTH_3DS, HEIGHT_3DS);
    for i in 0..NO_BOTTOM_COLUMNS {
        put_column(&mut left, i, column(data, i * 2));
        put_column(&mut right, i, column(data, i * 2 + 1));
    }
    let base = NO_BOTTOM_COLUMNS * 2;
    let last = BOT_WIDTH_3DS - 1;
    for i in 0..last {
        put_column(&mut left, NO_BOTTOM_COLUMNS + i, column(data, base + i * 3));
        put_column(&mut right, NO_BOTTOM_COLUMNS + i, column(data, base + i * 3 + 1));
        put_column(&mut bottom, i, column(data, base + i * 3 + 2));
    }
    // The last triple swaps the right-eye and bottom columns.
    put_column(&mut left, NO_BOTTOM_COLUMNS + last, column(data, base + last * 3));
    put_column(&mut bottom, last, column(data, base + last * 3 + 1));
    put_column(&mut right, NO_BOTTOM_COLUMNS + last, column(data, base + last * 3 + 2));
    Frame { is_3ds: true, top: left, top_right: Some(right), bottom }
}

/// Old 3DS (legacy USB) board: 400 top columns followed by 320 bottom columns.
pub fn old_3ds(data: &[u8]) -> Frame {
    let mut top = Plane::new(TOP_WIDTH_3DS, HEIGHT_3DS);
    let mut bottom = Plane::new(BOT_WIDTH_3DS, HEIGHT_3DS);
    for i in 0..TOP_WIDTH_3DS {
        put_column(&mut top, i, column(data, i));
    }
    for i in 0..BOT_WIDTH_3DS {
        put_column(&mut bottom, i, column(data, TOP_WIDTH_3DS + i));
    }
    Frame { is_3ds: true, top, top_right: None, bottom }
}

#[inline]
fn rgb565_to_rgb(px: u16, out: &mut [u8]) {
    let r = ((px >> 11) & 0x1F) as u8;
    let g = ((px >> 5) & 0x3F) as u8;
    let b = (px & 0x1F) as u8;
    out[0] = (r << 3) | (r >> 2);
    out[1] = (g << 2) | (g >> 4);
    out[2] = (b << 3) | (b >> 2);
}

/// DS boards (old USB and new FTD2): little-endian RGB565 pairs of (bottom, top).
pub fn ds(data: &[u8]) -> Frame {
    let mut top = Plane::new(WIDTH_DS, HEIGHT_DS);
    let mut bottom = Plane::new(WIDTH_DS, HEIGHT_DS);
    for p in 0..WIDTH_DS * HEIGHT_DS {
        let b = u16::from_le_bytes([data[p * 4], data[p * 4 + 1]]);
        let t = u16::from_le_bytes([data[p * 4 + 2], data[p * 4 + 3]]);
        rgb565_to_rgb(b, &mut bottom.rgb[p * 3..p * 3 + 3]);
        rgb565_to_rgb(t, &mut top.rgb[p * 3..p * 3 + 3]);
    }
    Frame { is_3ds: false, top, top_right: None, bottom }
}

/// Interleaved stereo i16 samples following the video data.
/// `max_samples` counts individual i16 values, as in `get_audio_n_samples`.
pub fn audio_after_video(data: &[u8], video_size: usize, max_samples: usize) -> Vec<i16> {
    if data.len() <= video_size {
        return Vec::new();
    }
    let mut n = ((data.len() - video_size) / 2).min(max_samples);
    n -= n % 2;
    data[video_size..video_size + n * 2]
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fills every input column with its own index so the destination can be checked.
    fn tagged_columns(columns: usize) -> Vec<u8> {
        let mut data = vec![0u8; columns * HEIGHT_3DS * 3];
        for c in 0..columns {
            for p in 0..HEIGHT_3DS {
                let i = (c * HEIGHT_3DS + p) * 3;
                data[i..i + 3].copy_from_slice(&[(c >> 8) as u8, c as u8, p as u8]);
            }
        }
        data
    }

    fn source_column(plane: &Plane, x: usize, y: usize) -> (usize, usize) {
        let i = (y * plane.width + x) * 3;
        (((plane.rgb[i] as usize) << 8) | plane.rgb[i + 1] as usize, plane.rgb[i + 2] as usize)
    }

    #[test]
    fn ftd3_2d_column_order() {
        let f = ftd3_2d(&tagged_columns(COLUMNS_3DS_2D));
        // First 80 input columns are the left of the top screen.
        assert_eq!(source_column(&f.top, 0, 0), (0, HEIGHT_3DS - 1));
        assert_eq!(source_column(&f.top, 79, HEIGHT_3DS - 1), (79, 0));
        // Then bottom and top columns alternate.
        assert_eq!(source_column(&f.bottom, 0, 0).0, 80);
        assert_eq!(source_column(&f.top, 80, 0).0, 81);
        assert_eq!(source_column(&f.bottom, 319, 0).0, 80 + 638);
        assert_eq!(source_column(&f.top, 399, 0).0, 80 + 639);
    }

    #[test]
    fn ftd3_3d_column_order() {
        let f = ftd3_3d(&tagged_columns(COLUMNS_3DS_3D));
        let right = f.top_right.as_ref().unwrap();
        assert_eq!(source_column(&f.top, 0, 0).0, 0);
        assert_eq!(source_column(right, 0, 0).0, 1);
        assert_eq!(source_column(&f.top, 80, 0).0, 160);
        assert_eq!(source_column(right, 80, 0).0, 161);
        assert_eq!(source_column(&f.bottom, 0, 0).0, 162);
        // Last triple has right eye and bottom swapped.
        assert_eq!(source_column(&f.top, 399, 0).0, 160 + 319 * 3);
        assert_eq!(source_column(&f.bottom, 319, 0).0, 160 + 319 * 3 + 1);
        assert_eq!(source_column(right, 399, 0).0, 160 + 319 * 3 + 2);
    }

    #[test]
    fn ds_deinterleaves_bottom_then_top() {
        let mut data = vec![0u8; VIDEO_SIZE_DS];
        // Pixel 0: bottom pure red, top pure blue.
        data[0..2].copy_from_slice(&0xF800u16.to_le_bytes());
        data[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        let f = ds(&data);
        assert_eq!(&f.bottom.rgb[0..3], &[255, 0, 0]);
        assert_eq!(&f.top.rgb[0..3], &[0, 0, 255]);
    }

    #[test]
    fn audio_is_capped_and_even() {
        let data = vec![0u8; 10 + 7 * 2];
        assert_eq!(audio_after_video(&data, 10, 100).len(), 6);
        assert_eq!(audio_after_video(&data, 10, 4).len(), 4);
    }
}
