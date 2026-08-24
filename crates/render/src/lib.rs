use std::collections::HashMap;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use fontdue::{Font, FontSettings};
use vt_core::{TermSize, Terminal};

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

        let lm = font
            .horizontal_line_metrics(px)
            .ok_or_else(|| anyhow::anyhow!("font has no horizontal line metrics"))?;
        let cell_h = (lm.ascent - lm.descent + lm.line_gap).ceil().max(1.0) as usize;
        let cell_w = font.metrics('M', px).advance_width.ceil().max(1.0) as usize;

        Ok(Self {
            font,
            px,
            cell_w,
            cell_h,
            baseline: lm.ascent.ceil() as i32,
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

    pub fn set_padding(&mut self, pad: usize) {
        self.pad = pad;
    }

    pub fn set_colors(&mut self, fg: u32, bg: u32) {
        self.fg = fg;
        self.bg = bg;
    }

    pub fn background(&self) -> u32 {
        self.bg
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

    pub fn draw(&mut self, term: &Terminal, buf: &mut [u32], width: usize, height: usize) {
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

                if cursor == Some((row, col)) {
                    std::mem::swap(&mut fg, &mut bg);
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

                if skip || ch == ' ' || ch == '\0' {
                    continue;
                }
                self.blit(buf, width, height, x0, y0, ch, fg, bg);
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
        r.draw(&term, &mut buf, w, h);

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
        r.draw(&term, &mut buf, w, h);

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
