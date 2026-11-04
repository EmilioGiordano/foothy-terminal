use config::{Blink, Variant, bundled, merge, parse};

fn p(text: &str) -> config::Partial {
    parse(text).0
}

#[test]
fn a_field_set_in_the_overlay_wins() {
    let base = p("font-size=14\npad=2\n");
    let over = p("font-size=20\n");
    let m = merge(base, over);
    assert_eq!(m.settings.font_size, Some(20.0));
}

#[test]
fn a_field_absent_from_the_overlay_keeps_the_base() {
    let base = p("font-size=14\npad=2\nblink=off\n");
    let over = p("font-size=20\n");
    let m = merge(base, over);
    assert_eq!(m.settings.pad, Some(2));
    assert_eq!(m.settings.blink, Some(Blink::Off));
}

#[test]
fn colour_slots_merge_one_by_one() {
    let base = p("[colors-dark]\nbackground=111111\nregular0=222222\nregular1=333333\n");
    let over = p("[colors-dark]\nregular1=444444\n");
    let m = merge(base, over);
    assert_eq!(m.dark.background, Some(0x11_1111));
    assert_eq!(m.dark.palette[0], Some(0x22_2222));
    assert_eq!(m.dark.palette[1], Some(0x44_4444));
}

#[test]
fn the_two_variants_do_not_bleed_into_each_other() {
    let base = p("[colors-light]\nbackground=eeeeee\n");
    let over = p("[colors-dark]\nbackground=111111\n");
    let m = merge(base, over);
    assert_eq!(m.light.background, Some(0xee_eeee));
    assert_eq!(m.dark.background, Some(0x11_1111));
}

#[test]
fn merging_is_associative_over_three_layers() {
    let a = p("font-size=14\npad=2\n[colors-dark]\nbackground=111111\nregular0=aaaaaa\n");
    let b = p("font-size=16\n[colors-dark]\nregular0=bbbbbb\nforeground=cccccc\n");
    let c = p("pad=8\n[colors-dark]\nregular0=dddddd\n");

    let left = merge(merge(a.clone(), b.clone()), c.clone());
    let right = merge(a, merge(b, c));
    assert_eq!(left, right);
    assert_eq!(left.settings.font_size, Some(16.0));
    assert_eq!(left.settings.pad, Some(8));
    assert_eq!(left.dark.palette[0], Some(0xdd_dddd));
}

#[test]
fn the_defaults_state_user_env_order_lets_the_user_win() {
    let defaults = config::Partial::default();
    let state = p("theme=nord\nfont-size=14\n");
    let user = p("font-size=18\npaste-warning=no\n");
    let env = p("font-size=22\n");

    let resolved = merge(merge(merge(defaults, state), user), env).resolve(Variant::Dark);
    assert_eq!(resolved.font_size, 22.0);
    assert!(!resolved.paste_warning);
}

#[test]
fn a_user_colour_overrides_the_theme_it_names() {
    let theme = parse(bundled("catppuccin-mocha").unwrap()).0;
    let user = p("[colors-dark]\nbackground=000000\n");
    // The theme sits under the user layers, never over them.
    let c = merge(theme, user).resolve(Variant::Dark);
    assert_eq!(c.colors.background, 0x00_0000);
    assert_eq!(c.colors.foreground, 0xcd_d6f4);
}

#[test]
fn a_user_initial_color_theme_beats_the_themes_own() {
    let theme = parse(bundled("ferra").unwrap()).0;
    assert_eq!(theme.settings.initial_variant, Some(Variant::Dark));
    let user = p("initial-color-theme=light\n[colors-light]\nbackground=eeeeee\n");
    let m = merge(theme, user);
    assert_eq!(m.settings.initial_variant, Some(Variant::Light));
    assert_eq!(m.resolve(Variant::Dark).colors.background, 0xee_eeee);
}
