//! Where the screens go on an output, in physical pixels.
//! Same maths as the `Screens` component in `ui/app.slint`, for outputs
//! that are not drawn by Slint (the HDR window).

use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub top: Option<Placement>,
    pub bottom: Option<Placement>,
}

/// `top` / `bottom` are the shown screen sizes in source pixels (after crop
/// and rotation), or `None` when that screen is hidden or empty.
pub fn compute(s: &Settings, out_w: f32, out_h: f32, top: Option<(f32, f32)>, bottom: Option<(f32, f32)>, split: bool) -> Layout {
    let layout = s.layout.clamp(0, 5);
    let top = top.filter(|_| layout != 5);
    let bottom = bottom.filter(|_| layout != 4 && !split);
    let arrangement = if layout >= 4 { 0 } else { layout };

    let (tw, th) = top.unwrap_or((0.0, 0.0));
    let (bw, bh) = bottom.map(|(w, h)| (w * s.bottom_scale, h * s.bottom_scale)).unwrap_or((0.0, 0.0));
    let sep = if top.is_some() && bottom.is_some() { s.separator as f32 } else { 0.0 };
    let vertical = arrangement <= 1;
    let nat_w = (if vertical { tw.max(bw) } else { tw + bw + sep }).max(1.0);
    let nat_h = (if vertical { th + bh + sep } else { th.max(bh) }).max(1.0);
    let fit = (out_w / nat_w).min(out_h / nat_h);
    let scale = if s.integer_scaling && fit >= 1.0 { fit.floor() } else { fit };
    let ox = (out_w - nat_w * scale) / 2.0;
    let oy = (out_h - nat_h * scale) / 2.0;
    let top_first = arrangement == 0 || arrangement == 2;
    let offset = s.screen_offset;

    let place = |x: f32, y: f32, w: f32, h: f32| Placement { x: ox + x * scale, y: oy + y * scale, w: w * scale, h: h * scale };
    let top_rect = top.map(|_| {
        let x = if vertical { (nat_w - tw) * offset } else if top_first { 0.0 } else { bw + sep };
        let y = if vertical { if top_first { 0.0 } else { bh + sep } } else { (nat_h - th) * offset };
        place(x, y, tw, th)
    });
    let bottom_rect = bottom.map(|_| {
        let x = if vertical { (nat_w - bw) * offset } else if top_first { tw + sep } else { 0.0 };
        let y = if vertical { if top_first { th + sep } else { 0.0 } } else { (nat_h - bh) * offset };
        place(x, y, bw, bh)
    });
    Layout { top: top_rect, bottom: bottom_rect }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::PRESETS;

    fn layout_for(preset: usize) -> Layout {
        let mut s = Settings::default();
        PRESETS[preset].apply(&mut s);
        compute(&s, 3840.0, 2160.0, Some((400.0, 240.0)), Some((320.0, 240.0)), false)
    }

    /// Positions measured from Game Sack's 4K footage and from cc3dsfs fullscreen at 4K.
    #[test]
    fn game_sack_positions_on_4k() {
        let top = layout_for(0).top.unwrap();
        assert_eq!((top.x, top.y, top.w, top.h), (120.0, 0.0, 3600.0, 2160.0));
        let bottom = layout_for(1).bottom.unwrap();
        assert_eq!((bottom.x, bottom.w), (480.0, 2880.0));
        let both = layout_for(2);
        assert_eq!((both.top.unwrap().x, both.top.unwrap().w), (1020.0, 1800.0));
        assert_eq!((both.bottom.unwrap().x, both.bottom.unwrap().y), (1200.0, 1080.0));
    }
}
