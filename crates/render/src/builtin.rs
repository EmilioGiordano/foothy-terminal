//! Glyphs the renderer draws itself when the font has none.
//!
//! The prompt chip is built from Nerd Font's private use area, which a stock
//! Cascadia install does not carry. Synthesising the four shapes it needs keeps
//! the chip looking right without asking anyone to install a font, and they are
//! regenerated per size, so zooming stays crisp.

/// Every codepoint this module can draw.
pub const BUILTIN: [char; 4] = [PILL_LEFT, PILL_RIGHT, FOLDER, BRANCH];

pub(crate) const PILL_LEFT: char = '\u{e0b6}';
pub(crate) const PILL_RIGHT: char = '\u{e0b4}';
pub(crate) const FOLDER: char = '\u{f07b}';
pub(crate) const BRANCH: char = '\u{e0a0}';

pub(crate) fn has(c: char) -> bool {
    BUILTIN.contains(&c)
}

/// Samples per axis inside one pixel. Enough to keep a small curve from looking
/// like a staircase without making the cache noticeably slower to fill.
const SUB: usize = 4;

/// A coverage bitmap the size of one cell, or `None` for a codepoint we do not
/// synthesise. The caller places it at the cell origin, not the baseline.
pub(crate) fn coverage(c: char, w: usize, h: usize) -> Option<Vec<u8>> {
    if w == 0 || h == 0 {
        return None;
    }
    let (fw, fh) = (w as f32, h as f32);
    let inside: Box<dyn Fn(f32, f32) -> bool> = match c {
        PILL_LEFT => Box::new(move |x, y| in_cap(x, y, fw, fh, true)),
        PILL_RIGHT => Box::new(move |x, y| in_cap(x, y, fw, fh, false)),
        FOLDER => Box::new(move |x, y| in_folder(x, y, fw, fh)),
        BRANCH => Box::new(move |x, y| in_branch(x, y, fw, fh)),
        _ => return None,
    };

    let mut out = vec![0u8; w * h];
    let step = 1.0 / SUB as f32;
    for py in 0..h {
        for px in 0..w {
            let mut hits = 0;
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let x = px as f32 + (sx as f32 + 0.5) * step;
                    let y = py as f32 + (sy as f32 + 0.5) * step;
                    if inside(x, y) {
                        hits += 1;
                    }
                }
            }
            out[py * w + px] = ((hits * 255) / (SUB * SUB)) as u8;
        }
    }
    Some(out)
}

/// A cap fills its cell except for the two corners on its outer side, which are
/// rounded off. The inner edge stays square so it meets the cell background of
/// the pill body exactly; any gap there would show as a hairline.
fn in_cap(x: f32, y: f32, w: f32, h: f32, left: bool) -> bool {
    // Mirror the right cap so only one shape has to be reasoned about.
    let x = if left { x } else { w - x };
    let r = (h / 3.0).min(w);
    if x >= r {
        return true;
    }
    let upper = y < h / 2.0;
    let cy = if upper { r } else { h - r };
    let in_corner = if upper { y < cy } else { y > cy };
    if !in_corner {
        return true;
    }
    let (dx, dy) = (x - r, y - cy);
    dx * dx + dy * dy <= r * r
}

fn in_folder(x: f32, y: f32, w: f32, h: f32) -> bool {
    // A box roughly the height of a lowercase letter, sitting on the baseline
    // area rather than filling the cell.
    let pad_x = w * 0.08;
    let (x0, x1) = (pad_x, w - pad_x);
    let (y0, y1) = (h * 0.30, h * 0.72);
    let tab_h = (y1 - y0) * 0.24;

    // The raised tab on the left.
    if x >= x0 && x <= x0 + (x1 - x0) * 0.45 && y >= y0 && y < y0 + tab_h {
        return true;
    }
    x >= x0 && x <= x1 && y >= y0 + tab_h && y <= y1
}

fn in_branch(x: f32, y: f32, w: f32, h: f32) -> bool {
    // Powerline's branch: a trunk with two nodes, and a limb to a third.
    let (top, bottom) = (h * 0.30, h * 0.72);
    let trunk_x = w * 0.30;
    let limb_x = w * 0.74;
    let r = (w * 0.16).max(1.0);
    let t = (w * 0.10).max(0.7);

    if disc(x, y, trunk_x, top, r) || disc(x, y, trunk_x, bottom, r) {
        return true;
    }
    if disc(x, y, limb_x, top, r) {
        return true;
    }
    if segment(x, y, trunk_x, top, trunk_x, bottom, t) {
        return true;
    }
    // The limb leaves the trunk low and rises to the third node.
    segment(
        x,
        y,
        trunk_x,
        bottom - (bottom - top) * 0.25,
        limb_x,
        top,
        t,
    )
}

