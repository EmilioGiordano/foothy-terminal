use config::{PartialColors, Settings, Variant, bundled, into_variant, parse_theme};

#[test]
fn a_theme_contributes_colours_and_the_variant_and_nothing_else() {
    let (p, problems) = parse_theme(
        "[main]\ninitial-color-theme=light\npaste-warning=no\nprompt=no\nfont=C:\\evil.ttf\nblink=1\nalpha=0\nfont-size=50\npad=40\ntheme=nord\n[colors-dark]\nbackground=111111\n[colors-light]\nbackground=eeeeee\n",
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        p.settings,
        Settings {
            initial_variant: Some(Variant::Light),
            ..Default::default()
        }
    );
    assert_eq!(p.dark.background, Some(0x0011_1111));
    assert_eq!(p.light.background, Some(0x00ee_eeee));
}

#[test]
fn a_bundled_theme_keeps_its_variant_and_colours() {
    let (p, problems) = parse_theme(bundled("ferra").unwrap());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(p.variant_for(Variant::Dark), Variant::Dark);
    assert_eq!(p.dark.background, Some(0x002b_292d));
}

#[test]
fn a_malformed_theme_line_is_still_reported() {
    let (_, problems) = parse_theme("this is not a setting\n");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line, 1);
}

#[test]
fn colours_land_in_the_slot_of_the_variant_they_were_built_for() {
    let colors = PartialColors {
        background: Some(0x0012_3456),
        ..Default::default()
    };
    let light = into_variant(Variant::Light, colors.clone(), Settings::default());
    assert!(light.dark.is_empty());
    assert_eq!(light.light.background, Some(0x0012_3456));

    let dark = into_variant(Variant::Dark, colors, Settings::default());
    assert!(dark.light.is_empty());
    assert_eq!(dark.dark.background, Some(0x0012_3456));
}
