//! Reading of `foothy.ini` and of foot-format colour themes.
//!
//! Layer 2: no OS, no file system. The caller reads the bytes and hands over
//! the text; this crate only parses, merges and resolves.

mod ini;
mod theme;

pub use ini::{Problem, parse};
pub use theme::{bundled, bundled_names};

/// Which of a theme's two colour sets is wanted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    Dark,
    Light,
}

impl Variant {
    pub fn flipped(self) -> Self {
        match self {
            Variant::Dark => Variant::Light,
            Variant::Light => Variant::Dark,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blink {
    /// The platform decides; it already honours the accessibility preference.
    System,
    Off,
    Ms(u64),
}

/// foot writes `cursor=<text> <cursor>`: the first colour is the text drawn
/// under the cursor, the second the cursor body (upstream/foot/config.c:1575).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CursorColors {
    pub text: u32,
    pub cursor: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelectionColors {
    pub foreground: u32,
    pub background: u32,
}

/// One colour variant as a file gave it: every field optional, so layers merge.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PartialColors {
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    /// Slots 0..8 are `regular0..7`, 8..16 `bright0..7`, the rest numeric keys.
    pub palette: [Option<u32>; 256],
    pub cursor: Option<CursorColors>,
    pub selection_foreground: Option<u32>,
    pub selection_background: Option<u32>,
}

impl Default for PartialColors {
    fn default() -> Self {
        Self {
            foreground: None,
            background: None,
            palette: [None; 256],
            cursor: None,
            selection_foreground: None,
            selection_background: None,
        }
    }
}

impl PartialColors {
    pub fn is_empty(&self) -> bool {
        self.foreground.is_none()
            && self.background.is_none()
            && self.cursor.is_none()
            && self.selection_foreground.is_none()
            && self.selection_background.is_none()
            && self.palette.iter().all(Option::is_none)
    }

    /// True when all of `regular0..7` and `bright0..7` are present.
    pub fn has_all_16(&self) -> bool {
        self.palette[..16].iter().all(Option::is_some)
    }

    fn merge(base: Self, over: Self) -> Self {
        let mut palette = base.palette;
        for (slot, over) in palette.iter_mut().zip(over.palette) {
            if over.is_some() {
                *slot = over;
            }
        }
        Self {
            foreground: over.foreground.or(base.foreground),
            background: over.background.or(base.background),
            palette,
            cursor: over.cursor.or(base.cursor),
            selection_foreground: over.selection_foreground.or(base.selection_foreground),
            selection_background: over.selection_background.or(base.selection_background),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Settings {
    /// A path to a .ttf. Family-name lookup needs OS font enumeration, which
    /// is not this layer's business.
    pub font: Option<String>,
    pub font_size: Option<f32>,
    pub pad: Option<u32>,
    pub alpha: Option<u8>,
    pub blink: Option<Blink>,
    pub paste_warning: Option<bool>,
    pub theme: Option<String>,
    pub initial_variant: Option<Variant>,
}

impl Settings {
    fn merge(base: Self, over: Self) -> Self {
        Self {
            font: over.font.or(base.font),
            font_size: over.font_size.or(base.font_size),
            pad: over.pad.or(base.pad),
            alpha: over.alpha.or(base.alpha),
            blink: over.blink.or(base.blink),
            paste_warning: over.paste_warning.or(base.paste_warning),
            theme: over.theme.or(base.theme),
            initial_variant: over.initial_variant.or(base.initial_variant),
        }
    }
}

/// One parsed file, or one layer of precedence.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Partial {
    pub settings: Settings,
    pub dark: PartialColors,
    pub light: PartialColors,
}

/// `over` wins field by field; anything it leaves unset keeps `base`.
pub fn merge(base: Partial, over: Partial) -> Partial {
    Partial {
        settings: Settings::merge(base.settings, over.settings),
        dark: PartialColors::merge(base.dark, over.dark),
        light: PartialColors::merge(base.light, over.light),
    }
}

/// The resolved colours: no holes left.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Colors {
    pub foreground: u32,
    pub background: u32,
    pub palette: [u32; 256],
    pub cursor: Option<CursorColors>,
    pub selection: Option<SelectionColors>,
}

impl Colors {
    pub fn regular(&self) -> &[u32; 8] {
        self.palette[..8].try_into().unwrap()
    }

    pub fn bright(&self) -> &[u32; 8] {
        self.palette[8..16].try_into().unwrap()
    }
}

/// The resolved configuration a platform layer can run on.
#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub font: Option<String>,
    pub font_size: f32,
    pub pad: u32,
    pub alpha: u8,
    pub blink: Blink,
    pub paste_warning: bool,
    pub variant: Variant,
    pub colors: Colors,
}

pub const DEFAULT_FOREGROUND: u32 = 0x00ff_ffff;
pub const DEFAULT_BACKGROUND: u32 = 0x0024_2424;
pub const DEFAULT_FONT_SIZE: f32 = 16.0;
pub const DEFAULT_ALPHA: u8 = 235;

// foot's own defaults, upstream/foot/config.c:33-68. The man page still
// documents the old Solarized values; the code is what ships.
const DEFAULT_16: [u32; 16] = [
    0x0024_2424,
    0x00f6_2b5a,
    0x0047_b413,
    0x00e3_c401,
    0x0024_acd4,
    0x00f2_affd,
    0x0013_c299,
    0x00e6_e6e6,
    0x0061_6161,
    0x00ff_4d51,
    0x0035_d450,
    0x00e9_e836,
    0x005d_c5f8,
    0x00fe_abf2,
    0x0024_dfc4,
    0x00ff_ffff,
];

pub fn default_palette() -> [u32; 256] {
    let mut p = [0u32; 256];
    p[..16].copy_from_slice(&DEFAULT_16);
    const STEPS: [u32; 6] = [0, 95, 135, 175, 215, 255];
    for i in 0..216 {
        p[16 + i] = (STEPS[i / 36] << 16) | (STEPS[(i / 6) % 6] << 8) | STEPS[i % 6];
    }
    for i in 0..24 {
        let v = 8 + i as u32 * 10;
        p[232 + i] = (v << 16) | (v << 8) | v;
    }
    p
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: None,
            font_size: DEFAULT_FONT_SIZE,
            pad: 0,
            alpha: DEFAULT_ALPHA,
            blink: Blink::System,
            paste_warning: true,
            variant: Variant::Dark,
            colors: Colors {
                foreground: DEFAULT_FOREGROUND,
                background: DEFAULT_BACKGROUND,
                palette: default_palette(),
                cursor: None,
                selection: None,
            },
        }
    }
}

