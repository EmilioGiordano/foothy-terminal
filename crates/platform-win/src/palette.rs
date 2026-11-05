use config::Variant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Commands,
    Themes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    SelectTheme,
    SplitRight,
    SplitDown,
    ClosePane,
    Copy,
    Paste,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    ScrollToTop,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Stay,
    /// A theme to paint, not to keep: only Enter commits.
    Preview(usize),
    Commit(usize),
    Run(Action),
    /// Drop the preview but keep the palette open: Esc steps out of the theme
    /// list instead of closing.
    Restore,
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Home,
    End,
    Enter,
    Esc,
    Backspace,
    Char(char),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Half {
    Top,
    Bottom,
}

/// Half-cell resolution: a filled rectangle, not a glyph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Marker {
    pub row: usize,
    pub half: Half,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThemeEntry {
    pub name: String,
    pub variant: Variant,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row<'a> {
    pub label: &'a str,
    pub tag: &'a str,
    pub selected: bool,
    /// Index into the unfiltered theme list, for the caller's swatch colours.
    pub theme: Option<usize>,
}

/// Only actions main.rs already implements, each tagged with the shortcut it
/// already answers to.
const COMMANDS: [(Action, &str, &str); 10] = [
    (Action::SelectTheme, "Select theme...", ""),
    (Action::SplitRight, "Split right", "Ctrl+Shift+Right"),
    (Action::SplitDown, "Split down", "Ctrl+Shift+Down"),
    (Action::ClosePane, "Close pane", "Ctrl+Shift+W"),
    (Action::Copy, "Copy", "Ctrl+Shift+C"),
    (Action::Paste, "Paste", "Ctrl+V"),
    (Action::ZoomIn, "Zoom in", "Ctrl+Plus"),
    (Action::ZoomOut, "Zoom out", "Ctrl+Minus"),
    (Action::ResetZoom, "Reset zoom", "Ctrl+0"),
    (Action::ScrollToTop, "Scroll to top", "Shift+Home"),
];

pub struct Palette {
    mode: Mode,
    filter: String,
    /// Index into `matches`, never into the unfiltered list.
    sel: usize,
    themes: Vec<ThemeEntry>,
    current: Option<String>,
    matches: Vec<usize>,
}