fn disc(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> bool {
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

/// Distance from a point to a segment, against half the stroke width.
fn segment(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32) -> bool {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((x - x0) * dx + (y - y0) * dy) / len2).clamp(0.0, 1.0)
    };
    let (px, py) = (x0 + dx * t, y0 + dy * t);
    let (ex, ey) = (x - px, y - py);
    ex * ex + ey * ey <= (thickness / 2.0) * (thickness / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 10;
    const H: usize = 19;

    fn at(cov: &[u8], x: usize, y: usize) -> u8 {
        cov[y * W + x]
    }

    #[test]
    fn a_sprite_is_exactly_one_cell() {
        for c in BUILTIN {
            let cov = coverage(c, W, H).expect("builtin");
            assert_eq!(cov.len(), W * H, "{c:?} is not cell sized");
        }
    }

    #[test]
    fn only_the_four_shapes_are_synthesised() {
        assert!(has(PILL_LEFT) && has(BRANCH));
        assert!(!has('a'));
        assert_eq!(coverage('a', W, H), None);
    }

    #[test]
    fn the_left_cap_meets_the_pill_fill_with_no_gap() {
        // The inner edge has to be solid top to bottom: the fill beside it comes
        // from the cell background, and any hole there shows as a hairline.
        let cov = coverage(PILL_LEFT, W, H).unwrap();
        for y in 0..H {
            assert_eq!(at(&cov, W - 1, y), 255, "hole in the inner edge at y={y}");
        }
    }

    #[test]
    fn the_right_cap_meets_the_pill_fill_with_no_gap() {
        let cov = coverage(PILL_RIGHT, W, H).unwrap();
        for y in 0..H {
            assert_eq!(at(&cov, 0, y), 255, "hole in the inner edge at y={y}");
        }
    }

    #[test]
    fn the_caps_are_rounded_on_their_outer_corners() {
        let left = coverage(PILL_LEFT, W, H).unwrap();
        assert_eq!(at(&left, 0, 0), 0, "top left should be cut away");
        assert_eq!(at(&left, 0, H - 1), 0, "bottom left should be cut away");
        assert_eq!(at(&left, 0, H / 2), 255, "the middle should stay solid");

        let right = coverage(PILL_RIGHT, W, H).unwrap();
        assert_eq!(at(&right, W - 1, 0), 0);
        assert_eq!(at(&right, W - 1, H - 1), 0);
        assert_eq!(at(&right, W - 1, H / 2), 255);
    }

    #[test]
    fn the_caps_are_mirror_images() {
        let left = coverage(PILL_LEFT, W, H).unwrap();
        let right = coverage(PILL_RIGHT, W, H).unwrap();
        for y in 0..H {
            for x in 0..W {
                assert_eq!(at(&left, x, y), at(&right, W - 1 - x, y), "at ({x},{y})");
            }
        }
    }

    #[test]
    fn the_rounding_is_antialiased_rather_than_a_staircase() {
        let left = coverage(PILL_LEFT, W, H).unwrap();
        let partial = left.iter().filter(|a| **a > 0 && **a < 255).count();
        assert!(
            partial >= 4,
            "only {partial} soft pixels, the curve is hard"
        );
    }

    #[test]
    fn the_icons_stay_clear_of_the_cell_edges() {
        // They sit next to text, so ink touching the edge would run into the
        // neighbouring character.
        for c in [FOLDER, BRANCH] {
            let cov = coverage(c, W, H).unwrap();
            for x in 0..W {
                assert_eq!(at(&cov, x, 0), 0, "{c:?} inks the top row");
                assert_eq!(at(&cov, x, H - 1), 0, "{c:?} inks the bottom row");
            }
        }
    }

    #[test]
    fn the_icons_actually_draw_something() {
        for c in [FOLDER, BRANCH] {
            let cov = coverage(c, W, H).unwrap();
            let inked = cov.iter().filter(|a| **a > 128).count();
            assert!(inked > 8, "{c:?} drew almost nothing ({inked} px)");
            assert!(inked < W * H / 2, "{c:?} is a blob ({inked} px)");
        }
    }

    #[test]
    fn a_tiny_cell_still_produces_something_drawable() {
        // Zooming all the way out must not panic or divide by zero.
        for c in BUILTIN {
            let cov = coverage(c, 3, 5).expect("builtin");
            assert_eq!(cov.len(), 15);
        }
        assert_eq!(coverage(PILL_LEFT, 0, 5), None);
    }
}
