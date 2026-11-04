use config::{Blink, Variant, parse};

fn clean(text: &str) -> config::Partial {
    let (partial, problems) = parse(text);
    assert!(problems.is_empty(), "{problems:?}");
    partial
}

#[test]
fn whitespace_around_the_equals_sign_is_allowed() {
    let p = clean("[colors-dark]\nregular0 = 3b4252\n   background\t=\t2e3440   \n");
    assert_eq!(p.dark.palette[0], Some(0x3b_4252));
    assert_eq!(p.dark.background, Some(0x2e_3440));
}

#[test]
fn a_trailing_comment_is_stripped_from_the_value() {
    let p = clean("[colors-dark]\nregular0=21222c # black\nforeground=f8f8f2\t# fg\n");
    assert_eq!(p.dark.palette[0], Some(0x21_222c));
    assert_eq!(p.dark.foreground, Some(0xf8_f8f2));
}

#[test]
fn a_hash_without_blank_space_before_it_stays_in_the_value() {
    let p = clean("[main]\nfont=C:\\Fonts\\C#Mono.ttf\n");
    assert_eq!(p.settings.font.as_deref(), Some("C:\\Fonts\\C#Mono.ttf"));
}

#[test]
fn crlf_line_endings_parse() {
    let p = clean("[colors-dark]\r\nregular0 = 3b4252\r\ncursor=11111b f5e0dc\r\n");
    assert_eq!(p.dark.palette[0], Some(0x3b_4252));
    assert_eq!(p.dark.cursor.unwrap().text, 0x11_111b);
}

#[test]
fn commented_out_keys_stay_ignored() {
    let p = clean("[colors-dark]\n# selection-foreground = d8dee9\n   # background=000000\n");
    assert!(p.dark.is_empty());
}

#[test]
fn unknown_keys_are_ignored_rather_than_fatal() {
    let p = clean(
        "[main]\nbell=none\n[colors-dark]\ndim0=3b4252\nflash=ffff00\nurls=89b4fa\n\
         search-box-match=cdd6f4 313244\njump-labels=11111b fab387\nbackground=1e1e2e\n",
    );
    assert_eq!(p.dark.background, Some(0x1e_1e2e));
}

#[test]
fn an_unknown_section_drops_its_keys_without_a_problem() {
    let p =
        clean("[key-bindings]\nscrollback-up-page=Control+Shift+Page_Up\n[cursor]\nstyle=beam\n");
    assert_eq!(p, config::Partial::default());
}

#[test]
fn keys_before_any_header_belong_to_main() {
    let p = clean("font-size=14\npad=3\n");
    assert_eq!(p.settings.font_size, Some(14.0));
    assert_eq!(p.settings.pad, Some(3));
}

#[test]
fn numeric_keys_address_the_palette() {
    let p = clean("[colors-dark]\n16=fab387\n17=f5e0dc\n255=ffffff\n");
    assert_eq!(p.dark.palette[16], Some(0xfa_b387));
    assert_eq!(p.dark.palette[17], Some(0xf5_e0dc));
    assert_eq!(p.dark.palette[255], Some(0xff_ffff));
}

#[test]
fn a_palette_index_past_the_table_is_a_problem() {
    let (_, problems) = parse("[colors-dark]\n256=ffffff\n");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line, 2);
}

#[test]
fn regular_and_bright_fill_the_first_sixteen_slots() {
    let mut text = String::from("[colors-dark]\n");
    for i in 0..8 {
        text.push_str(&format!("regular{i}=00000{i}\nbright{i}=10000{i}\n"));
    }
    let p = clean(&text);
    assert!(p.dark.has_all_16());
    assert_eq!(p.dark.palette[7], Some(0x7));
    assert_eq!(p.dark.palette[8], Some(0x10_0000));
    assert_eq!(p.dark.palette[15], Some(0x10_0007));
}

#[test]
fn regular8_and_beyond_are_not_colour_slots() {
    let p = clean("[colors-dark]\nregular8=ffffff\nregular10=ffffff\nbrightly=ffffff\n");
    assert!(p.dark.is_empty());
}

#[test]
fn the_deprecated_colors_sections_map_onto_the_variants() {
    let p = clean("[colors]\nbackground=111111\n[colors2]\nbackground=eeeeee\n");
    assert_eq!(p.dark.background, Some(0x11_1111));
    assert_eq!(p.light.background, Some(0xee_eeee));
}

#[test]
fn initial_color_theme_light_selects_the_light_variant() {
    let p = clean(
        "[main]\ninitial-color-theme=light\n[colors-dark]\nbackground=111111\n\
         [colors-light]\nbackground=eeeeee\n",
    );
    assert_eq!(p.variant_for(Variant::Dark), Variant::Light);
    assert_eq!(p.resolve(Variant::Dark).colors.background, 0xee_eeee);
}

#[test]
fn blink_takes_system_off_or_milliseconds() {
    assert_eq!(clean("blink=system\n").settings.blink, Some(Blink::System));
    assert_eq!(clean("blink=off\n").settings.blink, Some(Blink::Off));
    assert_eq!(clean("blink=0\n").settings.blink, Some(Blink::Off));
    assert_eq!(clean("blink=530\n").settings.blink, Some(Blink::Ms(530)));
    assert_eq!(parse("blink=sometimes\n").1.len(), 1);
}