impl Palette {
    /// The only constructor: the theme list is reached through `Select
    /// theme...`, so no caller can skip the mode switch.
    pub fn commands(themes: Vec<ThemeEntry>, current: Option<&str>) -> Self {
        let mut p = Self {
            mode: Mode::Commands,
            filter: String::new(),
            sel: 0,
            themes,
            current: current.map(str::to_string),
            matches: Vec::new(),
        };
        p.refilter();
        p
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Esc => match self.mode {
                Mode::Themes => self.back(),
                Mode::Commands => Outcome::Cancel,
            },
            Key::Enter => self.enter(),
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::Home => self.jump(0),
            Key::End => self.jump(self.matches.len().saturating_sub(1)),
            // winit reports Ctrl+letter as a control character; it would fill
            // the filter with something undrawable that matches nothing.
            Key::Char(c) if c.is_control() => Outcome::Stay,
            Key::Char(c) => {
                self.filter.push(c);
                self.refilter();
                self.moved()
            }
            Key::Backspace => {
                if self.filter.pop().is_some() {
                    self.refilter();
                }
                self.moved()
            }
        }
    }

    /// The wheel clamps where the arrows wrap: spatial input does not wrap.
    pub fn wheel(&mut self, down: bool) -> Outcome {
        let len = self.matches.len();
        if len == 0 {
            return Outcome::Stay;
        }
        self.sel = if down {
            (self.sel + 1).min(len - 1)
        } else {
            self.sel.saturating_sub(1)
        };
        self.moved()
    }

    pub fn visible(&self, rows: usize) -> Vec<Row<'_>> {
        let off = self.offset(rows);
        self.matches
            .iter()
            .enumerate()
            .skip(off)
            .take(rows)
            .map(|(pos, &i)| self.row(pos, i))
            .collect()
    }

    pub fn marker(&self, rows: usize) -> Option<Marker> {
        let len = self.matches.len();
        if rows == 0 || len <= rows {
            return None;
        }
        // Marker and window come from the same offset; deriving one from `sel`
        // and the other from a stored scroll position is what made them drift.
        let pos = self.offset(rows) * (2 * rows - 1) / (len - rows);
        Some(Marker {
            row: pos / 2,
            half: if pos.is_multiple_of(2) {
                Half::Top
            } else {
                Half::Bottom
            },
        })
    }

    /// Not a field: a stored offset would be stale after a resize, and the
    /// height is only known at draw time.
    fn offset(&self, rows: usize) -> usize {
        let len = self.matches.len();
        if rows == 0 || len <= rows {
            return 0;
        }
        self.sel.saturating_sub(rows / 2).min(len - rows)
    }

    fn row(&self, pos: usize, i: usize) -> Row<'_> {
        let selected = pos == self.sel;
        match self.mode {
            Mode::Commands => Row {
                label: COMMANDS[i].1,
                tag: COMMANDS[i].2,
                selected,
                theme: None,
            },
            Mode::Themes => Row {
                label: &self.themes[i].name,
                tag: match self.themes[i].variant {
                    Variant::Dark => "dark",
                    Variant::Light => "light",
                },
                selected,
                theme: Some(i),
            },
        }
    }

    fn label(&self, i: usize) -> &str {
        match self.mode {
            Mode::Commands => COMMANDS[i].1,
            Mode::Themes => &self.themes[i].name,
        }
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        let total = match self.mode {
            Mode::Commands => COMMANDS.len(),
            Mode::Themes => self.themes.len(),
        };
        self.matches = (0..total)
            .filter(|&i| needle.is_empty() || self.label(i).to_lowercase().contains(&needle))
            .collect();
        self.sel = 0;
    }

    fn select_current(&mut self) {
        if let Some(name) = &self.current
            && let Some(pos) = self
                .matches
                .iter()
                .position(|&i| self.themes[i].name == *name)
        {
            self.sel = pos;
        }
    }

    fn jump(&mut self, pos: usize) -> Outcome {
        if self.matches.is_empty() {
            return Outcome::Stay;
        }
        self.sel = pos;
        self.moved()
    }

    fn step(&mut self, delta: isize) -> Outcome {
        let len = self.matches.len();
        if len == 0 {
            return Outcome::Stay;
        }
        self.sel = (self.sel as isize + delta).rem_euclid(len as isize) as usize;
        self.moved()
    }

    fn enter(&mut self) -> Outcome {
        let Some(&i) = self.matches.get(self.sel) else {
            return Outcome::Stay;
        };
        match self.mode {
            Mode::Themes => Outcome::Commit(i),
            Mode::Commands => match COMMANDS[i].0 {
                Action::SelectTheme => {
                    self.mode = Mode::Themes;
                    // A filter typed over the commands must not narrow the themes.
                    self.filter.clear();
                    self.refilter();
                    self.select_current();
                    self.moved()
                }
                other => Outcome::Run(other),
            },
        }
    }

    /// The mirror of entering the list: the filter the theme list collected is
    /// not the one the command list had, so it cannot survive the step back.
    fn back(&mut self) -> Outcome {
        self.mode = Mode::Commands;
        self.filter.clear();
        self.refilter();
        self.sel = self
            .matches
            .iter()
            .position(|&i| COMMANDS[i].0 == Action::SelectTheme)
            .unwrap_or(0);
        Outcome::Restore
    }

    fn moved(&self) -> Outcome {
        match (self.mode, self.matches.get(self.sel)) {
            (Mode::Themes, Some(&i)) => Outcome::Preview(i),
            _ => Outcome::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(names: &[&str]) -> Vec<ThemeEntry> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| ThemeEntry {
                name: (*n).to_string(),
                variant: if i % 2 == 0 {
                    Variant::Dark
                } else {
                    Variant::Light
                },
            })
            .collect()
    }

    /// The theme list through the path a user takes, so every test below
    /// exercises the mode switch.
    fn themes(entries: Vec<ThemeEntry>, current: Option<&str>) -> Palette {
        let mut p = Palette::commands(entries, current);
        p.key(Key::Enter);
        assert_eq!(p.mode(), Mode::Themes);
        p
    }

    fn many(n: usize) -> Vec<ThemeEntry> {
        let names: Vec<String> = (0..n).map(|i| format!("theme-{i:02}")).collect();
        entries(&names.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn labels(p: &Palette, rows: usize) -> Vec<String> {
        p.visible(rows)
            .iter()
            .map(|r| r.label.to_string())
            .collect()
    }

    fn type_str(p: &mut Palette, s: &str) {
        for c in s.chars() {
            p.key(Key::Char(c));
        }
    }

    #[test]
    fn the_command_list_starts_with_select_theme() {
        let p = Palette::commands(many(3), None);
        let rows = p.visible(50);
        assert_eq!(rows[0].label, "Select theme...");
        assert_eq!(rows.len(), COMMANDS.len());
        assert!(rows[0].selected);
    }

    #[test]
    fn a_filter_narrows_and_clearing_it_restores_the_full_list() {
        let mut p = Palette::commands(many(3), None);
        type_str(&mut p, "spl");
        assert_eq!(labels(&p, 50), ["Split right", "Split down"]);
        assert_eq!(p.filter(), "spl");
        for _ in 0..3 {
            p.key(Key::Backspace);
        }
        assert_eq!(p.filter(), "");
        assert_eq!(labels(&p, 50).len(), COMMANDS.len());
    }

    #[test]
    fn a_control_character_never_reaches_the_filter() {
        let mut p = Palette::commands(many(3), None);
        type_str(&mut p, "zoom");
        assert_eq!(p.key(Key::Char('\u{1}')), Outcome::Stay);
        assert_eq!(p.filter(), "zoom");
    }

    #[test]
    fn the_filter_ignores_case() {
        let mut p = Palette::commands(many(3), None);
        type_str(&mut p, "ZOOM");
        assert_eq!(labels(&p, 50), ["Zoom in", "Zoom out", "Reset zoom"]);
    }

    #[test]
    fn typing_resets_the_selection_to_the_first_row() {
        let mut p = themes(many(40), None);
        p.key(Key::End);
        type_str(&mut p, "theme-1");
        assert_eq!(p.offset(8), 0);
        assert!(p.visible(8)[0].selected);
    }

    #[test]
    fn the_marker_and_the_highlight_never_disagree() {
        let rows = 8;
        let len = 40;
        let mut p = themes(many(len), None);
        let mut last = p.marker(rows).unwrap();
        assert_eq!(
            last,
            Marker {
                row: 0,
                half: Half::Top
            }
        );
        for step in 1..len {
            p.key(Key::Down);
            let view = p.visible(rows);
            assert_eq!(view.len(), rows);
            assert_eq!(view.iter().filter(|r| r.selected).count(), 1, "step {step}");
            let m = p.marker(rows).unwrap();
            let halves = |m: Marker| m.row * 2 + usize::from(m.half == Half::Bottom);
            assert!(halves(m) >= halves(last), "step {step}");
            assert!(m.row < rows);
            last = m;
        }
        assert_eq!(
            last,
            Marker {
                row: rows - 1,
                half: Half::Bottom
            }
        );
    }

    #[test]
    fn the_selection_stays_centred_except_at_the_ends() {
        let rows = 7;
        let len = 30;
        let mut p = themes(many(len), None);
        for sel in 0..len {
            let off = p.offset(rows);
            let pos = sel - off;
            if sel < rows / 2 {
                assert_eq!(off, 0, "head sel {sel}");
            } else if sel >= len - rows / 2 - 1 {
                assert_eq!(off, len - rows, "tail sel {sel}");
            } else {
                assert_eq!(pos, rows / 2, "middle sel {sel}");
            }
            p.key(Key::Down);
        }
    }

    #[test]
    fn a_short_list_has_no_marker() {
        let p = themes(many(4), None);
        assert!(p.marker(8).is_none());
    }

    #[test]
    fn the_arrows_wrap() {
        let mut p = themes(many(3), None);
        assert_eq!(p.key(Key::Up), Outcome::Preview(2));
        assert_eq!(p.key(Key::Down), Outcome::Preview(0));
        p.key(Key::End);
        assert_eq!(p.key(Key::Down), Outcome::Preview(0));
    }

    #[test]
    fn the_wheel_clamps() {
        let mut p = themes(many(3), None);
        assert_eq!(p.wheel(false), Outcome::Preview(0));
        for _ in 0..5 {
            p.wheel(true);
        }
        assert_eq!(p.wheel(true), Outcome::Preview(2));
    }

    #[test]
    fn an_empty_result_set_has_no_selection_no_marker_and_enter_does_nothing() {
        let mut p = themes(many(40), None);
        type_str(&mut p, "nothing-matches-this");
        assert!(p.visible(8).is_empty());
        assert!(p.marker(8).is_none());
        assert_eq!(p.key(Key::Enter), Outcome::Stay);
        assert_eq!(p.key(Key::Down), Outcome::Stay);
        assert_eq!(p.key(Key::Up), Outcome::Stay);
        assert_eq!(p.key(Key::Home), Outcome::Stay);
        assert_eq!(p.key(Key::End), Outcome::Stay);
        assert_eq!(p.wheel(true), Outcome::Stay);
    }

    #[test]
    fn enter_on_a_command_runs_it() {
        let mut p = Palette::commands(many(3), None);
        type_str(&mut p, "close");
        assert_eq!(p.key(Key::Enter), Outcome::Run(Action::ClosePane));
        assert_eq!(p.mode(), Mode::Commands);
    }

    #[test]
    fn enter_on_select_theme_switches_mode_and_drops_the_filter() {
        let mut p = Palette::commands(entries(&["one", "two", "three"]), Some("two"));
        type_str(&mut p, "theme");
        assert_eq!(p.key(Key::Enter), Outcome::Preview(1));
        assert_eq!(p.mode(), Mode::Themes);
        assert_eq!(p.filter(), "");
        assert_eq!(labels(&p, 50), ["one", "two", "three"]);
    }

    #[test]
    fn opening_the_theme_list_selects_the_current_theme() {
        let p = themes(many(40), Some("theme-25"));
        let view = p.visible(8);
        let sel = view.iter().find(|r| r.selected).unwrap();
        assert_eq!(sel.label, "theme-25");
        assert_eq!(sel.theme, Some(25));
    }

    #[test]
    fn an_unknown_current_theme_falls_back_to_the_first_row() {
        let p = themes(many(5), Some("not-bundled"));
        assert!(p.visible(8)[0].selected);
    }

    #[test]
    fn preview_and_commit_carry_the_unfiltered_theme_index() {
        let mut p = themes(many(40), None);
        type_str(&mut p, "theme-3");
        assert_eq!(p.key(Key::Down), Outcome::Preview(31));
        assert_eq!(p.key(Key::Enter), Outcome::Commit(31));
    }

    #[test]
    fn moving_in_theme_mode_previews_but_never_commits() {
        let mut p = themes(many(5), None);
        for _ in 0..5 {
            assert!(matches!(p.key(Key::Down), Outcome::Preview(_)));
        }
        assert!(matches!(p.wheel(true), Outcome::Preview(_)));
    }

    #[test]
    fn esc_cancels_the_command_list() {
        let mut p = Palette::commands(many(3), None);
        assert_eq!(p.key(Key::Esc), Outcome::Cancel);
    }

    #[test]
    fn esc_in_the_theme_list_steps_back_to_the_commands() {
        let mut p = themes(many(3), None);
        p.key(Key::Down);
        assert_eq!(p.key(Key::Esc), Outcome::Restore);
        assert_eq!(p.mode(), Mode::Commands);
        assert_eq!(p.filter(), "");
        let view = p.visible(50);
        assert_eq!(view.len(), COMMANDS.len());
        assert_eq!(
            view.iter().find(|r| r.selected).unwrap().label,
            "Select theme..."
        );
        assert_eq!(p.key(Key::Esc), Outcome::Cancel);
    }

    #[test]
    fn a_filter_typed_in_the_theme_list_does_not_survive_the_step_back() {
        let mut p = Palette::commands(many(40), None);
        type_str(&mut p, "theme");
        p.key(Key::Enter);
        type_str(&mut p, "theme-1");
        p.key(Key::Esc);
        assert_eq!(p.filter(), "");
        assert_eq!(labels(&p, 50).len(), COMMANDS.len());
    }

    #[test]
    fn a_theme_row_is_tagged_with_its_variant() {
        let p = themes(entries(&["a", "b"]), None);
        let view = p.visible(8);
        assert_eq!(view[0].tag, "dark");
        assert_eq!(view[1].tag, "light");
    }

    #[test]
    fn every_label_and_tag_is_ascii() {
        for (_, label, tag) in COMMANDS {
            assert!(label.is_ascii() && tag.is_ascii(), "{label}");
        }
    }
}
