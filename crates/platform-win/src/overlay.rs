use config::{Config, Partial, Variant};
use render::Renderer;

use crate::config_file;
use crate::palette::{Half, Mode, Palette, ThemeEntry};

/// Enough cells to recognise a theme: background, foreground and four hues.
pub const SWATCH: usize = 6;
pub const MAX_COLS: usize = 76;
const MAX_ROWS: usize = 18;
const CHROME_LINES: usize = 2;

/// Built once and kept: ~86 files, and work per event-loop turn is bounded
/// (docs/02 §3 rule 7). The resolved layer is kept so a preview never re-reads.
pub struct ThemeList {
    pub entries: Vec<ThemeEntry>,
    pub layers: Vec<Partial>,
    pub swatches: Vec<[u32; SWATCH]>,
}

impl ThemeList {
    pub fn build() -> (Self, Vec<String>) {
        let user = config_file::user_theme_names();
        let read = |name: &str| config_file::read_user_theme(name);
        let mut list = Self {
            entries: Vec::new(),
            layers: Vec::new(),
            swatches: Vec::new(),
        };
        let mut problems = Vec::new();
        // Resolving each theme to a full Config would rebuild the 256-colour
        // palette 86 times, which is most of the cost of opening the palette.
        let fallback = config::default_palette();
        for name in config_file::theme_names(&user) {
            let (layer, found) = config_file::theme_layer(&name, &read);
            problems.extend(found);
            let variant = layer.variant_for(Variant::Dark);
            let c = layer.colors_for(Variant::Dark);
            let slot = |i: usize| c.palette[i].unwrap_or(fallback[i]);
            list.swatches.push([
                c.background.unwrap_or(config_file::BG),
                c.foreground.unwrap_or(config_file::FG),
                slot(1),
                slot(2),
                slot(3),
                slot(4),
            ]);
            list.layers.push(layer);
            list.entries.push(ThemeEntry { name, variant });
        }
        (list, problems)
    }
}

pub struct Layout {
    /// Visible list rows. The same number must reach `visible` and `marker`.
    pub rows: usize,
    /// Cells across the inside, the marker track included.
    pub cols: usize,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub text_x: usize,
    pub filter_y: usize,
    pub rows_y: usize,
    pub footer_y: usize,
}

/// `None` when the window is too small to hold a usable box: half a palette is
/// worse than none.
pub fn layout(
    win: (usize, usize),
    cell: (usize, usize),
    notes: usize,
    wanted_rows: usize,
) -> Option<Layout> {
    let (w, h) = win;
    let (cw, ch) = cell;
    if cw == 0 || ch == 0 {
        return None;
    }
    let lines_chrome = CHROME_LINES + notes;
    let cols = (w / cw).saturating_sub(4).min(MAX_COLS);
    let fits = (h / ch).saturating_sub(lines_chrome + 2).min(MAX_ROWS);
    let rows = fits.min(wanted_rows.max(1));
    if cols < 16 || rows == 0 {
        return None;
    }

    let pad_x = cw;
    let pad_y = (ch / 2).max(2);
    let box_w = cols * cw + 2 * pad_x;
    let box_h = (rows + lines_chrome) * ch + 2 * pad_y;
    let x = w.saturating_sub(box_w) / 2;
    // Placed as if the list were full, so the filter line does not move up the
    // window as typing narrows the list.
    let y = h.saturating_sub((fits + lines_chrome) * ch + 2 * pad_y) / 2;
    Some(Layout {
        rows,
        cols,
        x,
        y,
        w: box_w,
        h: box_h,
        text_x: x + pad_x,
        filter_y: y + pad_y,
        rows_y: y + pad_y + ch,
        footer_y: y + pad_y + (rows + 1) * ch,
    })
}

struct Look {
    bg: u32,
    fg: u32,
    dim: u32,
    sel_bg: u32,
    sel_fg: u32,
}

/// Every colour comes from the resolved config: the overlay is part of the
/// theme, and a constant would clash with half the catalogue.
fn look(cfg: &Config) -> Look {
    let (sel_fg, sel_bg) = match cfg.colors.selection {
        Some(s) => (s.foreground, s.background),
        None => (cfg.colors.background, cfg.colors.foreground),
    };
    Look {
        bg: cfg.colors.background,
        fg: cfg.colors.foreground,
        dim: cfg.colors.palette[8],
        sel_bg,
        sel_fg,
    }
}

fn footer(mode: Mode) -> &'static str {
    match mode {
        Mode::Commands => "Enter: run   Esc: close   Type to filter",
        Mode::Themes => "Enter: keep   Esc: restore   Up/Down: preview",
    }
}