impl Partial {
    /// The variant actually used for `wanted`: the file's own
    /// `initial-color-theme` overrides the request, and a theme carrying only
    /// the other variant is used rather than ignored.
    pub fn variant_for(&self, wanted: Variant) -> Variant {
        let asked = self.settings.initial_variant.unwrap_or(wanted);
        let (same, other) = match asked {
            Variant::Dark => (&self.dark, &self.light),
            Variant::Light => (&self.light, &self.dark),
        };
        if same.is_empty() && !other.is_empty() {
            asked.flipped()
        } else {
            asked
        }
    }

    pub fn colors_for(&self, wanted: Variant) -> &PartialColors {
        match self.variant_for(wanted) {
            Variant::Dark => &self.dark,
            Variant::Light => &self.light,
        }
    }

    /// Fill every hole with a default. Colours are taken from one variant
    /// only: mixing a light foreground into a dark background is never right.
    pub fn resolve(&self, wanted: Variant) -> Config {
        let variant = self.variant_for(wanted);
        let c = self.colors_for(wanted);
        let mut palette = default_palette();
        for (slot, over) in palette.iter_mut().zip(&c.palette) {
            if let Some(v) = over {
                *slot = *v;
            }
        }
        let d = Config::default();
        Config {
            font: self.settings.font.clone(),
            font_size: self.settings.font_size.unwrap_or(d.font_size),
            pad: self.settings.pad.unwrap_or(d.pad),
            alpha: self.settings.alpha.unwrap_or(d.alpha),
            blink: self.settings.blink.unwrap_or(d.blink),
            paste_warning: self.settings.paste_warning.unwrap_or(d.paste_warning),
            variant,
            colors: Colors {
                foreground: c.foreground.unwrap_or(d.colors.foreground),
                background: c.background.unwrap_or(d.colors.background),
                palette,
                cursor: c.cursor,
                selection: match (c.selection_foreground, c.selection_background) {
                    (Some(foreground), Some(background)) => Some(SelectionColors {
                        foreground,
                        background,
                    }),
                    _ => None,
                },
            },
        }
    }
}
