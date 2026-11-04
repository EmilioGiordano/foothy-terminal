//! The only layer allowed to know where the files live.

use std::path::PathBuf;

use config::{Blink, Config, Partial, PartialColors, Settings, Variant, bundled, merge, parse};

pub const FONT_PX: f32 = 16.0;
// foot's own default is pad=0x0 (doc/foot.ini.5.scd).
pub const PAD: u32 = 0;
/// foothy's identity, not foot's: layer 3 supplies it as the bottom layer so a
/// user with no foothy.ini sees these and not config's 242424/ffffff.
pub const BG: u32 = 0x0016_2a33;
pub const FG: u32 = 0x00cf_ecf7;
pub const ALPHA: u8 = 235;

pub const USER_FILE: &str = "foothy.ini";
pub const STATE_FILE: &str = "state.ini";

pub struct Loaded {
    pub config: Config,
    pub problems: Vec<String>,
}

fn dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("foothy"))
}

/// `None` for a missing file, which is the normal case; any other IO error is
/// a problem to report, not a reason to refuse to start.
fn slurp(path: &std::path::Path, problems: &mut Vec<String>) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            problems.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

/// Reads `%APPDATA%\foothy`. Never writes: foothy.ini is the user's.
pub fn load_from_disk() -> Loaded {
    let io = std::cell::RefCell::new(Vec::new());
    let dir = dir();
    let read = |name: &str| -> Option<String> {
        let dir = dir.as_ref()?;
        slurp(&dir.join(name), &mut io.borrow_mut())
    };
    let theme = |name: &str| -> Option<String> {
        let dir = dir.as_ref()?;
        slurp(&dir.join("themes").join(name), &mut io.borrow_mut())
    };
    let mut out = load(read, theme, |k| std::env::var(k).ok());
    out.problems.splice(0..0, io.into_inner());
    out
}

/// `app defaults < state.ini < foothy.ini < FOOTWIN_*`, with a named theme
/// merged under every user layer so an explicit `background=` still wins.
pub fn load(
    read: impl Fn(&str) -> Option<String>,
    user_theme: impl Fn(&str) -> Option<String>,
    env: impl Fn(&str) -> Option<String>,
) -> Loaded {
    let mut problems = Vec::new();
    let layer = |name: &str, problems: &mut Vec<String>| match read(name) {
        Some(text) => {
            let (partial, found) = parse(&text);
            for p in found {
                problems.push(format!("{name}:{}: {}", p.line, p.message));
            }
            partial
        }
        None => Partial::default(),
    };

    let state = layer(STATE_FILE, &mut problems);
    let user = layer(USER_FILE, &mut problems);
    let files = merge(state, user);

    let theme = match files.settings.theme.as_deref() {
        None => Partial::default(),
        Some(name) => match user_theme(name).or_else(|| bundled(name).map(str::to_string)) {
            Some(text) => {
                let (partial, found) = parse(&text);
                for p in found {
                    problems.push(format!("theme {name}:{}: {}", p.line, p.message));
                }
                from_theme(partial)
            }
            None => {
                problems.push(format!("theme '{name}' not found"));
                Partial::default()
            }
        },
    };

    let files = merge(theme, files);
    // The variant has to be settled before our own colours go in: they would
    // make the dark slot non-empty and hide a theme that only has a light one.
    let variant = files.variant_for(Variant::Dark);
    let resolved = merge(merge(defaults(variant), files), from_env(variant, &env));

    Loaded {
        config: resolved.resolve(Variant::Dark),
        problems,
    }
}

/// A theme may contribute colours and the variant, nothing else. Named field
/// by field on purpose: blanking the forbidden ones would leak the next
/// setting added to `Settings`, such as a `paste-warning=no` turning off the
/// multiline-paste guard without telling the user.
fn from_theme(theme: Partial) -> Partial {
    Partial {
        settings: Settings {
            initial_variant: theme.settings.initial_variant,
            ..Default::default()
        },
        dark: theme.dark,
        light: theme.light,
    }
}

fn into_variant(variant: Variant, colors: PartialColors, settings: Settings) -> Partial {
    let (dark, light) = match variant {
        Variant::Dark => (colors, PartialColors::default()),
        Variant::Light => (PartialColors::default(), colors),
    };
    Partial {
        settings,
        dark,
        light,
    }
}

