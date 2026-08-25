use std::collections::HashMap;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use fontdue::{Font, FontSettings};
use vt_core::{TermSize, Terminal};

/// How the cursor is painted this frame. The caller decides, because focus and
/// blink phase live in the platform layer, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorLook {
    Block,
    Beam,
    Underline,
    Hollow,
    Hidden,
}

pub const MIN_FONT_PX: f32 = 6.0;
pub const MAX_FONT_PX: f32 = 72.0;

// foot's own defaults, taken from upstream/foot/config.c:33-68. The man page
// still documents the old Solarized values; the code is what ships.
pub const DEFAULT_FG: u32 = 0x00ff_ffff;
pub const DEFAULT_BG: u32 = 0x0024_2424;

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

fn build_palette() -> [u32; 256] {
    let mut p = [0u32; 256];
    const BASE: [u32; 16] = [
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
    p[..16].copy_from_slice(&BASE);
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    for i in 0..216 {
        p[16 + i] = rgb(STEPS[i / 36], STEPS[(i / 6) % 6], STEPS[i % 6]);
    }
    for i in 0..24 {
        let v = 8 + i as u8 * 10;
        p[232 + i] = rgb(v, v, v);
    }
    p
}

struct Glyph {
    width: usize,
    height: usize,
    left: i32,
    top: i32,
    coverage: Vec<u8>,
}

pub struct Renderer {
    font: Font,
    px: f32,
    cell_w: usize,
    cell_h: usize,
    baseline: i32,
    palette: [u32; 256],
    glyphs: HashMap<char, Glyph>,
    pad: usize,
    fg: u32,
    bg: u32,
}

impl Renderer {
    pub fn new(font_bytes: &[u8], px: f32) -> anyhow::Result<Self> {
        let font = Font::from_bytes(
            font_bytes,
            FontSettings {
                scale: px,
                ..Default::default()
            },
        )
        .map_err(|e| anyhow::anyhow!("fontdue: {e}"))?;

        let (cell_w, cell_h, baseline) = metrics(&font, px)
            .ok_or_else(|| anyhow::anyhow!("font has no horizontal line metrics"))?;

        Ok(Self {
            font,
            px,
            cell_w,
            cell_h,
            baseline,
            palette: build_palette(),
            glyphs: HashMap::new(),
            pad: 0,
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
        })
    }

    pub fn cell_size(&self) -> (usize, usize) {
        (self.cell_w, self.cell_h)
    }

    pub fn font_size(&self) -> f32 {
        self.px
    }

    /// Re-rasterises at the new size instead of scaling the bitmaps, so glyphs
    /// stay crisp. The caller must recompute the grid afterwards: the cell
    /// changed size, so the terminal now holds a different number of them.
    pub fn set_font_size(&mut self, px: f32) -> bool {
        let px = px.clamp(MIN_FONT_PX, MAX_FONT_PX);
        if (px - self.px).abs() < 0.01 {
            return false;
        }
        let Some((cell_w, cell_h, baseline)) = metrics(&self.font, px) else {
            return false;
        };
        self.px = px;
        self.cell_w = cell_w;
        self.cell_h = cell_h;
        self.baseline = baseline;
        self.glyphs.clear();
        true
    }

    pub fn set_padding(&mut self, pad: usize) {
        self.pad = pad;
    }

    pub fn set_colors(&mut self, fg: u32, bg: u32) {
        self.fg = fg;
        self.bg = bg;
    }

    /// Which cell a pixel lands on, plus which half of it (for selection).
    pub fn cell_at(
        &self,
        x: f64,
        y: f64,
        width: usize,
        height: usize,
        size: TermSize,
    ) -> (usize, usize, bool) {
        let (ox, oy) = self.origin(width, height, size);
        let rel_x = (x - ox as f64).max(0.0);
        let rel_y = (y - oy as f64).max(0.0);
        let col = (rel_x as usize / self.cell_w).min(size.columns.saturating_sub(1));
        let row = (rel_y as usize / self.cell_h).min(size.screen_lines.saturating_sub(1));
        let right_half = (rel_x as usize % self.cell_w) * 2 >= self.cell_w;
        (row, col, right_half)
    }

    /// Where the grid starts, so leftover pixels are split evenly instead of
    /// piling up on one edge.
    fn origin(&self, width: usize, height: usize, size: TermSize) -> (usize, usize) {
        let x = width.saturating_sub(size.columns * self.cell_w) / 2;
        let y = height.saturating_sub(size.screen_lines * self.cell_h) / 2;
        (x, y)
    }

    /// False when the font has no glyph for `c` and would draw `.notdef` (tofu).
    /// Coverage is not a usable signal here: `.notdef` is itself an inked box.
    pub fn has_glyph(&self, c: char) -> bool {
        self.font.lookup_glyph_index(c) != 0
    }

    pub fn grid_size(&self, width_px: usize, height_px: usize) -> TermSize {
        let usable_w = width_px.saturating_sub(2 * self.pad);
        let usable_h = height_px.saturating_sub(2 * self.pad);
        TermSize::new(
            (usable_w / self.cell_w).max(1),
            (usable_h / self.cell_h).max(1),
        )
    }

    fn glyph(&mut self, c: char) -> &Glyph {
        self.glyphs.entry(c).or_insert_with(|| {
            let (m, coverage) = self.font.rasterize(c, self.px);
            Glyph {
                width: m.width,
                height: m.height,
                left: m.xmin,
                top: m.height as i32 + m.ymin,
                coverage,
            }
        })
    }

    fn resolve(&self, color: Color, default: u32) -> u32 {
        match color {
            Color::Spec(Rgb { r, g, b }) => rgb(r, g, b),
            Color::Indexed(i) => self.palette[i as usize],
            Color::Named(NamedColor::Foreground) => self.fg,
            Color::Named(NamedColor::Background) => self.bg,
            Color::Named(named) => {
                let i = named as usize;
                if i < 256 { self.palette[i] } else { default }
            }
        }
    }

    pub fn draw(
        &mut self,
        term: &Terminal,
        buf: &mut [u32],
        width: usize,
        height: usize,
        cursor_look: CursorLook,
    ) {
        buf.fill(self.bg);

        let offset = term.display_offset() as i32;
        let (cols, rows, cursor) = {
            let t = term.term();
            let p = t.grid().cursor.point;
            // The cursor lives in buffer coordinates; scrolling back can push it
            // off screen, and then it must not be drawn at all.
            let cursor_row = p.line.0 + offset;
            let visible = (0..t.screen_lines() as i32).contains(&cursor_row);
            (
                t.columns(),
                t.screen_lines(),
                visible.then_some((cursor_row as usize, p.column.0)),
            )
        };

        let (ox, oy) = self.origin(width, height, TermSize::new(cols, rows));

        for row in 0..rows {
            let y0 = oy + row * self.cell_h;
            if y0 >= height {
                break;
            }
            for col in 0..cols {
                let x0 = ox + col * self.cell_w;
                if x0 >= width {
                    break;
                }

                let (ch, mut fg, mut bg, wide, skip) = {
                    let cell =
                        &term.term().grid()[Point::new(Line(row as i32 - offset), Column(col))];
                    let inverse = cell.flags.contains(Flags::INVERSE);
                    let (f, b) = if inverse {
                        (
                            self.resolve(cell.bg, self.bg),
                            self.resolve(cell.fg, self.fg),
                        )
                    } else {
                        (
                            self.resolve(cell.fg, self.fg),
                            self.resolve(cell.bg, self.bg),
                        )
                    };
                    (
                        cell.c,
                        f,
                        b,
                        cell.flags.contains(Flags::WIDE_CHAR),
                        cell.flags
                            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                    )
                };

                let on_cursor = cursor == Some((row, col));
                if term.is_selected(row, col) || (on_cursor && cursor_look == CursorLook::Block) {
                    std::mem::swap(&mut fg, &mut bg);
                }

                // A wide glyph owns this cell and the next, and painted both when
                // its leading cell was reached. Touching the spacer here would
                // repaint over the half already drawn.
                if skip {
                    continue;
                }

                let span = if wide { 2 } else { 1 };
                fill_rect(
                    buf,
                    width,
                    height,
                    x0,
                    y0,
                    self.cell_w * span,
                    self.cell_h,
                    bg,
                );

                if ch != ' ' && ch != '\0' {
                    self.blit(buf, width, height, x0, y0, ch, fg, bg);
                }

                if on_cursor {
                    self.draw_cursor(buf, width, height, x0, y0, cursor_look, fg);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_cursor(
        &self,
        buf: &mut [u32],
        width: usize,
        height: usize,
        x0: usize,
        y0: usize,
        look: CursorLook,
        color: u32,
    ) {
        // Thin enough to sit beside the glyph rather than cover it, but never
        // less than one pixel or it disappears at small sizes.
        let thickness = (self.cell_w / 8).clamp(1, 2);
        match look {
            CursorLook::Hidden | CursorLook::Block => {}
            CursorLook::Beam => {
                fill_rect(buf, width, height, x0, y0, thickness, self.cell_h, color);
            }
            CursorLook::Underline => {
                let y = y0 + self.cell_h.saturating_sub(thickness);
                fill_rect(buf, width, height, x0, y, self.cell_w, thickness, color);
            }
            CursorLook::Hollow => {
                let (w, h) = (self.cell_w, self.cell_h);
                fill_rect(buf, width, height, x0, y0, w, 1, color);
                fill_rect(
                    buf,
                    width,
                    height,
                    x0,
                    y0 + h.saturating_sub(1),
                    w,
                    1,
                    color,
                );
                fill_rect(buf, width, height, x0, y0, 1, h, color);
                fill_rect(
                    buf,
                    width,
                    height,
                    x0 + w.saturating_sub(1),
                    y0,
                    1,
                    h,
                    color,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn blit(
        &mut self,
        buf: &mut [u32],
        width: usize,
        height: usize,
        x0: usize,
        y0: usize,
        ch: char,
        fg: u32,
        bg: u32,
    ) {
        let baseline = self.baseline;
        let g = self.glyph(ch);
        for gy in 0..g.height {
            let y = y0 as i32 + baseline - g.top + gy as i32;
            if y < 0 || y as usize >= height {
                continue;
            }
            for gx in 0..g.width {
                let x = x0 as i32 + g.left + gx as i32;
                if x < 0 || x as usize >= width {
                    continue;
                }
                let a = g.coverage[gy * g.width + gx];
                if a == 0 {
                    continue;
                }
                let idx = y as usize * width + x as usize;
                buf[idx] = blend(bg, fg, a);
            }
        }
    }
}

/// One zoom step. Multiplicative so a step feels the same at any size, rounded
/// to whole pixels so cell metrics stay crisp, and nudged so it always moves by
/// at least one pixel (rounding would otherwise swallow small steps).
pub fn step_font_size(current: f32, steps: i32) -> f32 {
    if steps == 0 {
        return current;
    }
    let mut target = (current * 1.15f32.powi(steps)).round();
    if (target - current).abs() < 1.0 {
        target = current + steps.signum() as f32;
    }
    target.clamp(MIN_FONT_PX, MAX_FONT_PX)
}

fn metrics(font: &Font, px: f32) -> Option<(usize, usize, i32)> {
    let lm = font.horizontal_line_metrics(px)?;
    let cell_h = (lm.ascent - lm.descent + lm.line_gap).ceil().max(1.0) as usize;
    let cell_w = font.metrics('M', px).advance_width.ceil().max(1.0) as usize;
    Some((cell_w, cell_h, lm.ascent.ceil() as i32))
}

fn blend(bg: u32, fg: u32, a: u8) -> u32 {
    let mix = |shift: u32| {
        let b = ((bg >> shift) & 0xff) as i32;
        let f = ((fg >> shift) & 0xff) as i32;
        (b + (f - b) * a as i32 / 255).clamp(0, 255) as u32
    };
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(
    buf: &mut [u32],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    for y in y0..(y0 + h).min(height) {
        let row = y * width;
        for x in x0..(x0 + w).min(width) {
            buf[row + x] = color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &str = r"C:\Windows\Fonts\CascadiaMono.ttf";

    fn renderer() -> Renderer {
        let bytes = std::fs::read(FONT).expect("CascadiaMono.ttf");
        Renderer::new(&bytes, 16.0).unwrap()
    }

    #[test]
    fn cells_are_not_degenerate() {
        let r = renderer();
        let (w, h) = r.cell_size();
        assert!(w > 3 && h > 5, "celda sospechosa: {w}x{h}");
    }

    #[test]
    fn grid_size_scales_with_pixels() {
        let r = renderer();
        let (cw, ch) = r.cell_size();
        let size = r.grid_size(cw * 80, ch * 24);
        assert_eq!(size, TermSize::new(80, 24));
    }

    #[test]
    fn drawing_text_marks_pixels() {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 10, ch * 2);
        let mut term = Terminal::new(TermSize::new(10, 2));
        term.feed(b"X");

        let mut buf = vec![0u32; w * h];
        r.draw(&term, &mut buf, w, h, CursorLook::Beam);

        let first_cell_lit = (0..ch)
            .flat_map(|y| (0..cw).map(move |x| (x, y)))
            .any(|(x, y)| buf[y * w + x] != DEFAULT_BG);
        assert!(first_cell_lit, "la celda 0 deberia tener pixeles de la X");
    }

    #[test]
    fn blank_screen_is_background() {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let term = Terminal::new(TermSize::new(4, 2));
        let mut buf = vec![0u32; w * h];
        r.draw(&term, &mut buf, w, h, CursorLook::Beam);

        // El cursor invierte su celda, asi que solo se mira fuera de ella.
        let dirty = (ch..h).any(|y| (0..w).any(|x| buf[y * w + x] != DEFAULT_BG));
        assert!(
            !dirty,
            "sin contenido no deberia pintarse nada fuera del cursor"
        );
    }

    #[test]
    fn font_covers_what_a_terminal_needs() {
        let r = renderer();
        for c in "Aa1@#áéñüÇ┌─┐│└┘├┼█▀▄".chars() {
            assert!(r.has_glyph(c), "falta el glifo {c:?}");
        }
    }

    #[test]
    fn known_gap_no_font_fallback() {
        // Cascadia Mono carries no CJK or emoji and there is no fallback font,
        // so these draw as tofu. Delete this test when fallback lands.
        let r = renderer();
        assert!(!r.has_glyph('中'));
        assert!(!r.has_glyph('😀'));
    }

    fn painted_columns(look: CursorLook) -> usize {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let term = Terminal::new(TermSize::new(4, 2));
        let mut buf = vec![0u32; w * h];
        r.draw(&term, &mut buf, w, h, look);
        // Columnas de la celda 0 con algun pixel distinto del fondo.
        (0..cw)
            .filter(|&x| (0..ch).any(|y| buf[y * w + x] != DEFAULT_BG))
            .count()
    }

    #[test]
    fn beam_cursor_is_a_thin_bar_not_a_block() {
        let (cw, _) = renderer().cell_size();
        let beam = painted_columns(CursorLook::Beam);
        assert!(beam >= 1, "la barra no se pinto");
        assert!(
            beam <= 2,
            "la barra ocupa {beam} columnas, deberia ser fina"
        );
        assert!(beam < cw, "la barra cubre toda la celda");
    }

    #[test]
    fn block_cursor_fills_the_cell() {
        let (cw, _) = renderer().cell_size();
        assert_eq!(painted_columns(CursorLook::Block), cw);
    }

    #[test]
    fn hidden_cursor_paints_nothing() {
        assert_eq!(painted_columns(CursorLook::Hidden), 0);
    }

    #[test]
    fn underline_cursor_spans_the_width_but_not_the_height() {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let term = Terminal::new(TermSize::new(4, 2));
        let mut buf = vec![0u32; w * h];
        r.draw(&term, &mut buf, w, h, CursorLook::Underline);
        let rows = (0..ch)
            .filter(|&y| (0..cw).any(|x| buf[y * w + x] != DEFAULT_BG))
            .count();
        assert!(rows <= 2, "el subrayado ocupa {rows} filas");
        assert_eq!(painted_columns(CursorLook::Underline), cw);
    }

    #[test]
    fn zooming_in_grows_the_cell_and_shrinks_the_grid() {
        let mut r = renderer();
        let (w0, h0) = r.cell_size();
        let cols0 = r.grid_size(800, 600).columns;

        assert!(r.set_font_size(28.0));
        let (w1, h1) = r.cell_size();
        assert!(w1 > w0 && h1 > h0, "{w0}x{h0} -> {w1}x{h1}");
        assert!(
            r.grid_size(800, 600).columns < cols0,
            "deberian entrar menos columnas"
        );
    }

    #[test]
    fn zooming_out_shrinks_the_cell() {
        let mut r = renderer();
        let (w0, _) = r.cell_size();
        assert!(r.set_font_size(9.0));
        assert!(r.cell_size().0 < w0);
    }

    #[test]
    fn font_size_is_clamped_and_reports_no_change() {
        let mut r = renderer();
        r.set_font_size(1000.0);
        assert_eq!(r.font_size(), MAX_FONT_PX);
        // Ya esta en el tope: pedir mas no cambia nada.
        assert!(!r.set_font_size(1000.0));

        r.set_font_size(0.1);
        assert_eq!(r.font_size(), MIN_FONT_PX);
    }

    #[test]
    fn glyphs_are_rerasterised_not_scaled() {
        let mut r = renderer();
        let small = r.glyph('M').coverage.len();
        r.set_font_size(32.0);
        let big = r.glyph('M').coverage.len();
        assert!(
            big > small,
            "el glifo deberia rasterizarse mas grande: {small} -> {big}"
        );
    }

    #[test]
    fn a_zoom_step_always_moves_at_least_one_pixel() {
        // Rounding a 15% step can land back on the same integer at small sizes,
        // which would make the key feel dead. Boundaries are excluded: there the
        // clamp is supposed to stop the move.
        for size in [7.0f32, 8.0, 10.0, 16.0, 24.0] {
            assert!(step_font_size(size, 1) > size, "no subio desde {size}");
            assert!(step_font_size(size, -1) < size, "no bajo desde {size}");
        }
    }

    #[test]
    fn zoom_steps_are_clamped_to_the_allowed_range() {
        assert_eq!(step_font_size(MAX_FONT_PX, 1), MAX_FONT_PX);
        assert_eq!(step_font_size(MIN_FONT_PX, -1), MIN_FONT_PX);
        assert_eq!(step_font_size(1000.0, 1), MAX_FONT_PX);
    }

    #[test]
    fn zero_steps_changes_nothing() {
        assert_eq!(step_font_size(16.0, 0), 16.0);
    }

    #[test]
    fn zoom_in_then_out_lands_near_the_start() {
        let start = 16.0;
        let out = step_font_size(step_font_size(start, 1), -1);
        assert!((out - start).abs() <= 1.0, "{start} -> {out}");
    }

    #[test]
    fn a_wide_cell_keeps_both_halves() {
        // The spacer cell used to repaint its own background after the leading
        // cell had already drawn across both, cutting the glyph in half.
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 6, ch * 2);
        let mut term = Terminal::new(TermSize::new(6, 2));
        term.feed("中".as_bytes());
        // Park the cursor on the wide char so its inverted block spans two cells.
        term.feed(b"[1;1H");

        let mut buf = vec![0u32; w * h];
        r.draw(&term, &mut buf, w, h, CursorLook::Block);

        let lit =
            |col: usize| (0..ch).any(|y| (0..cw).any(|x| buf[y * w + col * cw + x] != DEFAULT_BG));
        assert!(lit(0), "la celda guia deberia estar pintada");
        assert!(
            lit(1),
            "la mitad derecha se borro: el espaciador la repinto"
        );
    }

    #[test]
    fn palette_matches_foot_defaults() {
        // upstream/foot/config.c:33-86
        let r = renderer();
        assert_eq!(DEFAULT_FG, 0x00ff_ffff);
        assert_eq!(DEFAULT_BG, 0x0024_2424);
        assert_eq!(r.palette[0], 0x0024_2424, "negro de foot es el fondo");
        assert_eq!(r.palette[1], 0x00f6_2b5a, "rojo de foot");
        assert_eq!(r.palette[15], 0x00ff_ffff);
        // cubo 6x6x6 y rampa de grises, iguales a xterm y a foot
        assert_eq!(r.palette[16], 0x0000_0000);
        assert_eq!(r.palette[231], 0x00ff_ffff);
        assert_eq!(r.palette[232], 0x0008_0808);
        assert_eq!(r.palette[255], 0x00ee_eeee);
    }
}
