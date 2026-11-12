use std::sync::mpsc::{Receiver, Sender, channel};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::Config;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::Processor;

pub mod pty;

pub use alacritty_terminal::event::Event as TermEvent;
pub use alacritty_terminal::grid::Scroll;
pub use alacritty_terminal::index::Side;
pub use alacritty_terminal::selection::{SelectionRange, SelectionType};
pub use alacritty_terminal::term::{MIN_COLUMNS, MIN_SCREEN_LINES};
pub use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};

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

    // One column hangs `Grid::shrink_columns` forever on a wide character, and
    // no row panics the first write.
    fn floored(self) -> Self {
        Self {
            columns: self.columns.max(MIN_COLUMNS),
            screen_lines: self.screen_lines.max(MIN_SCREEN_LINES),
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

    /// `shape` is what the terminal shows until a program asks for something
    /// else with DECSCUSR.
    pub fn with_cursor(size: TermSize, shape: CursorShape) -> Self {
        let config = Config {
            default_cursor_style: CursorStyle {
                shape,
                blinking: false,
            },
            ..Config::default()
        };
        Self::with_config(size, config)
    }

    /// What the running program wants the cursor to look like.
    pub fn cursor_style(&self) -> CursorStyle {
        self.term.cursor_style()
    }

    pub fn with_config(size: TermSize, config: Config) -> Self {
        let (tx, events) = channel();
        let term = Term::new(config, &size.floored(), EventSink(tx));
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

    /// Clamped to at least [`MIN_COLUMNS`]; [`Terminal::size`] is what it took.
    pub fn resize(&mut self, size: TermSize) {
        self.term.resize(size.floored());
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

    /// Viewport row + column to the buffer coordinates the grid uses.
    fn point_at(&self, row: usize, col: usize) -> Point {
        Point::new(
            Line(row as i32 - self.display_offset() as i32),
            Column(col.min(self.term.columns().saturating_sub(1))),
        )
    }

    pub fn start_selection(&mut self, row: usize, col: usize, side: Side, ty: SelectionType) {
        let point = self.point_at(row, col);
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn update_selection(&mut self, row: usize, col: usize, side: Side) {
        let point = self.point_at(row, col);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.term.selection.as_ref()?.to_range(&self.term)
    }

    /// Whether the running program asked for bracketed paste, which is how it
    /// tells pasted text apart from typed text.
    pub fn bracketed_paste(&self) -> bool {
        self.term
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)
    }

    pub fn has_selection(&self) -> bool {
        self.selection_range().is_some()
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    /// Whether a viewport cell falls inside the current selection.
    ///
    /// Resolving the range costs a search for word and line selections, so a
    /// caller looping over cells should hoist [`Terminal::selection_range`] out
    /// and use [`Terminal::is_in_range`] instead of calling this per cell.
    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        match self.selection_range() {
            Some(range) => self.is_in_range(&range, row, col),
            None => false,
        }
    }

    pub fn is_in_range(&self, range: &SelectionRange, row: usize, col: usize) -> bool {
        let point = self.point_at(row, col);
        if range.is_block {
            point.line >= range.start.line
                && point.line <= range.end.line
                && point.column >= range.start.column
                && point.column <= range.end.column
        } else {
            point >= range.start && point <= range.end
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn term() -> Terminal {
        Terminal::new(TermSize::new(20, 5))
    }

    #[test]
    fn plain_text_reaches_grid() {
        let mut t = term();
        t.feed(b"word");
        assert_eq!(t.line_text(0), "word");
        assert_eq!(t.cursor(), (0, 4));
    }

    #[test]
    fn newline_advances_row() {
        let mut t = term();
        t.feed(b"one\r\ntwo");
        assert_eq!(t.line_text(0), "one");
        assert_eq!(t.line_text(1), "two");
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
        t.feed(b"junk\x1b[2J\x1b[H");
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
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        assert_eq!(t.display_offset(), 0);
        assert_eq!(t.line_text(0), "line 9");

        t.scroll(Scroll::Delta(3));
        assert_eq!(t.display_offset(), 3);
        assert_eq!(t.line_text(0), "line 6");

        t.scroll(Scroll::Bottom);
        assert_eq!(t.display_offset(), 0);
        assert_eq!(t.line_text(0), "line 9");
    }

    #[test]
    fn scroll_is_clamped_at_both_ends() {
        let mut t = Terminal::new(TermSize::new(20, 3));
        for i in 1..=10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        t.scroll(Scroll::Delta(9999));
        let top = t.display_offset();
        t.scroll(Scroll::Delta(1));
        assert_eq!(t.display_offset(), top, "must not scroll past the top");

        t.scroll(Scroll::Delta(-9999));
        assert_eq!(t.display_offset(), 0);
    }

    #[test]
    fn selecting_a_span_yields_its_text() {
        let mut t = term();
        t.feed(b"word stuff");
        t.start_selection(0, 0, Side::Left, SelectionType::Simple);
        t.update_selection(0, 3, Side::Right);
        assert_eq!(t.selection_text().as_deref(), Some("word"));
        assert!(t.has_selection());
    }

    #[test]
    fn selection_spans_lines() {
        let mut t = term();
        t.feed(b"one\r\ntwo");
        t.start_selection(0, 0, Side::Left, SelectionType::Simple);
        t.update_selection(1, 2, Side::Right);
        assert_eq!(t.selection_text().as_deref(), Some("one\ntwo"));
    }

    #[test]
    fn is_selected_marks_only_the_range() {
        let mut t = term();
        t.feed(b"word stuff");
        t.start_selection(0, 5, Side::Left, SelectionType::Simple);
        t.update_selection(0, 9, Side::Right);
        assert!(!t.is_selected(0, 4));
        assert!(t.is_selected(0, 5));
        assert!(t.is_selected(0, 9));
        assert!(!t.is_selected(0, 10));
        assert!(!t.is_selected(1, 5));
    }

    #[test]
    fn clearing_drops_the_selection() {
        let mut t = term();
        t.feed(b"word");
        t.start_selection(0, 0, Side::Left, SelectionType::Simple);
        t.update_selection(0, 3, Side::Right);
        t.clear_selection();
        assert!(!t.has_selection());
        assert_eq!(t.selection_text(), None);
        assert!(!t.is_selected(0, 0));
    }

    #[test]
    fn selection_follows_the_scrollback() {
        let mut t = Terminal::new(TermSize::new(20, 3));
        for i in 1..=10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        t.scroll(Scroll::Delta(3));
        t.start_selection(0, 0, Side::Left, SelectionType::Simple);
        t.update_selection(0, 5, Side::Right);
        assert_eq!(t.selection_text().as_deref(), Some("line 6"));
    }

    #[test]
    fn resize_keeps_content() {
        let mut t = term();
        t.feed(b"word");
        t.resize(TermSize::new(40, 10));
        assert_eq!(t.size(), TermSize::new(40, 10));
        assert_eq!(t.line_text(0), "word");
    }

    /// A hang here would allocate ~250 MB/s until the machine dies, so the
    /// process is killed rather than left to the harness.
    fn within_five_seconds(name: &str, body: impl FnOnce() + Send + 'static) {
        let (done, wait) = channel();
        let worker = std::thread::spawn(move || {
            body();
            let _ = done.send(());
        });
        match wait.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(()) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                std::panic::resume_unwind(worker.join().unwrap_err())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // libtest captures `eprintln!` and `exit` throws the buffer away,
                // which would leave the regression reported as a bare exit code.
                let _ = writeln!(
                    std::io::stderr(),
                    "{name} did not return in 5s: the column floor is gone"
                );
                std::process::exit(1);
            }
        }
    }

    #[test]
    fn narrow_resize_with_a_wide_char_returns() {
        within_five_seconds("narrow_resize_with_a_wide_char_returns", || {
            let mut t = Terminal::new(TermSize::new(2, 1));
            t.feed("中".as_bytes());
            t.resize(TermSize::new(1, 1));
            assert_eq!(t.size().columns, MIN_COLUMNS);
            t.resize(TermSize::new(0, 0));
            assert_eq!(t.size(), TermSize::new(MIN_COLUMNS, MIN_SCREEN_LINES));
        });
    }

    #[test]
    fn a_narrow_terminal_cannot_be_constructed() {
        for columns in [0, 1] {
            let mut t = Terminal::new(TermSize::new(columns, 0));
            assert_eq!(t.size(), TermSize::new(MIN_COLUMNS, MIN_SCREEN_LINES));
            t.feed("中".as_bytes());
            assert_eq!(t.line_text(0), "中");
        }
    }

    #[test]
    fn a_wide_char_in_the_scrollback_survives_the_floor() {
        within_five_seconds("a_wide_char_in_the_scrollback_survives_the_floor", || {
            let mut t = Terminal::new(TermSize::new(4, 1));
            t.feed("中\r\n".as_bytes());
            t.feed(b"ab");
            t.resize(TermSize::new(1, 1));
            assert_eq!(t.size().columns, MIN_COLUMNS);
            t.scroll(Scroll::Delta(1));
            assert_eq!(t.line_text(0), "中");
        });
    }
}