fn defaults(variant: Variant) -> Partial {
    into_variant(
        variant,
        PartialColors {
            foreground: Some(FG),
            background: Some(BG),
            ..Default::default()
        },
        Settings {
            font_size: Some(FONT_PX),
            pad: Some(PAD),
            alpha: Some(ALPHA),
            ..Default::default()
        },
    )
}

/// Deliberately not routed through the ini parser: these keep exactly the
/// semantics they had before there was a config file.
fn from_env(variant: Variant, env: &impl Fn(&str) -> Option<String>) -> Partial {
    let hex =
        |key: &str| env(key).and_then(|v| u32::from_str_radix(v.trim_start_matches('#'), 16).ok());
    fn num<T: std::str::FromStr>(env: &impl Fn(&str) -> Option<String>, key: &str) -> Option<T> {
        env(key).and_then(|v| v.parse().ok())
    }

    into_variant(
        variant,
        PartialColors {
            foreground: hex("FOOTWIN_FG"),
            background: hex("FOOTWIN_BG"),
            ..Default::default()
        },
        Settings {
            font: env("FOOTWIN_FONT"),
            font_size: num(env, "FOOTWIN_FONT_PX"),
            pad: num(env, "FOOTWIN_PAD"),
            alpha: num(env, "FOOTWIN_ALPHA"),
            blink: num::<u64>(env, "FOOTWIN_BLINK").map(|ms| match ms {
                0 => Blink::Off,
                ms => Blink::Ms(ms),
            }),
            paste_warning: num::<i32>(env, "FOOTWIN_PASTE_WARN").map(|v| v != 0),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    fn only(name: &'static str, text: &'static str) -> impl Fn(&str) -> Option<String> {
        move |k: &str| (k == name).then(|| text.to_string())
    }

    #[test]
    fn with_no_files_the_look_is_unchanged() {
        let c = load(none, none, none).config;
        assert_eq!(c.colors.foreground, 0x00cf_ecf7);
        assert_eq!(c.colors.background, 0x0016_2a33);
        assert_eq!(c.colors.palette, config::default_palette());
        assert_eq!(c.font, None);
        assert_eq!(c.font_size, 16.0);
        assert_eq!(c.pad, 0);
        assert_eq!(c.alpha, 235);
        assert_eq!(c.blink, Blink::System);
        assert!(c.paste_warning);
        assert_eq!(c.variant, Variant::Dark);
    }

    #[test]
    fn the_default_palette_is_foots() {
        // upstream/foot/config.c:33-86
        let p = config::default_palette();
        assert_eq!(p[0], 0x0024_2424);
        assert_eq!(p[1], 0x00f6_2b5a);
        assert_eq!(p[15], 0x00ff_ffff);
        assert_eq!(p[16], 0x0000_0000);
        assert_eq!(p[231], 0x00ff_ffff);
        assert_eq!(p[232], 0x0008_0808);
        assert_eq!(p[255], 0x00ee_eeee);
    }

    #[test]
    fn env_beats_the_user_file_which_beats_state() {
        let read = |name: &str| {
            let text = match name {
                STATE_FILE => "font-size=14\npad=9\nalpha=0.5\n",
                USER_FILE => "font-size=18\npad=3\n",
                _ => return None,
            };
            Some(text.to_string())
        };
        let env = only("FOOTWIN_FONT_PX", "22");
        let c = load(read, none, env).config;
        assert_eq!(c.font_size, 22.0);
        assert_eq!(c.pad, 3);
        assert_eq!(c.alpha, 128);
    }

    #[test]
    fn a_named_bundled_theme_applies() {
        let read = only(USER_FILE, "theme=catppuccin-mocha\n");
        let c = load(read, none, none).config;
        assert_eq!(c.colors.foreground, 0x00cd_d6f4);
        assert_eq!(c.colors.background, 0x001e_1e2e);
    }

    #[test]
    fn a_user_theme_file_wins_over_the_bundled_one_of_that_name() {
        let read = only(USER_FILE, "theme=nord\n");
        let mine = only("nord", "[colors-dark]\nbackground=010203\n");
        let c = load(read, mine, none).config;
        assert_eq!(c.colors.background, 0x0001_0203);
    }

    #[test]
    fn an_unknown_theme_is_reported_and_the_app_still_starts() {
        let read = only(USER_FILE, "theme=does-not-exist\npad=4\n");
        let out = load(read, none, none);
        assert_eq!(out.config.colors.background, BG);
        assert_eq!(out.config.pad, 4);
        assert_eq!(out.problems.len(), 1);
        assert!(
            out.problems[0].contains("does-not-exist"),
            "{:?}",
            out.problems
        );
    }

    #[test]
    fn the_user_file_overrides_the_theme_it_names() {
        let read = only(
            USER_FILE,
            "theme=catppuccin-mocha\n[colors-dark]\nbackground=000000\n",
        );
        let c = load(read, none, none).config;
        assert_eq!(c.colors.background, 0x0000_0000);
        assert_eq!(c.colors.foreground, 0x00cd_d6f4);
    }

    #[test]
    fn footwin_fg_beats_a_theme() {
        let read = only(USER_FILE, "theme=catppuccin-mocha\n");
        let env = only("FOOTWIN_FG", "#ff0000");
        let c = load(read, none, env).config;
        assert_eq!(c.colors.foreground, 0x00ff_0000);
        assert_eq!(c.colors.background, 0x001e_1e2e);
    }

    #[test]
    fn a_light_only_theme_is_not_hidden_by_our_own_colours() {
        let read = only(USER_FILE, "theme=gruvbox-light\n");
        let c = load(read, none, none).config;
        assert_eq!(c.variant, Variant::Light);
        assert_ne!(c.colors.background, BG);
    }

    #[test]
    fn a_theme_with_only_light_colours_still_flips_the_variant() {
        let read = only(USER_FILE, "theme=mine\n");
        let mine = only("mine", "[colors-light]\nbackground=eeeeee\n");
        let c = load(read, mine, none).config;
        assert_eq!(c.variant, Variant::Light);
        assert_eq!(c.colors.background, 0x00ee_eeee);
    }

    #[test]
    fn a_theme_contributes_colours_and_the_variant_and_nothing_else() {
        let read = only(USER_FILE, "theme=hostile\n");
        let theme = only(
            "hostile",
            "[main]\ninitial-color-theme=light\npaste-warning=no\nfont=C:\\evil.ttf\nblink=1\nalpha=0\nfont-size=50\npad=40\ntheme=nord\n[colors-dark]\nbackground=111111\n[colors-light]\nbackground=eeeeee\n",
        );
        let c = load(read, theme, none).config;
        assert_eq!(c.variant, Variant::Light);
        assert_eq!(c.colors.background, 0x00ee_eeee);
        assert!(c.paste_warning);
        assert_eq!(c.font, None);
        assert_eq!(c.blink, Blink::System);
        assert_eq!(c.alpha, ALPHA);
        assert_eq!(c.font_size, FONT_PX);
        assert_eq!(c.pad, PAD);
    }

    #[test]
    fn ferras_dark_variant_and_colours_survive_the_theme_filter() {
        let read = only(USER_FILE, "theme=ferra\n");
        let out = load(read, none, none);
        assert_eq!(out.config.variant, Variant::Dark);
        assert_eq!(out.config.colors.background, 0x002b_292d);
        assert!(out.problems.is_empty(), "{:?}", out.problems);
    }

    #[test]
    fn footwin_alpha_stays_a_byte_not_a_fraction() {
        assert_eq!(load(none, none, only("FOOTWIN_ALPHA", "1")).config.alpha, 1);
    }

    #[test]
    fn a_malformed_line_is_reported_and_the_rest_applies() {
        let read = only(USER_FILE, "font-size=18\nthis is not a setting\npad=2\n");
        let out = load(read, none, none);
        assert_eq!(out.config.font_size, 18.0);
        assert_eq!(out.config.pad, 2);
        assert_eq!(out.problems.len(), 1);
        assert!(
            out.problems[0].starts_with("foothy.ini:2:"),
            "{:?}",
            out.problems
        );
    }

    #[test]
    fn footwin_blink_zero_turns_it_off() {
        assert_eq!(
            load(none, none, only("FOOTWIN_BLINK", "0")).config.blink,
            Blink::Off
        );
        assert_eq!(
            load(none, none, only("FOOTWIN_BLINK", "700")).config.blink,
            Blink::Ms(700)
        );
        assert_eq!(
            load(none, none, only("FOOTWIN_BLINK", "x")).config.blink,
            Blink::System
        );
    }
}
