use std::sync::mpsc::{Receiver, Sender, channel};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Config;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::Processor;

pub mod pty;

pub use alacritty_terminal::grid::Scroll;

#[derive(Clone)]
pub struct EventSink(Sender<Event>);

impl EventListener for EventSink {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    pub columns: usize,
    pub screen_lines: usize,
}

impl TermSize {
    pub fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns,
            screen_lines,
        }
    }
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

pub struct Terminal {
    term: Term<EventSink>,
    parser: Processor,
    events: Receiver<Event>,
    pending: Vec<Event>,
}

impl Terminal {
    pub fn new(size: TermSize) -> Self {
        Self::with_config(size, Config::default())
    }

    pub fn with_config(size: TermSize, config: Config) -> Self {
        let (tx, events) = channel();
        let term = Term::new(config, &size, EventSink(tx));
        Self {
            term,
            parser: Processor::new(),
            events,
            pending: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn pump(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.pending.push(event);
        }
    }

    /// Returns every event except `PtyWrite`, which only [`Terminal::take_pty_replies`] consumes.
    pub fn drain_events(&mut self) -> Vec<Event> {
        self.pump();
        let mut out = Vec::new();
        self.pending.retain(|event| {
            if matches!(event, Event::PtyWrite(_)) {
                true
            } else {
                out.push(event.clone());
                false
            }
        });
        out
    }

    // ConPTY probes with DSR (`ESC [ 6 n`) and blocks the child until answered.
    // Dropping these replies deadlocks the terminal on startup.
    pub fn take_pty_replies(&mut self) -> Vec<u8> {
        self.pump();
        let mut out = Vec::new();
        self.pending.retain(|event| {
            if let Event::PtyWrite(text) = event {
                out.extend_from_slice(text.as_bytes());
                false
            } else {
                true
            }
        });
        out
    }

    pub fn resize(&mut self, size: TermSize) {
        self.term.resize(size);
    }

    pub fn scroll(&mut self, scroll: Scroll) {
        self.term.scroll_display(scroll);
    }

    /// How many lines above the live screen the viewport is parked at.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn size(&self) -> TermSize {
        TermSize::new(self.term.columns(), self.term.screen_lines())
    }

    pub fn cursor(&self) -> (usize, usize) {
        let p = self.term.grid().cursor.point;
        (p.line.0.max(0) as usize, p.column.0)
    }

    /// `line` is relative to the viewport, so it follows the scrollback position.
    pub fn line_text(&self, line: usize) -> String {
        let grid = self.term.grid();
        let row = Line(line as i32 - grid.display_offset() as i32);
        let mut out = String::with_capacity(self.term.columns());
        for col in 0..self.term.columns() {
            let cell = &grid[Point::new(row, Column(col))];
            // A wide glyph owns two cells; the spacer is padding, not a character.
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            out.push(cell.c);
        }
        out.trim_end().to_string()
    }

    pub fn screen_text(&self) -> String {
        let mut lines: Vec<String> = (0..self.term.screen_lines())
            .map(|l| self.line_text(l))
            .collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }

    pub fn term(&self) -> &Term<EventSink> {
        &self.term
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term() -> Terminal {
        Terminal::new(TermSize::new(20, 5))
    }

    #[test]
    fn plain_text_reaches_grid() {
        let mut t = term();
        t.feed(b"hola");
        assert_eq!(t.line_text(0), "hola");
        assert_eq!(t.cursor(), (0, 4));
    }

    #[test]
    fn newline_advances_row() {
        let mut t = term();
        t.feed(b"uno\r\ndos");
        assert_eq!(t.line_text(0), "uno");
        assert_eq!(t.line_text(1), "dos");
    }

    #[test]
    fn csi_moves_cursor() {
        let mut t = term();
        t.feed(b"\x1b[3;5Hx");
        assert_eq!(t.line_text(2), "    x");
    }

    #[test]
    fn csi_2j_clears_screen() {
        let mut t = term();
        t.feed(b"basura\x1b[2J\x1b[H");
        assert_eq!(t.screen_text(), "");
    }

    #[test]
    fn escape_split_across_reads() {
        let mut t = term();
        t.feed(b"\x1b[3");
        t.feed(b";5Hx");
        assert_eq!(t.line_text(2), "    x");
    }

    #[test]
    fn utf8_split_across_reads() {
        let mut t = term();
        let bytes = "áé".as_bytes();
        t.feed(&bytes[..1]);
        t.feed(&bytes[1..]);
        assert_eq!(t.line_text(0), "áé");
    }

    #[test]
    fn backspace_moves_cursor_without_erasing() {
        let mut t = term();
        t.feed(b"abc\x08");
        assert_eq!(t.cursor(), (0, 2));
        assert_eq!(t.line_text(0), "abc");
    }

    #[test]
    fn answers_device_status_report() {
        let mut t = term();
        t.feed(b"\x1b[6n");
        let reply = t.take_pty_replies();
        assert_eq!(String::from_utf8_lossy(&reply), "\x1b[1;1R");
    }

    #[test]
    fn wide_chars_round_trip_without_spacers() {
        let mut t = term();
        t.feed("中文ab".as_bytes());
        assert_eq!(t.line_text(0), "中文ab");
    }

    #[test]
    fn drain_events_does_not_eat_pty_replies() {
        let mut t = term();
        t.feed(b"\x1b[6n");
        let _ = t.drain_events();
        assert_eq!(String::from_utf8_lossy(&t.take_pty_replies()), "\x1b[1;1R");
    }

    #[test]
    fn scrolling_up_reveals_scrollback() {
        let mut t = Terminal::new(TermSize::new(20, 3));
        for i in 1..=10 {
            t.feed(format!("linea {i}\r\n").as_bytes());
        }
        assert_eq!(t.display_offset(), 0);
        assert_eq!(t.line_text(0), "linea 9");

        t.scroll(Scroll::Delta(3));
        assert_eq!(t.display_offset(), 3);
        assert_eq!(t.line_text(0), "linea 6");

        t.scroll(Scroll::Bottom);
        assert_eq!(t.display_offset(), 0);
        assert_eq!(t.line_text(0), "linea 9");
    }

    #[test]
    fn scroll_is_clamped_at_both_ends() {
        let mut t = Terminal::new(TermSize::new(20, 3));
        for i in 1..=10 {
            t.feed(format!("linea {i}\r\n").as_bytes());
        }
        t.scroll(Scroll::Delta(9999));
        let top = t.display_offset();
        t.scroll(Scroll::Delta(1));
        assert_eq!(t.display_offset(), top, "no deberia pasarse del inicio");

        t.scroll(Scroll::Delta(-9999));
        assert_eq!(t.display_offset(), 0);
    }

    #[test]
    fn resize_keeps_content() {
        let mut t = term();
        t.feed(b"hola");
        t.resize(TermSize::new(40, 10));
        assert_eq!(t.size(), TermSize::new(40, 10));
        assert_eq!(t.line_text(0), "hola");
    }
}
