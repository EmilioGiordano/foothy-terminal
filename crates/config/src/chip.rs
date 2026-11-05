use crate::Colors;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChipStyle {
    Surface,
    Inverse,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pill {
    Fill(u32),
    Reverse,
}

/// `None` means the chip leaves the colour alone: the default foreground under
/// `Pill::Fill`, the swapped pair under `Pill::Reverse`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChipColors {
    pub pill: Pill,
    pub path: Option<u32>,
    pub folder_icon: Option<u32>,
    pub branch: Option<u32>,
    pub branch_icon: Option<u32>,
    pub chevron: u32,
}

const SURFACE_LIFT: f32 = 0.14;
const BRANCH_FADE: f32 = 0.20;

/// Per-channel sRGB lerp, `t` of `b` over `a`.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let a = ((a >> shift) & 0xff) as f32;
        let b = ((b >> shift) & 0xff) as f32;
        ((a + (b - a) * t).round() as u32).min(0xff) << shift
    };
    channel(16) | channel(8) | channel(0)
}

pub fn chip_colors(colors: &Colors, style: ChipStyle) -> ChipColors {
    let fg = colors.foreground;
    let bg = colors.background;
    let cyan = colors.palette[6];
    match style {
        ChipStyle::Surface => ChipColors {
            pill: Pill::Fill(mix(bg, fg, SURFACE_LIFT)),
            path: Some(fg),
            folder_icon: Some(cyan),
            branch: Some(mix(fg, bg, BRANCH_FADE)),
            branch_icon: Some(colors.palette[5]),
            chevron: cyan,
        },
        ChipStyle::Inverse => ChipColors {
            pill: Pill::Reverse,
            path: None,
            folder_icon: None,
            branch: None,
            branch_icon: None,
            chevron: cyan,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Variant};

    fn colors(fg: u32, bg: u32) -> Colors {
        let mut c = Config::default().colors;
        c.foreground = fg;
        c.background = bg;
        c
    }

    #[test]
    fn mixing_keeps_the_ends_exact() {
        assert_eq!(mix(0x0012_3456, 0x00ab_cdef, 0.0), 0x0012_3456);
        assert_eq!(mix(0x0012_3456, 0x00ab_cdef, 1.0), 0x00ab_cdef);
    }

    #[test]
    fn mixing_is_per_channel() {
        assert_eq!(mix(0x0000_0000, 0x00ff_8000, 0.5), 0x0080_4000);
    }

    #[test]
    fn a_fraction_outside_the_range_cannot_wrap_a_channel() {
        assert_eq!(mix(0x0000_0000, 0x00ff_ffff, 2.0), 0x00ff_ffff);
        assert_eq!(mix(0x00ff_ffff, 0x0000_0000, -1.0), 0x00ff_ffff);
    }

    #[test]
    fn surface_lifts_the_pill_off_the_background() {
        let c = chip_colors(&colors(0x00ff_ffff, 0x0000_0000), ChipStyle::Surface);
        assert_eq!(c.pill, Pill::Fill(mix(0x0000_0000, 0x00ff_ffff, 0.14)));
        assert_eq!(c.path, Some(0x00ff_ffff));
        assert_eq!(c.branch, Some(mix(0x00ff_ffff, 0x0000_0000, 0.20)));
    }

    #[test]
    fn surface_takes_its_icons_from_the_palette() {
        let mut colors = colors(0x00ff_ffff, 0x0000_0000);
        colors.palette[5] = 0x0011_1111;
        colors.palette[6] = 0x0022_2222;
        let c = chip_colors(&colors, ChipStyle::Surface);
        assert_eq!(c.branch_icon, Some(0x0011_1111));
        assert_eq!(c.folder_icon, Some(0x0022_2222));
        assert_eq!(c.chevron, 0x0022_2222);
    }

    #[test]
    fn inverse_names_no_colour_but_the_chevron() {
        let mut colors = colors(0x00ff_ffff, 0x0000_0000);
        colors.palette[6] = 0x0022_2222;
        let c = chip_colors(&colors, ChipStyle::Inverse);
        assert_eq!(c.pill, Pill::Reverse);
        assert_eq!(c.path, None);
        assert_eq!(c.folder_icon, None);
        assert_eq!(c.branch, None);
        assert_eq!(c.branch_icon, None);
        assert_eq!(c.chevron, 0x0022_2222);
    }

    #[test]
    fn the_mapping_follows_the_resolved_theme() {
        let cfg = Config::default();
        assert_eq!(cfg.variant, Variant::Dark);
        let c = chip_colors(&cfg.colors, ChipStyle::Surface);
        assert_eq!(c.path, Some(cfg.colors.foreground));
    }
}