fn prompt(mode: Mode) -> &'static str {
    match mode {
        Mode::Commands => "Command> ",
        Mode::Themes => "Theme> ",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    r: &mut Renderer,
    buf: &mut [u32],
    win: (usize, usize),
    p: &Palette,
    list: &ThemeList,
    cfg: &Config,
    notes: &[String],
) {
    let (w, h) = win;
    let (cw, ch) = r.cell_size();
    // Ask for one row per match so a short list does not leave a hole.
    let wanted = p.visible(MAX_ROWS).len();
    let Some(l) = layout(win, (cw, ch), notes.len(), wanted) else {
        return;
    };
    let look = look(cfg);
    let rows = p.visible(l.rows);
    let marker = p.marker(l.rows);

    r.fill_area(buf, w, h, (l.x, l.y, l.w, l.h), look.bg);
    r.outline_area(buf, w, h, (l.x, l.y, l.w, l.h), look.fg, 255);

    let inner_w = l.cols * cw;
    let text_x = l.text_x;
    // The last two cell columns hold the marker track and the gap before it,
    // so no label or tag runs under the marker.
    let list_cols = l.cols.saturating_sub(2);

    r.draw_text(
        buf,
        w,
        h,
        text_x,
        l.filter_y,
        inner_w,
        &format!("{}{}_", prompt(p.mode()), p.filter()),
        look.fg,
        look.bg,
    );
    r.fill_area(buf, w, h, (text_x, l.rows_y - 1, inner_w, 1), look.dim);

    let track_w = (cw / 3).max(2);
    let track_x = text_x + (l.cols - 1) * cw + cw.saturating_sub(track_w) / 2;
    if marker.is_some() {
        r.fill_area(
            buf,
            w,
            h,
            (track_x, l.rows_y, track_w, l.rows * ch),
            look.dim,
        );
    }

    for (n, row) in rows.iter().enumerate() {
        let y = l.rows_y + n * ch;
        let (fg, bg) = if row.selected {
            (look.sel_fg, look.sel_bg)
        } else {
            (look.fg, look.bg)
        };
        if row.selected {
            r.fill_area(buf, w, h, (text_x, y, list_cols * cw, ch), bg);
        }

        let tag_cols = row.tag.chars().count();
        let tag_x = text_x + list_cols.saturating_sub(tag_cols) * cw;
        let mut label_end = tag_x;

        if let Some(i) = row.theme {
            let swatch_x = tag_x.saturating_sub((SWATCH + 1) * cw);
            for (k, color) in list.swatches[i].iter().enumerate() {
                r.fill_area(
                    buf,
                    w,
                    h,
                    (swatch_x + k * cw, y + 1, cw, ch.saturating_sub(2)),
                    *color,
                );
            }
            label_end = swatch_x;
        }

        let label_w = label_end.saturating_sub(text_x + cw);
        r.draw_text(buf, w, h, text_x + cw, y, label_w, row.label, fg, bg);
        r.draw_text(buf, w, h, tag_x, y, tag_cols * cw, row.tag, fg, bg);
    }

    if let Some(m) = marker {
        let y = l.rows_y + m.row * ch + if m.half == Half::Bottom { ch / 2 } else { 0 };
        // A rectangle, never a half-block glyph: `font=` may point anywhere.
        r.fill_area(buf, w, h, (track_x, y, track_w, (ch / 2).max(1)), look.fg);
    }

    r.fill_area(buf, w, h, (text_x, l.footer_y, inner_w, 1), look.dim);
    r.draw_text(
        buf,
        w,
        h,
        text_x,
        l.footer_y,
        inner_w,
        footer(p.mode()),
        look.dim,
        look.bg,
    );
    for (n, note) in notes.iter().enumerate() {
        r.draw_text(
            buf,
            w,
            h,
            text_x,
            l.footer_y + (n + 1) * ch,
            inner_w,
            note,
            look.fg,
            look.bg,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_fits_inside_the_window_and_leaves_room_for_every_line() {
        for notes in 0..3 {
            let l = layout((1200, 800), (9, 19), notes, 60).unwrap();
            assert!(l.x + l.w <= 1200, "notes {notes}");
            assert!(l.y + l.h <= 800, "notes {notes}");
            assert!(l.footer_y + (notes + 1) * 19 <= l.y + l.h, "notes {notes}");
            assert_eq!(l.rows_y, l.filter_y + 19);
            assert_eq!(l.footer_y, l.rows_y + l.rows * 19);
        }
    }

    #[test]
    fn a_tiny_window_draws_nothing_rather_than_half_a_palette() {
        assert!(layout((80, 60), (9, 19), 0, 60).is_none());
        assert!(layout((1200, 40), (9, 19), 0, 60).is_none());
        assert!(layout((1200, 800), (0, 0), 0, 60).is_none());
    }

    #[test]
    fn a_short_list_shrinks_the_box_instead_of_leaving_holes() {
        let l = layout((1200, 800), (9, 19), 0, 3).unwrap();
        assert_eq!(l.rows, 3);
    }

    #[test]
    fn every_string_the_palette_draws_is_ascii() {
        for mode in [Mode::Commands, Mode::Themes] {
            assert!(footer(mode).is_ascii());
            assert!(prompt(mode).is_ascii());
        }
    }

    #[test]
    fn the_catalogue_holds_every_bundled_theme_with_a_swatch() {
        // Problems are not asserted away: a broken theme in the user's own
        // directory must not fail the suite.
        let (list, _) = ThemeList::build();
        assert!(list.entries.len() >= config::bundled_names().len());
        assert_eq!(list.entries.len(), list.swatches.len());
        assert_eq!(list.entries.len(), list.layers.len());
        let at = |name: &str| list.entries.iter().position(|e| e.name == name).unwrap();
        let i = at("gruvbox-light");
        assert_eq!(list.entries[i].variant, Variant::Light);
        let mocha = at("catppuccin-mocha");
        assert_eq!(list.swatches[mocha][0], 0x001e_1e2e);
        assert_eq!(list.swatches[mocha][1], 0x00cd_d6f4);
    }

    fn renderer() -> Option<Renderer> {
        let bytes = std::fs::read(r"C:\Windows\Fonts\CascadiaMono.ttf").ok()?;
        Renderer::new(&bytes, 16.0).ok()
    }

    /// Drawing is where an index can go out of range: the swatch, the marker
    /// and the right-aligned tag all derive their x from the box width.
    #[test]
    fn drawing_paints_inside_the_box_and_never_panics() {
        let Some(mut r) = renderer() else {
            return;
        };
        let (list, _) = ThemeList::build();
        let cfg = Config::default();
        let (w, h) = (1200usize, 800usize);

        for win in [(w, h), (300, 240), (80, 60), (1, 1)] {
            let mut buf = vec![0u32; win.0 * win.1];
            let mut p = Palette::commands(list.entries.clone(), Some("nord"));
            draw(&mut r, &mut buf, win, &p, &list, &cfg, &[]);
            p.key(crate::palette::Key::Enter);
            for _ in 0..90 {
                p.key(crate::palette::Key::Down);
            }
            draw(
                &mut r,
                &mut buf,
                win,
                &p,
                &list,
                &cfg,
                &["a note".to_string()],
            );
            if win == (w, h) {
                let l = layout(win, r.cell_size(), 1, MAX_ROWS).unwrap();
                let inside = buf[(l.y + l.h / 2) * win.0 + l.x + l.w / 2];
                assert_ne!(inside, 0, "the box was not painted");
                assert_eq!(buf[0], 0, "painted outside the box");
            }
        }
    }

    /// Lets a human look at the overlay without opening a window:
    /// `cargo test -p platform-win dump_the_palette -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_the_palette() {
        let mut r = renderer().expect("font");
        let (list, _) = ThemeList::build();
        let (w, h) = (1000usize, 620usize);
        let mut buf = vec![0u32; w * h];
        let mut p = Palette::commands(list.entries.clone(), Some("nord"));
        p.key(crate::palette::Key::Enter);
        let i = list
            .entries
            .iter()
            .position(|e| e.name == "nord")
            .unwrap_or(0);
        let cfg = config_file::compose(&Partial::default(), list.layers[i].clone(), &|_| None);
        r.set_colors(cfg.colors.foreground, cfg.colors.background);
        r.set_palette(&cfg.colors.palette);
        r.clear(&mut buf);
        let notes = config_file::Pins {
            theme_file: true,
            env_fg: true,
            env_bg: true,
        }
        .notes();
        draw(&mut r, &mut buf, (w, h), &p, &list, &cfg, &notes);

        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for px in &buf {
            out.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, *px as u8]);
        }
        let path = std::env::temp_dir().join("foothy-palette.ppm");
        std::fs::write(&path, out).unwrap();
        println!("wrote {}", path.display());
    }

    /// A theme must not reach the session through the swatch path either.
    #[test]
    fn the_catalogue_keeps_the_filtered_theme_layer() {
        let (list, _) = ThemeList::build();
        for layer in &list.layers {
            assert_eq!(layer.settings.font_size, None);
            assert_eq!(layer.settings.theme, None);
        }
    }
}
