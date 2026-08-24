use std::sync::mpsc::{Receiver, Sender, channel};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Config;
use alacritty_terminal::vte::ansi::Processor;

pub mod pty;

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
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn drain_events(&self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }

    // ConPTY probes with DSR (`ESC [ 6 n`) and blocks the child until answered.
    // Dropping these replies deadlocks the terminal on startup.
    pub fn take_pty_replies(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for event in self.drain_events() {
            if let Event::PtyWrite(text) = event {
                out.extend_from_slice(text.as_bytes());
            }
        }
        out
    }

    pub fn resize(&mut self, size: TermSize) {
        self.term.resize(size);
    }

    pub fn size(&self) -> TermSize {
        TermSize::new(self.term.columns(), self.term.screen_lines())
    }

    pub fn cursor(&self) -> (usize, usize) {
        let p = self.term.grid().cursor.point;
        (p.line.0.max(0) as usize, p.column.0)
    }

    pub fn line_text(&self, line: usize) -> String {
        let grid = self.term.grid();
        let mut out = String::with_capacity(self.term.columns());
        for col in 0..self.term.columns() {
            out.push(grid[Point::new(Line(line as i32), Column(col))].c);
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
    fn resize_keeps_content() {
        let mut t = term();
        t.feed(b"hola");
        t.resize(TermSize::new(40, 10));
        assert_eq!(t.size(), TermSize::new(40, 10));
        assert_eq!(t.line_text(0), "hola");
    }
}
