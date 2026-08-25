use std::collections::HashMap;
use std::sync::Arc;

use layout::{Direction, Divider, DividerId, DropZone, PaneId, Rect, Tree};
use portable_pty::CommandBuilder;
use vt_core::pty::Pty;
use vt_core::{CursorShape, TermSize, Terminal};

/// Hit target for a divider. Wider than what is drawn, because grabbing a
/// one-pixel line with a mouse is miserable.
pub const DIVIDER_HIT: f32 = 8.0;
pub const DIVIDER_DRAW: usize = 1;

/// Breathing room on the sides of every pane so text never touches the divider.
/// Sides only: vertical space is scarce and the header already separates panes.
pub const PANE_PAD_X: usize = 6;

pub struct Pane {
    pub term: Terminal,
    pub pty: Option<Pty>,
    /// Set once the child exits: the pane keeps draining until this passes.
    pub ending: Option<std::time::Instant>,
    /// Order of opening, shown in the header so panes stay distinguishable.
    pub number: usize,
    pub title: String,
}

impl Pane {
    pub fn label(&self) -> String {
        match short_title(&self.title) {
            Some(t) => format!(" {}  {t} ", self.number),
            None => format!(" {} ", self.number),
        }
    }
}

/// Shells announce their own executable path as the title, which says nothing.
/// Keep the last path segment, which is either the program or the directory.
fn short_title(title: &str) -> Option<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tail = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed).trim();
    (!tail.is_empty()).then(|| tail.to_string())
}

/// Every pane, the tree that arranges them, and which one has the keyboard.
pub struct Panes {
    pub tree: Tree,
    pub panes: HashMap<PaneId, Pane>,
    pub focus: PaneId,
    next_number: usize,
    header_h: usize,
    cell_w: usize,
    /// Base64 for `-EncodedCommand`, set once at startup so every pane opens
    /// with the same prompt.
    prompt: Option<String>,
}

impl Panes {
    pub fn new(header_h: usize, cell_w: usize) -> Self {
        Self {
            tree: Tree::new(),
            panes: HashMap::new(),
            focus: 0,
            next_number: 1,
            header_h,
            cell_w,
            prompt: None,
        }
    }

    pub fn set_prompt(&mut self, encoded: Option<String>) {
        self.prompt = encoded;
    }

