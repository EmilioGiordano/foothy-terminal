use std::collections::HashMap;
use std::collections::hash_map::Entry;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use fontdue::{Font, FontSettings};
use vt_core::{TermSize, Terminal};

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
const FALLBACK_FONT_PX: f32 = 16.0;

/// Every size entering fontdue goes through here: a negative or huge `scale`
/// aborts the process, and NaN survives the clamp to die at rasterisation.
fn sane_px(px: f32) -> f32 {
    if px.is_nan() {
        return FALLBACK_FONT_PX;
    }
    px.clamp(MIN_FONT_PX, MAX_FONT_PX)
}

pub const DEFAULT_FG: u32 = 0x00ff_ffff;
pub const DEFAULT_BG: u32 = 0x0024_2424;

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

mod builtin;

/// Codepoints the renderer draws itself when the font lacks them. Callers
/// choosing glyphs can rely on these being drawable whatever font is loaded.
pub use builtin::BUILTIN as SYNTHESISED_GLYPHS;

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
    aliases: Vec<(u32, u32)>,
}

impl Renderer {
    pub fn new(font_bytes: &[u8], px: f32, palette: &[u32; 256]) -> anyhow::Result<Self> {
        let px = sane_px(px);
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
            palette: *palette,
            glyphs: HashMap::new(),
            pad: 0,
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            aliases: Vec::new(),
        })
    }

    pub fn cell_size(&self) -> (usize, usize) {
        (self.cell_w, self.cell_h)
    }

    pub fn font_size(&self) -> f32 {
        self.px
    }

    /// The caller must recompute the grid afterwards: the cell changed size, so
    /// the terminal holds a different number of them.
    pub fn set_font_size(&mut self, px: f32) -> bool {
        let px = sane_px(px);
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

    pub fn set_palette(&mut self, palette: &[u32; 256]) {
        self.palette = *palette;
    }

    pub fn set_color_aliases(&mut self, aliases: &[(u32, u32)]) {
        self.aliases.clear();
        self.aliases.extend_from_slice(aliases);
    }

    pub fn header_height(&self) -> usize {
        self.cell_h
    }

    /// Returns the x the pen stopped at.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        buf: &mut [u32],
        width: usize,
        height: usize,
        x: usize,
        y: usize,
        max_w: usize,
        text: &str,
        fg: u32,
        bg: u32,
    ) -> usize {
        let mut pen = x;
        for ch in text.chars() {
            if pen + self.cell_w > x + max_w || pen + self.cell_w > width {
                break;
            }
            if ch != ' ' {
                self.blit(buf, width, height, pen, y, ch, fg, bg);
            }
            pen += self.cell_w;
        }
        pen
    }

    pub fn tint_area(
        &self,
        buf: &mut [u32],
        width: usize,
        height: usize,
        area: (usize, usize, usize, usize),
        color: u32,
        alpha: u8,
    ) {
        let (x, y, w, h) = area;
        for row in y..(y + h).min(height) {
            let base = row * width;
            for col in x..(x + w).min(width) {
                buf[base + col] = blend(buf[base + col], color, alpha);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn outline_area(
        &self,
        buf: &mut [u32],
        width: usize,
        height: usize,
        area: (usize, usize, usize, usize),
        color: u32,
        alpha: u8,
    ) {
        let (x, y, w, h) = area;
        if w == 0 || h == 0 {
            return;
        }
        self.tint_area(buf, width, height, (x, y, w, 1), color, alpha);
        self.tint_area(buf, width, height, (x, y + h - 1, w, 1), color, alpha);
        // Blending is not idempotent: the sides stop short of the corners the
        // rows already covered.
        if h > 2 {
            self.tint_area(buf, width, height, (x, y + 1, 1, h - 2), color, alpha);
            self.tint_area(
                buf,
                width,
                height,
                (x + w - 1, y + 1, 1, h - 2),
                color,
                alpha,
            );
        }
    }

    pub fn fill_area(
        &self,
        buf: &mut [u32],
        width: usize,
        height: usize,
        area: (usize, usize, usize, usize),
        color: u32,
    ) {
        let (x, y, w, h) = area;
        fill_rect(buf, width, height, x, y, w, h, color);
    }

    /// Returns `(row, col, right_half)`.
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

    fn origin(&self, width: usize, height: usize, size: TermSize) -> (usize, usize) {
        let x = width.saturating_sub(size.columns * self.cell_w) / 2;
        let y = height.saturating_sub(size.screen_lines * self.cell_h) / 2;
        (x, y)
    }

    /// Coverage cannot answer this: `.notdef` is itself an inked box.
    pub fn has_glyph(&self, c: char) -> bool {
        self.font.lookup_glyph_index(c) != 0 || builtin::has(c)
    }

    pub fn grid_size(&self, width_px: usize, height_px: usize) -> TermSize {
        let usable_w = width_px.saturating_sub(2 * self.pad);
        let usable_h = height_px.saturating_sub(2 * self.pad);
        TermSize::new(
            (usable_w / self.cell_w).max(vt_core::MIN_COLUMNS),
            (usable_h / self.cell_h).max(vt_core::MIN_SCREEN_LINES),
        )
    }

    fn glyph(&mut self, c: char) -> &Glyph {
        let (cell_w, cell_h, baseline) = (self.cell_w, self.cell_h, self.baseline);
        let (font, px) = (&self.font, self.px);
        // Everything costly stays in the vacant arm: this runs once per cell per
        // frame, and a cache hit must not pay for a font lookup.
        match self.glyphs.entry(c) {
            Entry::Occupied(hit) => hit.into_mut(),
            Entry::Vacant(slot) => {
                let glyph = if font.lookup_glyph_index(c) == 0
                    && let Some(coverage) = builtin::coverage(c, cell_w, cell_h)
                {
                    // The blit offsets by `baseline - top`, so this lands the
                    // sprite on the cell origin and lets it fill the cell.
                    Glyph {
                        width: cell_w,
                        height: cell_h,
                        left: 0,
                        top: baseline,
                        coverage,
                    }
                } else {
                    let (m, coverage) = font.rasterize(c, px);
                    Glyph {
                        width: m.width,
                        height: m.height,
                        left: m.xmin,
                        top: m.height as i32 + m.ymin,
                        coverage,
                    }
                };
                slot.insert(glyph)
            }
        }
    }

    fn resolve(&self, color: Color, default: u32) -> u32 {
        match color {
            Color::Spec(Rgb { r, g, b }) => {
                let c = rgb(r, g, b);
                self.aliases
                    .iter()
                    .find(|(from, _)| *from == c)
                    .map_or(c, |(_, to)| *to)
            }
            Color::Indexed(i) => self.palette[i as usize],
            Color::Named(NamedColor::Foreground) => self.fg,
            Color::Named(NamedColor::Background) => self.bg,
            Color::Named(named) => {
                let i = named as usize;
                if i < 256 { self.palette[i] } else { default }
            }
        }
    }

    pub fn clear(&self, buf: &mut [u32]) {
        buf.fill(self.bg);
    }

    pub fn draw(
        &mut self,
        term: &Terminal,
        buf: &mut [u32],
        width: usize,
        height: usize,
        cursor_look: CursorLook,
    ) {
        self.clear(buf);
        self.draw_into(term, buf, width, height, (0, 0, width, height), cursor_look);
    }

    /// The caller clears first: with several panes the buffer is shared and each
    /// one owns only its own rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_into(
        &mut self,
        term: &Terminal,
        buf: &mut [u32],
        width: usize,
        height: usize,
        area: (usize, usize, usize, usize),
        cursor_look: CursorLook,
    ) {
        let (ax, ay, aw, ah) = area;
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

        // Hoisted: resolving it per cell re-runs a word/line search each time.
        let selection = term.selection_range();
        let (rx, ry) = self.origin(aw, ah, TermSize::new(cols, rows));
        let (ox, oy) = (ax + rx, ay + ry);
        let (right, bottom) = ((ax + aw).min(width), (ay + ah).min(height));

        for row in 0..rows {
            let y0 = oy + row * self.cell_h;
            if y0 + self.cell_h > bottom {
                break;
            }
            for col in 0..cols {
                let x0 = ox + col * self.cell_w;
                if x0 + self.cell_w > right {
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
                let selected = selection
                    .as_ref()
                    .is_some_and(|r| term.is_in_range(r, row, col));
                if selected || (on_cursor && cursor_look == CursorLook::Block) {
                    std::mem::swap(&mut fg, &mut bg);
                }

                // A wide glyph painted this cell and the next from its leading
                // cell; repainting the spacer would cut the glyph in half.
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

/// Nudged by a pixel because rounding a 15% step can land on the same integer,
/// which makes the key feel dead at small sizes.
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

    const FONT_CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\CascadiaMono.ttf",
        r"C:\Windows\Fonts\consola.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/System/Library/Fonts/Menlo.ttc",
    ];

    fn font_bytes() -> Vec<u8> {
        for path in FONT_CANDIDATES {
            if let Ok(bytes) = std::fs::read(path) {
                return bytes;
            }
        }
        panic!("no monospace font found; tried {FONT_CANDIDATES:?}");
    }

    /// Non-zero everywhere, so a palette that was never installed cannot pass
    /// for an installed one.
    fn test_palette() -> [u32; 256] {
        std::array::from_fn(|i| 0x0001_0000 | i as u32)
    }

    fn renderer() -> Renderer {
        Renderer::new(&font_bytes(), 16.0, &test_palette()).unwrap()
    }

    /// Has to rasterise, not just construct: NaN only dies at that point.
    #[test]
    fn a_hostile_font_size_still_renders() {
        let bytes = font_bytes();
        for px in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -16.0,
            1e30,
            0.0,
            1000.0,
        ] {
            let mut r = Renderer::new(&bytes, px, &test_palette()).unwrap();
            let got = r.font_size();
            assert!(
                got.is_finite() && (MIN_FONT_PX..=MAX_FONT_PX).contains(&got),
                "{px} gave {got}"
            );

            let (cw, ch) = r.cell_size();
            let (w, h) = (cw * 4, ch * 2);
            let mut term = Terminal::new(TermSize::new(4, 2));
            term.feed(b"X");
            let mut buf = vec![0u32; w * h];
            r.draw(&term, &mut buf, w, h, CursorLook::Beam);
            assert!(buf.iter().any(|p| *p != DEFAULT_BG), "{px} drew nothing");

            let mut r = renderer();
            r.set_font_size(px);
            let after = r.font_size();
            assert!(
                after.is_finite() && (MIN_FONT_PX..=MAX_FONT_PX).contains(&after),
                "set_font_size({px}) gave {after}"
            );
            let (cw, ch) = r.cell_size();
            let mut buf = vec![0u32; cw * 4 * ch * 2];
            r.draw(&term, &mut buf, cw * 4, ch * 2, CursorLook::Beam);
        }
    }

    #[test]
    fn cells_are_not_degenerate() {
        let r = renderer();
        let (w, h) = r.cell_size();
        assert!(w > 3 && h > 5, "suspicious cell: {w}x{h}");
    }

    #[test]
    fn grid_size_scales_with_pixels() {
        let r = renderer();
        let (cw, ch) = r.cell_size();
        let size = r.grid_size(cw * 80, ch * 24);
        assert_eq!(size, TermSize::new(80, 24));
    }

    #[test]
    fn grid_size_never_goes_below_the_floor() {
        for px in [8.0, 16.0, 400.0] {
            let mut r = Renderer::new(&font_bytes(), px, &test_palette()).unwrap();
            for pad in [0, 50] {
                r.set_padding(pad);
                for (w, h) in [(0, 0), (1, 1), (3, 3), (40, 40)] {
                    let size = r.grid_size(w, h);
                    assert!(
                        size.columns >= vt_core::MIN_COLUMNS,
                        "px {px} pad {pad} at {w}x{h}: {size:?}"
                    );
                    assert!(
                        size.screen_lines >= vt_core::MIN_SCREEN_LINES,
                        "px {px} pad {pad} at {w}x{h}: {size:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_grid_wider_than_its_area_stays_inside_the_buffer() {
        let mut r = renderer();
        let mut term = Terminal::new(TermSize::new(2, 1));
        term.feed("中".as_bytes());
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let mut buf = vec![0u32; w * h];
        r.draw_into(&term, &mut buf, w, h, (0, 0, 1, 1), CursorLook::Beam);
        let outside: Vec<usize> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| (x > 0 || y > 0) && buf[y * w + x] != 0)
            .map(|(x, _)| x)
            .collect();
        assert!(
            outside.is_empty(),
            "painted {} pixels outside a 1x1 area, up to x={:?}",
            outside.len(),
            outside.iter().max()
        );
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
        assert!(first_cell_lit, "cell 0 should hold pixels of the X");
    }

    #[test]
    fn blank_screen_is_background() {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let term = Terminal::new(TermSize::new(4, 2));
        let mut buf = vec![0u32; w * h];
        r.draw(&term, &mut buf, w, h, CursorLook::Beam);

        // The cursor inverts its own cell, so only look outside it.
        let dirty = (ch..h).any(|y| (0..w).any(|x| buf[y * w + x] != DEFAULT_BG));
        assert!(!dirty, "nothing but the cursor should be painted");
    }

    #[test]
    fn font_covers_what_a_terminal_needs() {
        let r = renderer();
        for c in "Aa1@#áéñüÇ┌─┐│└┘├┼█▀▄".chars() {
            assert!(r.has_glyph(c), "missing glyph {c:?}");
        }
    }

    #[test]
    fn known_gap_no_font_fallback() {
        // No fallback font yet, so these draw as tofu. Delete this test when
        // fallback lands.
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
        (0..cw)
            .filter(|&x| (0..ch).any(|y| buf[y * w + x] != DEFAULT_BG))
            .count()
    }

    #[test]
    fn beam_cursor_is_a_thin_bar_not_a_block() {
        let (cw, _) = renderer().cell_size();
        let beam = painted_columns(CursorLook::Beam);
        assert!(beam >= 1, "the bar was not painted");
        assert!(beam <= 2, "the bar spans {beam} columns, it should be thin");
        assert!(beam < cw, "the bar covers the whole cell");
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
        assert!(rows <= 2, "the underline spans {rows} rows");
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
            "fewer columns should fit"
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
            "the glyph should rasterise larger: {small} -> {big}"
        );
    }

    #[test]
    fn a_zoom_step_always_moves_at_least_one_pixel() {
        // Boundaries are excluded: there the clamp is supposed to stop the move.
        for size in [7.0f32, 8.0, 10.0, 16.0, 24.0] {
            assert!(step_font_size(size, 1) > size, "did not rise from {size}");
            assert!(step_font_size(size, -1) < size, "did not fall from {size}");
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
        assert!(lit(0), "the leading cell should be painted");
        assert!(lit(1), "the right half was erased by the spacer");
    }

    #[test]
    fn the_outline_blends_every_border_pixel_exactly_once() {
        let r = renderer();
        let (width, height) = (40usize, 30usize);
        let (x, y, w, h) = (5usize, 4usize, 20usize, 15usize);
        let mut buf = vec![0u32; width * height];
        r.clear(&mut buf);
        r.outline_area(&mut buf, width, height, (x, y, w, h), 0x00ff_ffff, 110);

        let corner = buf[y * width + x];
        assert_eq!(buf[y * width + x + w / 2], corner, "top edge");
        assert_eq!(buf[(y + h - 1) * width + x + w / 2], corner, "bottom edge");
        assert_eq!(buf[(y + h / 2) * width + x], corner, "left edge");
        assert_eq!(buf[(y + h / 2) * width + x + w - 1], corner, "right edge");
        assert_eq!(buf[y * width + x + w - 1], corner, "top right corner");
        assert_eq!(buf[(y + h - 1) * width + x], corner, "bottom left corner");
    }

    #[test]
    fn the_outline_survives_a_rectangle_too_thin_to_have_an_inside() {
        let r = renderer();
        let mut buf = vec![0u32; 40 * 30];
        r.clear(&mut buf);
        r.outline_area(&mut buf, 40, 30, (5, 4, 20, 1), 0x00ff_ffff, 110);
        r.outline_area(&mut buf, 40, 30, (5, 4, 1, 15), 0x00ff_ffff, 110);
        r.outline_area(&mut buf, 40, 30, (5, 4, 2, 2), 0x00ff_ffff, 110);
    }

    #[test]
    fn draw_into_never_paints_outside_its_area() {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 20, ch * 10);
        let area = (cw * 4, ch * 2, cw * 8, ch * 4);

        let mut term = Terminal::new(TermSize::new(40, 20));
        for i in 0..20 {
            term.feed(format!("line {i} full of text to overflow with\r\n").as_bytes());
        }

        let mut buf = vec![0u32; w * h];
        r.clear(&mut buf);
        r.draw_into(&term, &mut buf, w, h, area, CursorLook::Block);

        let (ax, ay, aw, ah) = area;
        for y in 0..h {
            for x in 0..w {
                let inside = x >= ax && x < ax + aw && y >= ay && y < ay + ah;
                if !inside {
                    assert_eq!(
                        buf[y * w + x],
                        DEFAULT_BG,
                        "pixel ({x},{y}) painted outside the area {area:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn draw_into_actually_paints_inside() {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 20, ch * 10);
        let area = (cw * 4, ch * 2, cw * 8, ch * 4);

        let mut term = Terminal::new(TermSize::new(8, 4));
        term.feed(b"XXXX");

        let mut buf = vec![0u32; w * h];
        r.clear(&mut buf);
        r.draw_into(&term, &mut buf, w, h, area, CursorLook::Beam);

        let (ax, ay, aw, ah) = area;
        let painted = (ay..ay + ah).any(|y| (ax..ax + aw).any(|x| buf[y * w + x] != DEFAULT_BG));
        assert!(painted, "nothing was drawn inside the area");
    }

    #[test]
    fn draw_text_stops_at_max_width() {
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 20, ch * 3);
        let mut buf = vec![0u32; w * h];
        r.clear(&mut buf);

        let limit = cw * 3;
        r.draw_text(
            &mut buf,
            w,
            h,
            0,
            0,
            limit,
            "XXXXXXXXXX",
            DEFAULT_FG,
            DEFAULT_BG,
        );

        let beyond = (0..ch).any(|y| (limit..w).any(|x| buf[y * w + x] != DEFAULT_BG));
        assert!(!beyond, "draw_text wrote past max_w");
    }

    #[test]
    fn reverse_video_fills_the_cell_with_the_foreground_and_follows_a_theme() {
        // A swapped pair is a slab only if it resolves the default pair, and
        // it has to do so every frame or it stops following the theme.
        let mut r = renderer();
        r.set_colors(0x00dc_ecf5, 0x0016_2a33);
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let mut term = Terminal::new(TermSize::new(4, 2));
        term.feed(b"\x1b[7m  ");

        let slab = |r: &mut Renderer| {
            let mut buf = vec![0u32; w * h];
            r.clear(&mut buf);
            r.draw(&term, &mut buf, w, h, CursorLook::Hidden);
            buf[(ch / 2) * w + cw + cw / 2]
        };
        assert_eq!(slab(&mut r), 0x00dc_ecf5);
        r.set_colors(0x0000_0000, 0x00ff_ffff);
        assert_eq!(slab(&mut r), 0x0000_0000);
    }

    #[test]
    fn an_alias_substitutes_a_truecolor_value_and_leaves_the_rest_alone() {
        let mut r = renderer();
        r.set_color_aliases(&[(0x0001_02f0, 0x0012_3456)]);
        assert_eq!(
            r.resolve(Color::Spec(Rgb { r: 1, g: 2, b: 240 }), DEFAULT_FG),
            0x0012_3456
        );
        assert_eq!(
            r.resolve(Color::Spec(Rgb { r: 1, g: 2, b: 241 }), DEFAULT_FG),
            0x0001_02f1
        );
        assert_eq!(r.resolve(Color::Indexed(6), DEFAULT_FG), test_palette()[6]);
        assert_eq!(
            r.resolve(Color::Named(NamedColor::Cyan), DEFAULT_FG),
            test_palette()[6]
        );
    }

    #[test]
    fn the_chevrons_sgr_36_lands_on_the_palette_not_on_the_foreground() {
        // `ESC[36m` parses to a named colour, never to `Indexed(6)`, and the
        // two only agree while `Named(n)` resolves through the palette.
        let mut r = renderer();
        r.set_colors(0x00dc_ecf5, 0x0016_2a33);
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let mut term = Terminal::new(TermSize::new(4, 2));
        // Reversed, so the cyan ink becomes a slab no glyph covers.
        term.feed(b"\x1b[36;7m  ");
        let mut buf = vec![0u32; w * h];
        r.clear(&mut buf);
        r.draw(&term, &mut buf, w, h, CursorLook::Hidden);
        assert_eq!(buf[(ch / 2) * w + cw + cw / 2], test_palette()[6]);
    }

    #[test]
    fn a_new_alias_table_repaints_a_cell_already_on_screen() {
        // The bug this guards: colours baked into bytes already drawn cannot
        // follow a theme, so the substitution has to happen per frame.
        let mut r = renderer();
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 4, ch * 2);
        let mut term = Terminal::new(TermSize::new(4, 2));
        term.feed(b"\x1b[48;2;1;2;240m  ");

        let slab = |r: &mut Renderer| {
            let mut buf = vec![0u32; w * h];
            r.clear(&mut buf);
            r.draw(&term, &mut buf, w, h, CursorLook::Hidden);
            buf[(ch / 2) * w + cw + cw / 2]
        };
        r.set_color_aliases(&[(0x0001_02f0, 0x0012_3456)]);
        assert_eq!(slab(&mut r), 0x0012_3456);
        r.set_color_aliases(&[(0x0001_02f0, 0x00ab_cdef)]);
        assert_eq!(slab(&mut r), 0x00ab_cdef);
        r.set_color_aliases(&[]);
        assert_eq!(slab(&mut r), 0x0001_02f0);
    }

    #[test]
    fn a_fresh_renderer_resolves_the_palette_it_was_built_with() {
        let p = test_palette();
        let r = renderer();
        for i in 0..256u16 {
            assert_eq!(
                r.resolve(Color::Indexed(i as u8), DEFAULT_FG),
                p[i as usize]
            );
        }
    }

    #[test]
    fn an_installed_palette_resolves_indexed_colors() {
        let mut r = renderer();
        let mut p = [0u32; 256];
        p[5] = 0x0012_3456;
        p[200] = 0x00ab_cdef;
        r.set_palette(&p);
        assert_eq!(r.resolve(Color::Indexed(5), DEFAULT_FG), 0x0012_3456);
        assert_eq!(r.resolve(Color::Indexed(200), DEFAULT_FG), 0x00ab_cdef);
        r.set_colors(0x0011_1111, 0x0022_2222);
        assert_eq!(
            r.resolve(Color::Named(NamedColor::Foreground), DEFAULT_FG),
            0x0011_1111
        );
    }
}
