use crate::Colors;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChipStyle {
    Surface,
    Inverse,
}

impl ChipStyle {
    pub const ALL: [ChipStyle; 2] = [ChipStyle::Surface, ChipStyle::Inverse];

    /// Must stay a spelling `prompt-style` parses: state.ini is read back.
    pub fn name(self) -> &'static str {
        match self {
            ChipStyle::Surface => "surface",
            ChipStyle::Inverse => "inverse",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pill {
    Fill(u32),
    Reverse,
}

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

/// A sentinel has to be a value no program is likely to print: anything that
/// emits it as truecolor gets repainted in a chip colour.
pub const SENTINEL_PILL: u32 = 0x0001_02f0;
pub const SENTINEL_PATH: u32 = 0x0001_02f1;
pub const SENTINEL_FOLDER_ICON: u32 = 0x0001_02f2;
pub const SENTINEL_BRANCH: u32 = 0x0001_02f3;
pub const SENTINEL_BRANCH_ICON: u32 = 0x0001_02f4;

pub const SENTINELS: [u32; 5] = [
    SENTINEL_PILL,
    SENTINEL_PATH,
    SENTINEL_FOLDER_ICON,
    SENTINEL_BRANCH,
    SENTINEL_BRANCH_ICON,
];

pub fn chip_aliases(colors: &Colors, style: ChipStyle) -> [(u32, u32); 5] {
    let chip = chip_colors(colors, style);
    let (pill, unnamed) = match chip.pill {
        Pill::Fill(c) => (c, colors.foreground),
        Pill::Reverse => (colors.foreground, colors.background),
    };
    let ink = |c: Option<u32>| c.unwrap_or(unnamed);
    [
        (SENTINEL_PILL, pill),
        (SENTINEL_PATH, ink(chip.path)),
        (SENTINEL_FOLDER_ICON, ink(chip.folder_icon)),
        (SENTINEL_BRANCH, ink(chip.branch)),
        (SENTINEL_BRANCH_ICON, ink(chip.branch_icon)),
    ]
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
    fn the_sentinels_are_the_values_the_chip_prints() {
        assert_eq!(
            SENTINELS,
            [
                0x0001_02f0,
                0x0001_02f1,
                0x0001_02f2,
                0x0001_02f3,
                0x0001_02f4
            ]
        );
    }

    #[test]
    fn no_sentinel_is_reachable_from_the_default_palette_or_the_cube() {
        let cube = [0u32, 95, 135, 175, 215, 255];
        for s in SENTINELS {
            let channels = [(s >> 16) & 0xff, (s >> 8) & 0xff, s & 0xff];
            assert!(
                !channels.iter().all(|c| cube.contains(c)),
                "{s:#08x} is in the 6x6x6 cube"
            );
            assert!(
                !(channels[0] == channels[1] && channels[1] == channels[2]),
                "{s:#08x} is on the grey ramp"
            );
            assert!(
                !crate::default_palette().contains(&s),
                "{s:#08x} is a default palette entry"
            );
        }
    }

    #[test]
    fn the_aliases_name_every_sentinel_once() {
        for style in ChipStyle::ALL {
            let pairs = chip_aliases(&Config::default().colors, style);
            let keys: Vec<u32> = pairs.iter().map(|(k, _)| *k).collect();
            assert_eq!(keys, SENTINELS, "{style:?}");
        }
    }

    #[test]
    fn surface_resolves_the_sentinels_to_the_fill_and_its_inks() {
        let c = colors(0x00ff_ffff, 0x0000_0000);
        let pairs = chip_aliases(&c, ChipStyle::Surface);
        assert_eq!(pairs[0].1, mix(0x0000_0000, 0x00ff_ffff, SURFACE_LIFT));
        assert_eq!(pairs[1].1, 0x00ff_ffff);
        assert_eq!(pairs[2].1, c.palette[6]);
        assert_eq!(pairs[3].1, mix(0x00ff_ffff, 0x0000_0000, BRANCH_FADE));
        assert_eq!(pairs[4].1, c.palette[5]);
    }

    #[test]
    fn inverse_resolves_the_pill_to_the_foreground_and_every_ink_to_the_background() {
        let c = colors(0x00ff_ffff, 0x0000_0000);
        let pairs = chip_aliases(&c, ChipStyle::Inverse);
        assert_eq!(pairs[0].1, 0x00ff_ffff);
        for (_, real) in &pairs[1..] {
            assert_eq!(*real, 0x0000_0000);
        }
    }

    #[test]
    fn the_mapping_follows_the_resolved_theme() {
        let cfg = Config::default();
        assert_eq!(cfg.variant, Variant::Dark);
        let c = chip_colors(&cfg.colors, ChipStyle::Surface);
        assert_eq!(c.path, Some(cfg.colors.foreground));
    }
}