    pub fn set_metrics(&mut self, header_h: usize, cell_w: usize) {
        self.header_h = header_h;
        self.cell_w = cell_w;
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn focused(&self) -> Option<&Pane> {
        self.panes.get(&self.focus)
    }

    pub fn focused_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.focus)
    }

    /// Creates the terminal and shell behind one pane.
    pub fn open(&mut self, id: PaneId, size: TermSize, waker: Option<Arc<dyn Fn() + Send + Sync>>) {
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        if let Some(encoded) = self.prompt.as_deref() {
            // -NoExit keeps the shell interactive after the script runs, and the
            // script runs after the profile, so it is ours that wins.
            cmd.args(["-NoExit", "-EncodedCommand", encoded]);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        let pty = match Pty::spawn_with_waker(cmd, size, waker) {
            Ok(pty) => Some(pty),
            Err(e) => {
                eprintln!("no se pudo abrir el PTY: {e}");
                None
            }
        };
        self.panes.insert(
            id,
            Pane {
                term: Terminal::with_cursor(size, CursorShape::Beam),
                pty,
                ending: None,
                number: self.next_number,
                title: String::new(),
            },
        );
        self.next_number += 1;
    }

    /// Splits the focused pane towards `zone` and focuses the new one.
    pub fn split(&mut self, zone: DropZone) -> Option<PaneId> {
        let new = self.tree.split_towards(self.focus, zone)?;
        self.focus = new;
        Some(new)
    }

    /// Closes a pane, moving focus to a neighbour. Returns false for the last one.
    pub fn close(&mut self, id: PaneId, area: Rect) -> bool {
        if self.tree.len() <= 1 {
            return false;
        }
        let next = self
            .tree
            .neighbour(id, Direction::Left, area)
            .or_else(|| self.tree.neighbour(id, Direction::Right, area))
            .or_else(|| self.tree.neighbour(id, Direction::Up, area))
            .or_else(|| self.tree.neighbour(id, Direction::Down, area));

        if !self.tree.close(id) {
            return false;
        }
        self.panes.remove(&id);
        if self.focus == id {
            self.focus = next
                .or_else(|| self.tree.panes().first().copied())
                .unwrap_or(0);
        }
        true
    }

    pub fn navigate(&mut self, direction: Direction, area: Rect) -> bool {
        match self.tree.neighbour(self.focus, direction, area) {
            Some(next) => {
                self.focus = next;
                true
            }
            None => false,
        }
    }

    pub fn rects(&self, area: Rect) -> HashMap<PaneId, Rect> {
        self.tree.layout(area)
    }

    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        self.tree.dividers(area, DIVIDER_HIT)
    }

    pub fn drag(&mut self, id: DividerId, delta: f32, area: Rect) -> bool {
        self.tree.drag_divider(id, delta, area)
    }

    /// Hit tests take the rectangles already computed instead of rebuilding the
    /// layout: the caller does it once per relayout, not once per mouse move.
    pub fn pane_at(rects: &HashMap<PaneId, Rect>, x: f64, y: f64) -> Option<PaneId> {
        let (x, y) = (x as f32, y as f32);
        rects.iter().find_map(|(id, r)| {
            (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).then_some(*id)
        })
    }

    pub fn divider_at(dividers: &[Divider], x: f64, y: f64) -> Option<Divider> {
        layout::divider_at(dividers, x as f32, y as f32)
    }

    pub fn header_at(
        &self,
        rects: &HashMap<PaneId, Rect>,
        x: f64,
        y: f64,
        show_header: bool,
    ) -> Option<PaneId> {
        if !show_header {
            return None;
        }
        let id = Self::pane_at(rects, x, y)?;
        let (header, _) = self.chrome(*rects.get(&id)?, true);
        let inside = x >= header.x as f64
            && x < (header.x + header.w) as f64
            && y >= header.y as f64
            && y < (header.y + header.h) as f64;
        inside.then_some(id)
    }

    pub fn close_button_at(
        &self,
        rects: &HashMap<PaneId, Rect>,
        x: f64,
        y: f64,
        show_header: bool,
    ) -> Option<PaneId> {
        let id = self.header_at(rects, x, y, show_header)?;
        let (header, _) = self.chrome(*rects.get(&id)?, true);
        let (bx, by, bw, bh) = self.close_button(header);
        let inside =
            x >= bx as f64 && x < (bx + bw) as f64 && y >= by as f64 && y < (by + bh) as f64;
        inside.then_some(id)
    }

    /// The close button sits at the right end of the header, one cell wide.
    pub fn close_button(&self, header: Header) -> (usize, usize, usize, usize) {
        let w = self.cell_w + PANE_PAD_X;
        let x = (header.x + header.w).saturating_sub(w);
        (x, header.y, w.min(header.w), header.h)
    }

    /// Where the glyph goes so it sits centred in the button rather than pinned
    /// to its left edge, which makes the hover highlight look lopsided.
    pub fn close_glyph_x(&self, header: Header) -> usize {
        let (x, _, w, _) = self.close_button(header);
        x + w.saturating_sub(self.cell_w) / 2
    }

    /// Where a pane dropped here would land, with the rectangle to preview.
    pub fn drop_target(
        &self,
        x: f64,
        y: f64,
        area: Rect,
        current: Option<(PaneId, DropZone)>,
    ) -> Option<(PaneId, DropZone, Rect)> {
        self.tree.drop_target(x as f32, y as f32, area, current)
    }

    pub fn move_pane(&mut self, pane: PaneId, target: PaneId, zone: DropZone) -> bool {
        self.tree.move_pane(pane, target, zone)
    }

    /// A pane's rectangle split into its header and the terminal below it.
    pub fn chrome(&self, rect: Rect, show_header: bool) -> (Header, TerminalArea) {
        let header_h = if show_header { self.header_h } else { 0 };
        let x = rect.x.round().max(0.0) as usize;
        let y = rect.y.round().max(0.0) as usize;
        let w = rect.w.round().max(0.0) as usize;
        let h = rect.h.round().max(0.0) as usize;
        (
            Header {
                x,
                y,
                w,
                h: header_h.min(h),
            },
            TerminalArea {
                x: x + PANE_PAD_X,
                y: y + header_h.min(h),
                w: w.saturating_sub(2 * PANE_PAD_X),
                h: h.saturating_sub(header_h),
            },
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalArea {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl TerminalArea {
    pub fn tuple(self) -> (usize, usize, usize, usize) {
        (self.x, self.y, self.w, self.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::Orientation;

    /// Geometry only: no shells are spawned, so this stays headless.
    fn split_panes(orientation: Orientation) -> Panes {
        let mut p = Panes::new(19, 10);
        p.tree.split(0, orientation);
        p
    }

    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 570.0,
    };

    #[test]
    fn divider_is_grabbable_around_the_boundary() {
        let p = split_panes(Orientation::Horizontal);
        let d = p.dividers(AREA);
        assert!(
            Panes::divider_at(&d, 500.0, 300.0).is_some(),
            "at the centre"
        );
        assert!(Panes::divider_at(&d, 497.0, 300.0).is_some(), "left edge");
        assert!(Panes::divider_at(&d, 503.0, 300.0).is_some(), "right edge");
        assert!(Panes::divider_at(&d, 400.0, 300.0).is_none(), "far away");
    }

    #[test]
    fn dragging_the_divider_moves_the_boundary() {
        let mut p = split_panes(Orientation::Horizontal);
        let d = Panes::divider_at(&p.dividers(AREA), 500.0, 300.0).expect("divider");
        assert!(p.drag(d.id, -200.0, AREA), "the drag should apply");

        let rects = p.rects(AREA);
        let left = rects[&0];
        assert!(
            (left.w - 300.0).abs() < 1.0,
            "boundary landed at {}, expected 300",
            left.w
        );
    }

    #[test]
    fn pane_at_finds_each_side() {
        let p = split_panes(Orientation::Horizontal);
        let rects = p.rects(AREA);
        assert_eq!(Panes::pane_at(&rects, 100.0, 300.0), Some(0));
        assert_eq!(Panes::pane_at(&rects, 900.0, 300.0), Some(1));
    }

    #[test]
    fn chrome_puts_the_terminal_below_the_header() {
        let p = split_panes(Orientation::Horizontal);
        let rect = p.rects(AREA)[&0];
        let (header, term) = p.chrome(rect, true);
        assert_eq!(header.h, 19);
        assert_eq!(term.y, header.y + 19);
        assert_eq!(term.h, rect.h as usize - 19);
    }

    #[test]
    fn the_terminal_is_inset_sideways_but_not_vertically() {
        let p = split_panes(Orientation::Horizontal);
        let rect = p.rects(AREA)[&0];
        let (header, term) = p.chrome(rect, true);
        assert_eq!(term.x, rect.x as usize + PANE_PAD_X, "left inset");
        assert_eq!(
            term.w,
            rect.w as usize - 2 * PANE_PAD_X,
            "inset on both sides"
        );
        // Vertical stays tight: only the header separates panes.
        assert_eq!(term.y, header.y + header.h);
        assert_eq!(term.h, rect.h as usize - header.h);
    }

    #[test]
    fn the_close_button_sits_at_the_right_end_of_the_header() {
        let p = split_panes(Orientation::Horizontal);
        let rect = p.rects(AREA)[&0];
        let (header, _) = p.chrome(rect, true);
        let (bx, by, bw, bh) = p.close_button(header);
        assert_eq!(bx + bw, header.x + header.w, "flush with the right edge");
        assert_eq!(by, header.y);
        assert_eq!(bh, header.h);
        assert!(bw > 0 && bw < header.w, "sensible width: {bw}");
    }

    #[test]
    fn the_glyph_is_centred_in_its_button() {
        let p = split_panes(Orientation::Horizontal);
        let rect = p.rects(AREA)[&0];
        let (header, _) = p.chrome(rect, true);
        let (bx, _, bw, _) = p.close_button(header);
        let gx = p.close_glyph_x(header);

        let left = gx - bx;
        let right = (bx + bw) - (gx + p.cell_w);
        assert!(
            left.abs_diff(right) <= 1,
            "glyph off centre: {left} px left, {right} px right"
        );
    }

    #[test]
    fn clicking_the_button_hits_it_and_the_label_does_not() {
        let p = split_panes(Orientation::Horizontal);
        let rect = p.rects(AREA)[&0];
        let (header, _) = p.chrome(rect, true);
        let (bx, by, bw, _) = p.close_button(header);

        let inside_x = (bx + bw / 2) as f64;
        let y = (by + 5) as f64;
        let rects = p.rects(AREA);
        assert_eq!(p.close_button_at(&rects, inside_x, y, true), Some(0));
        // On the header label, not the button.
        assert_eq!(p.close_button_at(&rects, 20.0, y, true), None);
        // No headers, no button.
        assert_eq!(p.close_button_at(&rects, inside_x, y, false), None);
    }

    #[test]
    fn without_headers_the_terminal_takes_the_whole_pane() {
        let p = split_panes(Orientation::Vertical);
        let rect = p.rects(AREA)[&0];
        let (header, term) = p.chrome(rect, false);
        assert_eq!(header.h, 0);
        assert_eq!(term.y, rect.y as usize);
    }

    #[test]
    fn short_title_keeps_the_last_segment() {
        assert_eq!(
            short_title(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            Some("powershell.exe".into())
        );
        assert_eq!(short_title("vim"), Some("vim".into()));
        assert_eq!(short_title("   "), None);
    }
}
