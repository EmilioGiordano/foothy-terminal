use config::{Variant, bundled, bundled_names, parse};

#[test]
fn the_whole_catalogue_is_embedded() {
    assert_eq!(bundled_names().len(), 86);
    assert!(bundled_names().windows(2).all(|w| w[0] < w[1]));
    assert!(bundled("catppuccin-mocha").is_some());
    assert!(bundled("README.md").is_none());
    assert!(bundled("LICENSE").is_none());
    assert!(bundled("no-such-theme").is_none());
}

#[test]
fn every_bundled_theme_parses_without_a_problem() {
    for name in bundled_names() {
        let (_, problems) = parse(bundled(name).unwrap());
        assert!(problems.is_empty(), "{name}: {problems:?}");
    }
}

#[test]
fn every_bundled_theme_yields_a_variant_with_all_sixteen_colours() {
    for name in bundled_names() {
        let (theme, _) = parse(bundled(name).unwrap());
        assert!(
            theme.dark.has_all_16() || theme.light.has_all_16(),
            "{name} defines neither full regular0-7 nor bright0-7"
        );
    }
}

#[test]
fn every_bundled_theme_resolves_for_either_request() {
    for name in bundled_names() {
        let (theme, _) = parse(bundled(name).unwrap());
        for wanted in [Variant::Dark, Variant::Light] {
            let c = theme.resolve(wanted);
            assert_ne!(
                c.colors.foreground, c.colors.background,
                "{name} resolved to an unreadable screen for {wanted:?}"
            );
        }
    }
}

#[test]
fn the_catalogue_splits_into_sixty_two_dark_seventeen_light_and_seven_both() {
    let mut dark_only = 0;
    let mut light_only = 0;
    let mut both = 0;
    for name in bundled_names() {
        let (theme, _) = parse(bundled(name).unwrap());
        match (theme.dark.is_empty(), theme.light.is_empty()) {
            (false, true) => dark_only += 1,
            (true, false) => light_only += 1,
            (false, false) => both += 1,
            (true, true) => panic!("{name} defines no colours at all"),
        }
    }
    assert_eq!((dark_only, light_only, both), (62, 17, 7));
}

#[test]
fn catppuccin_mocha_matches_the_file() {
    let (theme, problems) = parse(bundled("catppuccin-mocha").unwrap());
    assert!(problems.is_empty());
    let c = theme.resolve(Variant::Dark);
    assert_eq!(c.colors.background, 0x1e_1e2e);
    assert_eq!(c.colors.foreground, 0xcd_d6f4);
    assert_eq!(c.colors.regular()[1], 0xf3_8ba8);
    assert_eq!(c.colors.bright()[0], 0x58_5b70);
    assert_eq!(c.colors.palette[16], 0xfa_b387);
    assert_eq!(c.colors.palette[17], 0xf5_e0dc);
    // cursor=11111b f5e0dc, text first (upstream/foot/config.c:1575).
    let cursor = c.colors.cursor.unwrap();
    assert_eq!((cursor.text, cursor.cursor), (0x11_111b, 0xf5_e0dc));
    let selection = c.colors.selection.unwrap();
    assert_eq!(selection.background, 0x41_4356);
}

#[test]
fn a_light_only_theme_still_gives_colours_when_dark_is_asked() {
    let (theme, _) = parse(bundled("catppuccin-latte").unwrap());
    assert!(theme.dark.is_empty());
    assert_eq!(theme.variant_for(Variant::Dark), Variant::Light);
    let c = theme.resolve(Variant::Dark);
    assert_eq!(c.variant, Variant::Light);
    assert_eq!(c.colors.background, 0xef_f1f5);
}

#[test]
fn a_theme_with_both_variants_honours_the_request() {
    let (theme, _) = parse(bundled("selenized").unwrap());
    assert!(!theme.dark.is_empty() && !theme.light.is_empty());
    assert!(theme.settings.initial_variant.is_none());
    let dark = theme.resolve(Variant::Dark);
    let light = theme.resolve(Variant::Light);
    assert_ne!(dark.colors.background, light.colors.background);
}

#[test]
fn a_themes_own_initial_color_theme_overrides_the_request() {
    let (theme, _) = parse(bundled("ferra").unwrap());
    assert_eq!(theme.settings.initial_variant, Some(Variant::Dark));
    assert_eq!(theme.variant_for(Variant::Light), Variant::Dark);
}