#[test]
fn paste_warning_uses_foots_boolean_vocabulary() {
    for yes in ["yes", "on", "true", "1", "YES"] {
        let p = clean(&format!("paste-warning={yes}\n"));
        assert_eq!(p.settings.paste_warning, Some(true), "{yes}");
    }
    for no in ["no", "off", "false", "0"] {
        let p = clean(&format!("paste-warning={no}\n"));
        assert_eq!(p.settings.paste_warning, Some(false), "{no}");
    }
    assert_eq!(parse("paste-warning=maybe\n").1.len(), 1);
}

#[test]
fn alpha_reads_both_foots_fraction_and_our_byte() {
    assert_eq!(clean("alpha=1.0\n").settings.alpha, Some(255));
    assert_eq!(clean("alpha=1\n").settings.alpha, Some(255));
    assert_eq!(clean("alpha=0\n").settings.alpha, Some(0));
    assert_eq!(clean("alpha=0.5\n").settings.alpha, Some(128));
    assert_eq!(clean("alpha=235\n").settings.alpha, Some(235));
    assert_eq!(parse("alpha=256\n").1.len(), 1);
}

#[test]
fn alpha_is_also_accepted_inside_a_colour_section() {
    let p = clean("[colors-dark]\nalpha=0.9\nbackground=1e1e2e\n");
    assert_eq!(p.settings.alpha, Some(230));
}

#[test]
fn a_pad_pair_keeps_its_first_number() {
    assert_eq!(clean("pad=4x8 center\n").settings.pad, Some(4));
    assert_eq!(clean("pad=6\n").settings.pad, Some(6));
}

#[test]
fn a_non_ascii_font_path_survives() {
    let p = clean("[main]\nfont=C:\\Fuentes\\Año.ttf\n");
    assert_eq!(p.settings.font.as_deref(), Some("C:\\Fuentes\\Año.ttf"));
}

#[test]
fn an_empty_file_yields_nothing() {
    assert_eq!(clean(""), config::Partial::default());
    assert_eq!(clean("\n\n   \n\t\n"), config::Partial::default());
}

#[test]
fn a_section_with_no_body_yields_nothing() {
    assert_eq!(
        clean("[colors-dark]\n[colors-light]\n[main]"),
        config::Partial::default()
    );
}

#[test]
fn a_line_without_an_equals_sign_is_a_problem_but_not_fatal() {
    let (p, problems) = parse("[colors-dark]\nnonsense\nbackground=1e1e2e\n");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line, 2);
    assert_eq!(p.dark.background, Some(0x1e_1e2e));
}

#[test]
fn an_unclosed_section_header_is_a_problem_but_not_fatal() {
    let (p, problems) = parse("[colors-dark\nbackground=1e1e2e\n");
    assert_eq!(problems.len(), 1);
    // The header was rejected, so the key fell to the section in force.
    assert!(p.dark.is_empty());
}

#[test]
fn colours_that_are_not_six_hex_digits_are_rejected() {
    for bad in ["fff", "ffffffff", "zzzzzz", "", "1e1e2", "0x1e1e2e"] {
        let (p, problems) = parse(&format!("[colors-dark]\nbackground={bad}\n"));
        assert_eq!(problems.len(), 1, "{bad:?} was accepted");
        assert!(p.dark.background.is_none(), "{bad:?}");
    }
}

#[test]
fn a_cursor_pair_needs_exactly_two_colours() {
    assert_eq!(parse("[colors-dark]\ncursor=11111b\n").1.len(), 1);
    assert_eq!(
        parse("[colors-dark]\ncursor=11111b f5e0dc cdd6f4\n")
            .1
            .len(),
        1
    );
    assert!(
        clean("[colors-dark]\ncursor=11111b   f5e0dc\n")
            .dark
            .cursor
            .is_some()
    );
}

#[test]
fn an_empty_key_is_a_problem() {
    assert_eq!(parse("=ffffff\n").1.len(), 1);
}

#[test]
fn a_huge_line_does_not_panic() {
    let text = format!("[colors-dark]\nbackground={}\n", "f".repeat(100_000));
    assert_eq!(parse(&text).1.len(), 1);
    let long_key = format!("{}=ffffff\n", "k".repeat(100_000));
    assert!(parse(&long_key).1.is_empty());
}

#[test]
fn defaults_fill_everything_a_file_leaves_out() {
    let c = clean("[colors-dark]\nbackground=1e1e2e\n").resolve(Variant::Dark);
    let d = config::Config::default();
    assert_eq!(c.colors.background, 0x1e_1e2e);
    assert_eq!(c.colors.foreground, d.colors.foreground);
    assert_eq!(c.colors.palette[1..], d.colors.palette[1..]);
    assert_eq!(c.font_size, d.font_size);
    assert_eq!(c.alpha, d.alpha);
    assert_eq!(c.blink, Blink::System);
    assert!(c.paste_warning);
    assert!(c.font.is_none());
}
