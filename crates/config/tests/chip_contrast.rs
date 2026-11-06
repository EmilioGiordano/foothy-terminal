//! WCAG contrast of the prompt chip over every bundled theme.
//!
//! Run it for the numbers:
//!
//! ```sh
//! cargo test -p config --test chip_contrast -- --nocapture
//! ```

use config::{
    ChipColors, ChipStyle, Colors, Partial, PartialColors, SENTINEL_BRANCH, SENTINEL_BRANCH_ICON,
    SENTINEL_FOLDER_ICON, SENTINEL_PATH, SENTINEL_PILL, Settings, Variant, bundled, bundled_names,
    chip_aliases, chip_colors, parse_theme,
};

fn luminance(color: u32) -> f64 {
    let channel = |shift: u32| {
        let v = ((color >> shift) & 0xff) as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// What the terminal actually paints: `Pill::Reverse` is the foreground, and an
/// unnamed ink is the background under the reverse and the foreground over a
/// fill.
fn painted(chip: &ChipColors, colors: &Colors) -> (u32, [(&'static str, u32); 4]) {
    let (pill, unnamed) = match chip.pill {
        config::Pill::Fill(c) => (c, colors.foreground),
        config::Pill::Reverse => (colors.foreground, colors.background),
    };
    let ink = |c: Option<u32>| c.unwrap_or(unnamed);
    (
        pill,
        [
            ("path", ink(chip.path)),
            ("branch", ink(chip.branch)),
            ("folder icon", ink(chip.folder_icon)),
            ("branch icon", ink(chip.branch_icon)),
        ],
    )
}

/// The same four inks and the pill, read out of the alias table the renderer
/// is handed.
fn aliased(colors: &Colors, style: ChipStyle) -> (u32, [(&'static str, u32); 4]) {
    let table = chip_aliases(colors, style);
    let of = |key: u32| {
        table
            .iter()
            .find(|(k, _)| *k == key)
            .expect("no such sentinel")
            .1
    };
    (
        of(SENTINEL_PILL),
        [
            ("path", of(SENTINEL_PATH)),
            ("branch", of(SENTINEL_BRANCH)),
            ("folder icon", of(SENTINEL_FOLDER_ICON)),
            ("branch icon", of(SENTINEL_BRANCH_ICON)),
        ],
    )
}

struct Pair {
    name: String,
    colors: Colors,
    had_foreground: bool,
    had_background: bool,
}

/// One entry per non-empty colour section, with `initial-color-theme` cleared:
/// `variant_for` honours it over its argument and flips when a side is empty,
/// so resolving both variants of every theme would double-count.
fn pairs() -> Vec<Pair> {
    let mut out = Vec::new();
    for name in bundled_names() {
        let (theme, problems) = parse_theme(bundled(name).unwrap());
        assert!(problems.is_empty(), "{name}: {problems:?}");
        for (variant, colors) in [(Variant::Dark, &theme.dark), (Variant::Light, &theme.light)] {
            if colors.is_empty() {
                continue;
            }
            let one = Partial {
                settings: Settings::default(),
                dark: match variant {
                    Variant::Dark => colors.clone(),
                    Variant::Light => PartialColors::default(),
                },
                light: match variant {
                    Variant::Light => colors.clone(),
                    Variant::Dark => PartialColors::default(),
                },
            };
            let resolved = one.resolve(variant);
            assert_eq!(resolved.variant, variant, "{name}");
            out.push(Pair {
                name: format!("{name} {variant:?}").to_lowercase(),
                colors: resolved.colors,
                had_foreground: colors.foreground.is_some(),
                had_background: colors.background.is_some(),
            });
        }
    }
    out
}

struct Floor {
    ratio: f64,
    who: String,
}

impl Floor {
    fn new() -> Self {
        Self {
            ratio: f64::INFINITY,
            who: String::new(),
        }
    }

    fn see(&mut self, ratio: f64, who: &str) {
        if ratio < self.ratio {
            self.ratio = ratio;
            self.who = who.to_string();
        }
    }
}

#[test]
fn the_chip_contrast_floors_over_every_bundled_theme() {
    let pairs = pairs();
    println!("\n{} theme/variant pairs", pairs.len());
    for p in &pairs {
        if !p.had_foreground || !p.had_background {
            println!(
                "  {}: no {}{}{} of its own, falling back to config's default",
                p.name,
                if p.had_foreground { "" } else { "foreground" },
                if p.had_foreground || p.had_background {
                    ""
                } else {
                    " and "
                },
                if p.had_background { "" } else { "background" },
            );
        }
    }

    for style in [ChipStyle::Surface, ChipStyle::Inverse] {
        let mut pill = Floor::new();
        let mut inks = [Floor::new(), Floor::new(), Floor::new(), Floor::new()];
        let mut chevron = Floor::new();
        let mut labels = ["", "", "", ""];
        for p in &pairs {
            let chip = chip_colors(&p.colors, style);
            let (fill, painted_inks) = aliased(&p.colors, style);
            pill.see(contrast(fill, p.colors.background), &p.name);
            for (i, (what, color)) in painted_inks.into_iter().enumerate() {
                labels[i] = what;
                inks[i].see(contrast(color, fill), &p.name);
            }
            chevron.see(contrast(chip.chevron, p.colors.background), &p.name);
        }
        println!("\n{style:?}");
        println!(
            "  {:<26} {:.2}:1 ({})",
            "pill on the bg:", pill.ratio, pill.who
        );
        for (label, floor) in labels.iter().zip(&inks) {
            println!(
                "  {:<26} {:.2}:1 ({})",
                format!("{label} on the pill:"),
                floor.ratio,
                floor.who
            );
        }
        println!(
            "  {:<26} {:.2}:1 ({})",
            "chevron on the bg:", chevron.ratio, chevron.who
        );
    }
    println!();
}

/// The alias table is what the screen shows now, so a sentinel resolving to
/// anything but what the old explicit colours painted is a silent recolour of
/// every theme at once.
#[test]
fn the_alias_table_resolves_to_what_the_chip_used_to_print() {
    for p in &pairs() {
        for style in [ChipStyle::Surface, ChipStyle::Inverse] {
            let chip = chip_colors(&p.colors, style);
            assert_eq!(
                aliased(&p.colors, style),
                painted(&chip, &p.colors),
                "{} {style:?}",
                p.name
            );
        }
    }
}

#[test]
fn contrast_matches_the_wcag_extremes() {
    assert!((contrast(0x00ff_ffff, 0x0000_0000) - 21.0).abs() < 0.01);
    assert!((contrast(0x0012_3456, 0x0012_3456) - 1.0).abs() < 1e-9);
}
