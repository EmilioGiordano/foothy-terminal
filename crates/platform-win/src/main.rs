// A terminal must not open a console window of its own. Left on in debug so
// eprintln diagnostics still have somewhere to go.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chrome;
mod clipboard;
mod config_file;
mod overlay;
mod palette;
mod panes;
mod prompt;

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use input::paste::{self, Risk};
use input::{Key, Mods};
use layout::{Direction, Divider, DividerId, DropZone, Orientation, PaneId, Rect};
use panes::{DIVIDER_DRAW, Panes};
use render::{CursorLook, Renderer};
use vt_core::{CursorShape, Scroll, SelectionType, Side, TermEvent, TermSize};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{CursorIcon, Window, WindowId};

const FONT_PATH: &str = r"C:\Windows\Fonts\CascadiaMono.ttf";
const LINES_PER_NOTCH: i32 = 3;
/// ConPTY leaves bytes in flight past the child's exit, so reading has to go on
/// for a while or the last output is lost.
const TAIL_GRACE: Duration = Duration::from_millis(60);

const HEADER_FOCUSED: u32 = 0x0027_4a5c;
const HEADER_IDLE: u32 = 0x001a_2f38;
const HEADER_TEXT: u32 = 0x00cf_ecf7;
const HEADER_TEXT_IDLE: u32 = 0x0078_8f99;
const DIVIDER_COLOR: u32 = 0x0031_5a6d;
const DROP_TINT: u32 = 0x004e_9ad6;
const DROP_ALPHA: u8 = 110;
const PREVIEW_FADE: Duration = Duration::from_millis(90);
const PREVIEW_GLIDE: Duration = Duration::from_millis(90);
const PREVIEW_FRAME: Duration = Duration::from_millis(8);
const CLOSE_HOVER: u32 = 0x00b3_4a4a;
/// Picked at runtime: a font without the glyph draws tofu instead.
const CLOSE_GLYPHS: [char; 4] = ['×', '╳', '⨯', 'x'];
const FOCUS_GLYPHS: [char; 3] = ['▍', '▌', '|'];

fn prompt_command(cfg: &config::Config, renderer: &Renderer) -> Option<String> {
    if !cfg.prompt {
        return None;
    }
    let glyphs = prompt::Glyphs::pick(|c| renderer.has_glyph(c));
    let colors = config::chip_colors(&cfg.colors, cfg.prompt_style);
    Some(prompt::encoded_command(&prompt::script(&glyphs, &colors)))
}

fn preview(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let shown: String = first.chars().take(70).collect();
    if first.chars().count() > 70 {
        format!("{shown}…")
    } else {
        shown
    }
}

fn warning_for(risk: Risk, text: &str) -> Option<String> {
    match risk {
        Risk::Safe => None,
        Risk::WouldExecute { lines } => Some(format!(
            "You are about to paste {lines} lines.\n\n\
             The running program did not ask for bracketed paste, so every line break \
             runs just as if you had pressed Enter.\n\n\
             It starts with:\n    {}\n\nPaste anyway?",
            preview(text)
        )),
        Risk::Large { bytes } => Some(format!(
            "You are about to paste {bytes} bytes on a single line.\n\n\
             It starts with:\n    {}\n\nPaste anyway?",
            preview(text)
        )),
    }
}

fn translate(key: &WKey, text: Option<&str>) -> Option<Key> {
    Some(match key {
        WKey::Named(NamedKey::Enter) => Key::Enter,
        WKey::Named(NamedKey::Backspace) => Key::Backspace,
        WKey::Named(NamedKey::Tab) => Key::Tab,
        WKey::Named(NamedKey::Escape) => Key::Escape,
        WKey::Named(NamedKey::Delete) => Key::Delete,
        WKey::Named(NamedKey::Insert) => Key::Insert,
        WKey::Named(NamedKey::Home) => Key::Home,
        WKey::Named(NamedKey::End) => Key::End,
        WKey::Named(NamedKey::PageUp) => Key::PageUp,
        WKey::Named(NamedKey::PageDown) => Key::PageDown,
        WKey::Named(NamedKey::ArrowUp) => Key::Up,
        WKey::Named(NamedKey::ArrowDown) => Key::Down,
        WKey::Named(NamedKey::ArrowLeft) => Key::Left,
        WKey::Named(NamedKey::ArrowRight) => Key::Right,
        WKey::Named(NamedKey::Space) => Key::Char(' '),
        WKey::Named(NamedKey::F1) => Key::F(1),
        WKey::Named(NamedKey::F2) => Key::F(2),
        WKey::Named(NamedKey::F3) => Key::F(3),
        WKey::Named(NamedKey::F4) => Key::F(4),
        WKey::Named(NamedKey::F5) => Key::F(5),
        WKey::Named(NamedKey::F6) => Key::F(6),
        WKey::Named(NamedKey::F7) => Key::F(7),
        WKey::Named(NamedKey::F8) => Key::F(8),
        WKey::Named(NamedKey::F9) => Key::F(9),
        WKey::Named(NamedKey::F10) => Key::F(10),
        WKey::Named(NamedKey::F11) => Key::F(11),
        WKey::Named(NamedKey::F12) => Key::F(12),
        WKey::Character(s) => Key::Char(s.chars().next()?),
        _ => Key::Char(text?.chars().next()?),
    })
}

fn arrow_zone(key: &WKey) -> Option<DropZone> {
    match key {
        WKey::Named(NamedKey::ArrowLeft) => Some(DropZone::Left),
        WKey::Named(NamedKey::ArrowRight) => Some(DropZone::Right),
        WKey::Named(NamedKey::ArrowUp) => Some(DropZone::Top),
        WKey::Named(NamedKey::ArrowDown) => Some(DropZone::Bottom),
        _ => None,
    }
}

/// Keys the palette answers to. Everything else becomes filter text, and a
/// control character is dropped here: Tab and Ctrl+letter arrive as text too,
/// and neither matches nor draws.
fn palette_key(key: &WKey, text: Option<&str>) -> Option<palette::Key> {
    Some(match key {
        WKey::Named(NamedKey::ArrowUp) => palette::Key::Up,
        WKey::Named(NamedKey::ArrowDown) => palette::Key::Down,
        WKey::Named(NamedKey::Home) => palette::Key::Home,
        WKey::Named(NamedKey::End) => palette::Key::End,
        WKey::Named(NamedKey::Enter) => palette::Key::Enter,
        WKey::Named(NamedKey::Escape) => palette::Key::Esc,
        WKey::Named(NamedKey::Backspace) => palette::Key::Backspace,
        _ => {
            let c = text?.chars().next().filter(|c| !c.is_control())?;
            palette::Key::Char(c)
        }
    })
}

fn arrow_direction(key: &WKey) -> Option<Direction> {
    match key {
        WKey::Named(NamedKey::ArrowLeft) => Some(Direction::Left),
        WKey::Named(NamedKey::ArrowRight) => Some(Direction::Right),
        WKey::Named(NamedKey::ArrowUp) => Some(Direction::Up),
        WKey::Named(NamedKey::ArrowDown) => Some(Direction::Down),
        _ => None,
    }
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    let at = |a: f32, b: f32| a + (b - a) * t;
    Rect::new(
        at(from.x, to.x),
        at(from.y, to.y),
        at(from.w, to.w),
        at(from.h, to.h),
    )
}

struct DropPreview {
    target: (PaneId, DropZone),
    from: Rect,
    to: Rect,
    moved: Instant,
    /// Kept across retargets: a fade restarted per zone would blink.
    shown: Instant,
}

impl DropPreview {
    fn new(target: (PaneId, DropZone), rect: Rect) -> Self {
        let now = Instant::now();
        Self {
            target,
            from: rect,
            to: rect,
            moved: now,
            shown: now,
        }
    }

    fn retarget(&mut self, target: (PaneId, DropZone), rect: Rect) {
        self.from = self.rect();
        self.to = rect;
        self.moved = Instant::now();
        self.target = target;
    }

    fn rect(&self) -> Rect {
        let t = self.moved.elapsed().div_duration_f32(PREVIEW_GLIDE);
        lerp_rect(self.from, self.to, ease_out(t))
    }

    fn fade(&self) -> f32 {
        ease_out(self.shown.elapsed().div_duration_f32(PREVIEW_FADE))
    }

    fn alpha(&self) -> u8 {
        (DROP_ALPHA as f32 * self.fade()).round() as u8
    }

    fn outline_alpha(&self) -> u8 {
        (255.0 * self.fade()).round() as u8
    }

    fn animating(&self) -> bool {
        self.moved.elapsed() < PREVIEW_GLIDE || self.shown.elapsed() < PREVIEW_FADE
    }
}

struct App {
    proxy: EventLoopProxy<()>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    _context: Option<softbuffer::Context<Rc<Window>>>,
    renderer: Renderer,
    panes: Panes,
    mods: Mods,
    size: TermSize,
    dirty: bool,
    hwnd: isize,
    mouse: (f64, f64),
    selecting: bool,
    /// Rebuilt only when the layout changes: a mouse move used to recompute it
    /// half a dozen times, allocating a map each time.
    rects: HashMap<PaneId, Rect>,
    dividers: Vec<Divider>,
    dragging: Option<DividerId>,
    moving: Option<PaneId>,
    preview: Option<DropPreview>,
    last_frame: Instant,
    cursor_icon: CursorIcon,
    hover_close: Option<PaneId>,
    close_glyph: char,
    focus_glyph: char,
    last_click: Option<(Instant, usize, usize)>,
    click_streak: u8,
    focused: bool,
    blink: Option<Duration>,
    blink_on: bool,
    blink_since: Instant,
    /// The look currently painted, which a preview replaces and Esc restores.
    cfg: config::Config,
    /// state.ini and foothy.ini without a theme under them: a preview composes
    /// from this, so the user's own colours stay on top of the candidate.
    files: config::Partial,
    pins: config_file::Pins,
    current_theme: Option<String>,
    state: config_file::State,
    /// ~86 theme files: built on the first open and kept, so opening the
    /// palette stays bounded work (docs/02 §3 rule 7).
    themes: Option<overlay::ThemeList>,
    open: Option<Opened>,
    /// Shown in the palette footer, cleared when it opens and on every mode
    /// change. A failed write has nowhere else to go: stderr is gone in
    /// release (windows_subsystem).
    status: Option<String>,
}

struct Opened {
    palette: palette::Palette,
    /// Restored by Esc, including the titlebar variant.
    saved: config::Config,
    /// Last previewed row, so a repeated index does not repaint. Keyed by
    /// mode: two lists number their rows from zero.
    shown: Option<(palette::Mode, usize)>,
}

/// A file that exists is not a font, so both the read and the parse fall back.
fn build_renderer(font: Option<&str>, px: f32, palette: &[u32; 256]) -> anyhow::Result<Renderer> {
    if let Some(path) = font {
        match std::fs::read(path).map_err(|e| anyhow::anyhow!("{e}")) {
            Ok(bytes) => match Renderer::new(&bytes, px, palette) {
                Ok(r) => return Ok(r),
                Err(e) => eprintln!("foothy: font '{path}': {e}"),
            },
            Err(e) => eprintln!("foothy: font '{path}': {e}"),
        }
    }
    Renderer::new(&std::fs::read(FONT_PATH)?, px, palette)
}

impl App {
    fn new(proxy: EventLoopProxy<()>) -> anyhow::Result<Self> {
        let loaded = config_file::load_from_disk();
        for problem in &loaded.problems {
            eprintln!("foothy: {problem}");
        }
        let cfg = loaded.config;
        let current_theme = loaded.files.settings.theme.clone();
        let state = loaded.state.clone();

        let mut renderer = build_renderer(cfg.font.as_deref(), cfg.font_size, &cfg.colors.palette)?;
        renderer.set_padding(cfg.pad as usize);
        renderer.set_colors(cfg.colors.foreground, cfg.colors.background);
        let header_h = renderer.header_height();
        let cell_w = renderer.cell_size().0;
        let pick = |candidates: &[char], fallback: char| {
            candidates
                .iter()
                .copied()
                .find(|c| renderer.has_glyph(*c))
                .unwrap_or(fallback)
        };
        let close_glyph = pick(&CLOSE_GLYPHS, 'x');
        let focus_glyph = pick(&FOCUS_GLYPHS, '|');

        let mut panes = Panes::new(header_h, cell_w);
        panes.set_prompt(prompt_command(&cfg, &renderer));
        Ok(Self {
            proxy,
            window: None,
            surface: None,
            _context: None,
            renderer,
            panes,
            mods: Mods::default(),
            size: TermSize::new(100, 30),
            dirty: true,
            hwnd: 0,
            mouse: (0.0, 0.0),
            selecting: false,
            rects: HashMap::new(),
            dividers: Vec::new(),
            dragging: None,
            moving: None,
            preview: None,
            last_frame: Instant::now(),
            cursor_icon: CursorIcon::Default,
            hover_close: None,
            close_glyph,
            focus_glyph,
            last_click: None,
            click_streak: 0,
            focused: true,
            blink: None,
            blink_on: true,
            blink_since: Instant::now(),
            cfg,
            files: loaded.files,
            pins: loaded.pins,
            current_theme,
            state,
            themes: None,
            open: None,
            status: None,
        })
    }

    fn waker(&self) -> Arc<dyn Fn() + Send + Sync> {
        let proxy = self.proxy.clone();
        Arc::new(move || {
            let _ = proxy.send_event(());
        })
    }

    fn window_px(&self) -> (usize, usize) {
        self.window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width as usize, s.height as usize)
            })
            .unwrap_or((0, 0))
    }

    fn content(&self) -> Rect {
        let (w, h) = self.window_px();
        Rect::new(0.0, 0.0, w as f32, h as f32)
    }

    fn show_headers(&self) -> bool {
        self.panes.len() > 1
    }

    fn send(&mut self, bytes: &[u8]) {
        if let Some(pane) = self.panes.focused_mut()
            && let Some(pty) = pane.pty.as_mut()
        {
            let _ = pty.write(bytes);
        }
    }

    fn pump(&mut self) {
        let ids: Vec<PaneId> = self.panes.panes.keys().copied().collect();
        let mut finished = Vec::new();
        let mut pending = false;

        for id in ids {
            let Some(pane) = self.panes.panes.get_mut(&id) else {
                continue;
            };
            let Some(pty) = pane.pty.as_mut() else {
                continue;
            };
            let read = pty.drain(10);
            let exited = pty.child_exited().is_some();
            pending |= read.more;

            if !read.bytes.is_empty() {
                pane.term.feed(&read.bytes);
                self.dirty = true;
                let replies = pane.term.take_pty_replies();
                if !replies.is_empty() {
                    let _ = pty.write(&replies);
                }
                for event in pane.term.drain_events() {
                    if let TermEvent::Title(t) = event {
                        pane.title = t;
                    }
                }
            }
            if read.eof || exited {
                finished.push(id);
            }
        }

        for id in finished {
            self.end_pane(id);
        }
        self.reap_ending();

        // Work is capped per turn, so the leftovers have to be claimed here: the
        // waker only fires for new bytes.
        if pending {
            let _ = self.proxy.send_event(());
        }
    }

    /// The tail is collected over the following turns, never by sleeping here:
    /// this runs on the UI thread.
    fn end_pane(&mut self, id: PaneId) {
        if let Some(pane) = self.panes.panes.get_mut(&id)
            && pane.ending.is_none()
        {
            pane.ending = Some(Instant::now() + TAIL_GRACE);
        }
    }

    fn reap_ending(&mut self) {
        let now = Instant::now();
        let done: Vec<PaneId> = self
            .panes
            .panes
            .iter()
            .filter(|(_, p)| p.ending.is_some_and(|deadline| now >= deadline))
            .map(|(id, _)| *id)
            .collect();

        let reaped = !done.is_empty();
        for id in done {
            if let Some(pane) = self.panes.panes.get_mut(&id) {
                pane.pty = None;
            }
            let area = self.content();
            if !self.panes.close(id, area) {
                self.panes.panes.remove(&id);
            } else {
                self.relayout();
            }
        }
        // about_to_wait reaches here and schedules no frame of its own: without
        // this the closed pane stays painted until the next input.
        if reaped {
            self.redraw_soon();
        }
    }

    fn ending_deadline(&self) -> Option<Instant> {
        self.panes.panes.values().filter_map(|p| p.ending).min()
    }

    fn relayout(&mut self) {
        let area = self.content();
        if area.w <= 0.0 || area.h <= 0.0 {
            return;
        }
        let show = self.show_headers();
        self.rects = self.panes.rects(area);
        self.dividers = self.panes.dividers(area);
        let rects = self.rects.clone();
        for (id, rect) in rects {
            let (_, term_area) = self.panes.chrome(rect, show);
            let size = self.renderer.grid_size(term_area.w, term_area.h);
            if let Some(pane) = self.panes.panes.get_mut(&id)
                && pane.term.size() != size
            {
                pane.term.resize(size);
                if let Some(pty) = pane.pty.as_ref() {
                    let _ = pty.resize(size);
                }
            }
        }
        self.dirty = true;
    }

    fn pane_at(&self, x: f64, y: f64) -> Option<PaneId> {
        Panes::pane_at(&self.rects, x, y)
    }

    fn divider_at(&self, x: f64, y: f64) -> Option<Divider> {
        Panes::divider_at(&self.dividers, x, y)
    }

    fn header_at(&self, x: f64, y: f64) -> Option<PaneId> {
        self.panes.header_at(&self.rects, x, y, self.show_headers())
    }

    fn close_button_at(&self, x: f64, y: f64) -> Option<PaneId> {
        self.panes
            .close_button_at(&self.rects, x, y, self.show_headers())
    }

    /// Returns `(pane, row, col, right_half)`.
    fn hit(&self, x: f64, y: f64) -> Option<(PaneId, usize, usize, bool)> {
        let id = self.pane_at(x, y)?;
        let rect = *self.rects.get(&id)?;
        let (_, term_area) = self.panes.chrome(rect, self.show_headers());
        let pane = self.panes.panes.get(&id)?;
        let (row, col, right) = self.renderer.cell_at(
            x - term_area.x as f64,
            y - term_area.y as f64,
            term_area.w,
            term_area.h,
            pane.term.size(),
        );
        Some((id, row, col, right))
    }

    fn split(&mut self, zone: DropZone) {
        let size = self
            .panes
            .focused()
            .map(|p| p.term.size())
            .unwrap_or(self.size);
        if let Some(new) = self.panes.split(zone) {
            let waker = self.waker();
            self.panes.open(new, size, Some(waker));
            self.relayout();
            self.redraw_soon();
        }
    }

    fn close_focused(&mut self, event_loop: &ActiveEventLoop) {
        if self.panes.len() <= 1 {
            event_loop.exit();
            return;
        }
        self.end_pane(self.panes.focus);
        self.redraw_soon();
    }

    fn navigate(&mut self, direction: Direction) {
        let area = self.content();
        if self.panes.navigate(direction, area) {
            self.restart_blink();
            self.redraw_soon();
        }
    }

    fn copy_selection(&mut self) -> bool {
        match self.panes.focused().and_then(|p| p.term.selection_text()) {
            Some(text) => clipboard::set_text(&text),
            None => false,
        }
    }

    /// Copying clears the selection, like foot. Shared by Ctrl+C, Ctrl+Shift+C
    /// and the palette, so they cannot drift apart.
    fn copy(&mut self) -> bool {
        if !self.copy_selection() {
            return false;
        }
        if let Some(pane) = self.panes.focused_mut() {
            pane.term.clear_selection();
        }
        self.redraw_soon();
        true
    }

    /// None once the text is on the clipboard. The two failures read
    /// differently on purpose: one is the user's to fix, the other is not.
    fn copy_notice(&mut self) -> Option<String> {
        // selection_text, not has_selection: a click leaves a range that holds
        // no text, and "copy failed" would be a lie for it.
        if self
            .panes
            .focused()
            .and_then(|p| p.term.selection_text())
            .is_none()
        {
            return Some("Nothing selected: select with the mouse, then copy.".into());
        }
        if !self.copy() {
            return Some("Copy failed: the clipboard would not take the text.".into());
        }
        None
    }

    fn theme_list(&mut self) -> &overlay::ThemeList {
        self.themes.get_or_insert_with(|| {
            let (list, problems) = overlay::ThemeList::build();
            for problem in problems {
                eprintln!("foothy: {problem}");
            }
            list
        })
    }

    fn open_palette(&mut self) {
        if self.open.is_some() {
            return;
        }
        // The guard swallows the button release, so anything armed by the
        // mouse would resume once the palette closes.
        self.dragging = None;
        self.moving = None;
        self.preview = None;
        self.selecting = false;
        self.set_cursor(CursorIcon::Default);
        // A notice belongs to the session that produced it.
        self.status = None;

        let current = self.current_theme.clone();
        let style = self.cfg.prompt_style;
        let entries = self.theme_list().entries.clone();
        self.open = Some(Opened {
            palette: palette::Palette::commands(entries, current.as_deref(), style),
            saved: self.cfg.clone(),
            shown: None,
        });
        self.redraw_soon();
    }

    /// Composed through the same function `load` uses, so what a preview
    /// paints is what Enter will commit.
    fn composed(&self, theme: config::Partial) -> config::Config {
        config_file::compose(&self.files, theme, &|k| std::env::var(k).ok())
    }

    fn apply_look(&mut self, cfg: &config::Config) {
        self.renderer
            .set_colors(cfg.colors.foreground, cfg.colors.background);
        self.renderer.set_palette(&cfg.colors.palette);
        // The chip's colours are baked into the script, so a theme change has
        // to rebuild it; only a pane opened after this one sees the new one.
        self.panes.set_prompt(prompt_command(cfg, &self.renderer));
        if self.hwnd != 0 {
            chrome::set_dark_titlebar(self.hwnd as _, cfg.variant == config::Variant::Dark);
        }
    }

    fn show_theme(&mut self, i: usize) {
        let Some(layer) = self.themes.as_ref().map(|l| l.layers[i].clone()) else {
            return;
        };
        let cfg = self.composed(layer);
        self.apply_look(&cfg);
        self.cfg = cfg;
    }

    /// Clearing `shown` is part of it: left set, re-entering the theme list on
    /// the same index would be taken for a repeat and skip the repaint.
    fn show_style(&mut self, style: config::ChipStyle) {
        let mut cfg = self.cfg.clone();
        cfg.prompt_style = style;
        self.apply_look(&cfg);
        self.cfg = cfg;
    }

    fn restore_look(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.shown.take().is_none() {
            return;
        }
        let saved = open.saved.clone();
        self.apply_look(&saved);
        self.cfg = saved;
    }

    fn close_palette(&mut self, restore: bool) {
        if self.open.is_none() {
            return;
        }
        if restore {
            self.restore_look();
        }
        self.open = None;
        self.redraw_soon();
    }

    fn palette_key(&mut self, event: &KeyEvent, event_loop: &ActiveEventLoop) {
        let text = event.text.as_ref().map(|s| s.as_str());
        let Some(key) = palette_key(&event.logical_key, text) else {
            return;
        };
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let outcome = open.palette.key(key);
        self.palette_outcome(outcome, event_loop);
    }

    fn palette_wheel(&mut self, down: bool, event_loop: &ActiveEventLoop) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let outcome = open.palette.wheel(down);
        self.palette_outcome(outcome, event_loop);
    }

    fn palette_outcome(&mut self, outcome: palette::Outcome, event_loop: &ActiveEventLoop) {
        // Before the match: an arm that sets a notice this turn must keep it.
        if let Some(open) = self.open.as_mut()
            && open.palette.take_mode_change()
        {
            self.status = None;
        }
        match outcome {
            palette::Outcome::Stay => self.redraw_soon(),
            palette::Outcome::Preview(i) => {
                let Some(mode) = self.open.as_ref().map(|o| o.palette.mode()) else {
                    return;
                };
                // Backspace on an empty filter and the wheel at its clamp both
                // repeat the index; repainting the window for that is waste.
                let repeat = self
                    .open
                    .as_ref()
                    .is_some_and(|o| o.shown == Some((mode, i)));
                if !repeat {
                    if let Some(open) = self.open.as_mut() {
                        open.shown = Some((mode, i));
                    }
                    match mode {
                        palette::Mode::Themes => self.show_theme(i),
                        palette::Mode::Styles => {
                            if let Some(style) = palette::style(i) {
                                self.show_style(style);
                            }
                        }
                        palette::Mode::Commands => {}
                    }
                }
                self.redraw_soon();
            }
            palette::Outcome::Commit(i) => {
                let failed = match self.open.as_ref().map(|o| o.palette.mode()) {
                    Some(palette::Mode::Themes) => self.commit_theme(i),
                    Some(palette::Mode::Styles) => self.commit_style(i),
                    Some(palette::Mode::Commands) | None => false,
                };
                if failed {
                    // The notice is drawn inside the palette and nowhere else,
                    // so a failed save keeps it open to be read. Esc must then
                    // keep the pick, or the notice becomes a lie.
                    if let Some(open) = self.open.as_mut() {
                        open.saved = self.cfg.clone();
                        open.palette.set_current_style(self.cfg.prompt_style);
                        if let Some(name) = &self.current_theme {
                            open.palette.set_current(name);
                        }
                    }
                } else {
                    self.open = None;
                }
                self.redraw_soon();
            }
            // Copy and Paste can have nothing to do, and a row that does
            // nothing and says nothing is indistinguishable from a bug. The
            // notice lives in the palette footer, so they are answered here,
            // before the close.
            palette::Outcome::Run(palette::Action::Copy) => match self.copy_notice() {
                Some(note) => {
                    self.status = Some(note);
                    self.redraw_soon();
                }
                None => self.close_palette(true),
            },
            palette::Outcome::Run(palette::Action::Paste) => {
                match clipboard::get_text().filter(|t| !t.is_empty()) {
                    Some(text) => {
                        // Closed first: the paste warning is a modal dialog.
                        self.close_palette(true);
                        self.paste_text(&text);
                        self.redraw_soon();
                    }
                    None => {
                        self.status = Some("Nothing to paste: the clipboard has no text.".into());
                        self.redraw_soon();
                    }
                }
            }
            palette::Outcome::Run(action) => {
                // Closed first: Close pane may exit.
                self.close_palette(true);
                self.run_action(action, event_loop);
            }
            palette::Outcome::Restore => {
                self.restore_look();
                self.redraw_soon();
            }
            palette::Outcome::Cancel => self.close_palette(true),
        }
    }

    fn commit_theme(&mut self, i: usize) -> bool {
        let Some(name) = self.themes.as_ref().map(|l| l.entries[i].name.clone()) else {
            return false;
        };
        self.show_theme(i);
        self.current_theme = Some(name.clone());
        self.state.theme = Some(name);
        self.save_state(config_file::Field::Theme)
    }

    fn commit_style(&mut self, i: usize) -> bool {
        let Some(style) = palette::style(i) else {
            return false;
        };
        self.show_style(style);
        // Also into `files`: a later theme preview re-composes from it and
        // would otherwise bring the old style back.
        self.files.settings.prompt_style = Some(style);
        self.state.prompt_style = Some(style);
        self.save_state(config_file::Field::PromptStyle)
    }

    fn save_state(&mut self, field: config_file::Field) -> bool {
        match config_file::save_state(&self.state, field) {
            Ok(()) => {
                self.status = None;
                false
            }
            Err(e) => {
                eprintln!("foothy: could not write {}: {e}", config_file::STATE_FILE);
                self.status = Some(format!(
                    "Could not save {}: the pick applies to this session only.",
                    config_file::STATE_FILE
                ));
                true
            }
        }
    }

    fn run_action(&mut self, action: palette::Action, event_loop: &ActiveEventLoop) {
        match action {
            // The state machine answers this one by switching mode.
            palette::Action::SelectTheme | palette::Action::SelectPromptStyle => {}
            palette::Action::SplitRight => self.split(DropZone::Right),
            palette::Action::SplitDown => self.split(DropZone::Bottom),
            palette::Action::ClosePane => self.close_focused(event_loop),
            // Both answered in palette_outcome, where the palette is still
            // open to carry their notice.
            palette::Action::Copy | palette::Action::Paste => {}
            palette::Action::ZoomIn => self.zoom(1),
            palette::Action::ZoomOut => self.zoom(-1),
            palette::Action::ResetZoom => self.reset_zoom(),
            palette::Action::ScrollToTop => self.scroll_focused(Scroll::Top),
        }
    }

    /// While the palette is open nothing reaches the panes or the PTY. Returns
    /// true when the event was the palette's.
    fn palette_event(&mut self, event: &WindowEvent, event_loop: &ActiveEventLoop) -> bool {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    self.palette_key(event, event_loop);
                }
                true
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let down = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y < 0.0,
                    MouseScrollDelta::PixelDelta(p) => p.y < 0.0,
                };
                self.palette_wheel(down, event_loop);
                true
            }
            WindowEvent::MouseInput { .. } => true,
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x, position.y);
                true
            }
            _ => false,
        }
    }

    /// The pin notice belongs with the theme list, where it is read before
    /// Enter: a successful commit closes the palette and nobody would see it.
    /// A failed save, and a command with nothing to do, keep the palette open
    /// for their own notice.
    fn palette_notes(&self, mode: palette::Mode) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(status) = self.status.clone() {
            notes.push(status);
        }
        match mode {
            palette::Mode::Themes => notes.extend(self.pins.notes()),
            palette::Mode::Styles => notes.extend(self.pins.style_notes()),
            palette::Mode::Commands => {}
        }
        notes
    }

    fn paste(&mut self) {
        if let Some(text) = clipboard::get_text().filter(|t| !t.is_empty()) {
            self.paste_text(&text);
        }
    }

    /// Split from the clipboard read so the palette can report an empty
    /// clipboard before it closes.
    fn paste_text(&mut self, text: &str) {
        let bracketed = self
            .panes
            .focused()
            .is_some_and(|p| p.term.bracketed_paste());

        if self.cfg.paste_warning
            && let Some(message) = warning_for(paste::assess(text, bracketed), text)
            && !chrome::confirm(self.hwnd as _, "Confirm paste", &message)
        {
            return;
        }

        let text = paste::normalize(text);
        if bracketed {
            self.send(&paste::bracket(&text));
        } else {
            self.send(text.as_bytes());
        }
        if let Some(pane) = self.panes.focused_mut() {
            pane.term.scroll(Scroll::Bottom);
        }
    }

    /// Back to the configured size, not to a hardcoded one.
    fn reset_zoom(&mut self) {
        let base = self.cfg.font_size;
        self.apply_font_size(base);
    }

    fn scroll_focused(&mut self, scroll: Scroll) {
        if let Some(pane) = self.panes.focused_mut() {
            pane.term.scroll(scroll);
        }
        self.redraw_soon();
    }

    fn zoom(&mut self, steps: i32) {
        let target = render::step_font_size(self.renderer.font_size(), steps);
        self.apply_font_size(target);
    }

    fn apply_font_size(&mut self, px: f32) {
        if !self.renderer.set_font_size(px) {
            return;
        }
        self.panes
            .set_metrics(self.renderer.header_height(), self.renderer.cell_size().0);
        self.relayout();
        self.redraw_soon();
    }

    fn cursor_look(&self) -> CursorLook {
        if !self.focused {
            return CursorLook::Hollow;
        }
        if !self.blink_on {
            return CursorLook::Hidden;
        }
        match self.panes.focused().map(|p| p.term.cursor_style().shape) {
            Some(CursorShape::Block) => CursorLook::Block,
            Some(CursorShape::Underline) => CursorLook::Underline,
            Some(CursorShape::Beam) => CursorLook::Beam,
            Some(CursorShape::HollowBlock) => CursorLook::Hollow,
            Some(CursorShape::Hidden) | None => CursorLook::Hidden,
        }
    }

    fn restart_blink(&mut self) {
        self.blink_on = true;
        self.blink_since = Instant::now();
    }

    fn set_cursor(&mut self, icon: CursorIcon) {
        if self.cursor_icon == icon {
            return;
        }
        self.cursor_icon = icon;
        if let Some(w) = self.window.as_ref() {
            w.set_cursor(icon);
        }
    }

    fn redraw_soon(&mut self) {
        self.dirty = true;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let look = self.cursor_look();
        let show = self.show_headers();
        let focus = self.panes.focus;

        let Some(window) = self.window.clone() else {
            return;
        };
        let phys = window.inner_size();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(phys.width), NonZeroU32::new(phys.height))
        else {
            return;
        };
        let (w, h) = (phys.width as usize, phys.height as usize);

        let preview = self
            .preview
            .as_ref()
            .map(|p| (p.rect(), p.alpha(), p.outline_alpha()));
        let hover_close = self.hover_close;
        let notes = self
            .open
            .as_ref()
            .map(|o| self.palette_notes(o.palette.mode()))
            .unwrap_or_default();
        let close_glyph = self.close_glyph.to_string();
        let focus_glyph = self.focus_glyph;

        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };

        self.renderer.clear(&mut buffer);

        // Sorted so panes always paint in the same order.
        let mut ordered: Vec<(PaneId, Rect)> = self.rects.iter().map(|(id, r)| (*id, *r)).collect();
        ordered.sort_by_key(|(id, _)| *id);

        for (id, rect) in ordered {
            let (header, term_area) = self.panes.chrome(rect, show);
            let is_focus = id == focus;

            if show && header.h > 0 {
                let bg = if is_focus {
                    HEADER_FOCUSED
                } else {
                    HEADER_IDLE
                };
                let fg = if is_focus {
                    HEADER_TEXT
                } else {
                    HEADER_TEXT_IDLE
                };
                self.renderer.fill_area(
                    &mut buffer,
                    w,
                    h,
                    (header.x, header.y, header.w, header.h),
                    bg,
                );
                let label = self
                    .panes
                    .panes
                    .get(&id)
                    .map(|p| p.label())
                    .unwrap_or_default();
                // A marker, not colour alone, says which pane has the keyboard.
                let text = if is_focus {
                    format!("{focus_glyph}{label}")
                } else {
                    format!(" {label}")
                };

                let (bx, by, bw, bh) = self.panes.close_button(header);
                let label_w = bx.saturating_sub(header.x);
                self.renderer.draw_text(
                    &mut buffer,
                    w,
                    h,
                    header.x,
                    header.y,
                    label_w,
                    &text,
                    fg,
                    bg,
                );

                let hovered = hover_close == Some(id);
                if hovered {
                    self.renderer
                        .fill_area(&mut buffer, w, h, (bx, by, bw, bh), CLOSE_HOVER);
                }
                let gx = self.panes.close_glyph_x(header);
                self.renderer.draw_text(
                    &mut buffer,
                    w,
                    h,
                    gx,
                    by,
                    bx + bw - gx,
                    &close_glyph,
                    if hovered { HEADER_TEXT } else { fg },
                    if hovered { CLOSE_HOVER } else { bg },
                );
            }

            if let Some(pane) = self.panes.panes.get(&id) {
                let cursor = if is_focus { look } else { CursorLook::Hollow };
                self.renderer
                    .draw_into(&pane.term, &mut buffer, w, h, term_area.tuple(), cursor);
            }
        }

        for d in &self.dividers {
            let (x, y, dw, dh) = match d.orientation {
                Orientation::Horizontal => (
                    (d.rect.x + d.rect.w / 2.0).round().max(0.0) as usize,
                    d.rect.y.round().max(0.0) as usize,
                    DIVIDER_DRAW,
                    d.rect.h.round().max(0.0) as usize,
                ),
                Orientation::Vertical => (
                    d.rect.x.round().max(0.0) as usize,
                    (d.rect.y + d.rect.h / 2.0).round().max(0.0) as usize,
                    d.rect.w.round().max(0.0) as usize,
                    DIVIDER_DRAW,
                ),
            };
            self.renderer
                .fill_area(&mut buffer, w, h, (x, y, dw, dh), DIVIDER_COLOR);
        }

        if let Some((rect, alpha, outline)) = preview {
            let area = (
                rect.x.round().max(0.0) as usize,
                rect.y.round().max(0.0) as usize,
                rect.w.round().max(0.0) as usize,
                rect.h.round().max(0.0) as usize,
            );
            self.renderer
                .tint_area(&mut buffer, w, h, area, DROP_TINT, alpha);
            self.renderer
                .outline_area(&mut buffer, w, h, area, DROP_TINT, outline);
        }

        if let Some(open) = self.open.as_ref()
            && let Some(list) = self.themes.as_ref()
        {
            overlay::draw(
                &mut self.renderer,
                &mut buffer,
                (w, h),
                &open.palette,
                list,
                &self.cfg,
                &notes,
            );
        }

        let _ = buffer.present();
        self.dirty = false;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (cw, ch) = self.renderer.cell_size();
        let attrs = Window::default_attributes()
            .with_title("foot-win")
            // Physical, not logical: the cell metrics are already in pixels, so
            // scaling them again by the DPI factor oversizes the window.
            .with_inner_size(winit::dpi::PhysicalSize::new(
                (cw * self.size.columns) as u32,
                (ch * self.size.screen_lines) as u32,
            ));
        let window = Rc::new(event_loop.create_window(attrs).expect("create window"));

        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface =
            softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");

        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(w) = handle.as_raw()
        {
            self.hwnd = w.hwnd.get();
            let cfg = self.cfg.clone();
            self.apply_look(&cfg);
            chrome::set_opacity(self.hwnd as _, cfg.alpha);
        }

        self.blink = match self.cfg.blink {
            config::Blink::System => chrome::caret_blink(),
            config::Blink::Off => None,
            config::Blink::Ms(ms) => Some(Duration::from_millis(ms)),
        };
        if self.blink.is_none() {
            self.blink_on = true;
        }

        self.window = Some(window);
        self._context = Some(context);
        self.surface = Some(surface);

        let (w, h) = self.window_px();
        self.size = self.renderer.grid_size(w, h);
        let waker = self.waker();
        self.panes.open(0, self.size, Some(waker));
        self.relayout();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _: ()) {
        self.pump();
        if let Some(w) = self.window.as_ref()
            && self.dirty
        {
            w.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.open.is_some() && self.palette_event(&event, event_loop) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Focused(focused) => {
                self.focused = focused;
                // The button release lands on whoever stole the focus, so a drag
                // left armed here resumes on the next mouse move.
                if !focused {
                    self.dragging = None;
                    self.moving = None;
                    self.preview = None;
                    self.selecting = false;
                    self.set_cursor(CursorIcon::Default);
                }
                self.restart_blink();
                self.redraw_soon();
            }

            WindowEvent::Resized(size) => {
                // Minimising reports 0x0; resizing to it clamps every grid to one
                // column and reflows the scrollback through it, destroying it.
                if size.width == 0 || size.height == 0 {
                    return;
                }
                self.relayout();
                self.redraw_soon();
            }

            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.mods = Mods {
                    ctrl: s.control_key(),
                    alt: s.alt_key(),
                    shift: s.shift_key(),
                };
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }

                if self.mods.alt
                    && !self.mods.ctrl
                    && let Some(dir) = arrow_direction(&event.logical_key)
                {
                    self.navigate(dir);
                    return;
                }

                if self.mods.ctrl && self.mods.shift {
                    if let Some(zone) = arrow_zone(&event.logical_key) {
                        self.split(zone);
                        return;
                    }
                    if let WKey::Character(c) = &event.logical_key
                        && c.eq_ignore_ascii_case("w")
                    {
                        self.close_focused(event_loop);
                        return;
                    }
                    if let WKey::Character(c) = &event.logical_key
                        && c.eq_ignore_ascii_case("p")
                    {
                        self.open_palette();
                        return;
                    }
                }

                if self.mods.ctrl {
                    let zoom = match &event.logical_key {
                        WKey::Character(c) if c == "+" || c == "=" => Some(1),
                        WKey::Character(c) if c == "-" => Some(-1),
                        WKey::Character(c) if c == "0" => Some(0),
                        _ => None,
                    };
                    if let Some(steps) = zoom {
                        if steps == 0 {
                            self.reset_zoom();
                        } else {
                            self.zoom(steps);
                        }
                        return;
                    }

                    let has_selection =
                        self.panes.focused().is_some_and(|p| p.term.has_selection());
                    match &event.logical_key {
                        WKey::Character(c) if c.eq_ignore_ascii_case("c") => {
                            // Ctrl+C only copies when there is a selection;
                            // otherwise it must stay SIGINT.
                            if (self.mods.shift || has_selection) && self.copy() {
                                return;
                            }
                        }
                        WKey::Character(c) if c.eq_ignore_ascii_case("v") => {
                            self.paste();
                            self.redraw_soon();
                            return;
                        }
                        _ => {}
                    }
                }

                if self.mods.shift {
                    let page = match &event.logical_key {
                        WKey::Named(NamedKey::PageUp) => Some(Scroll::PageUp),
                        WKey::Named(NamedKey::PageDown) => Some(Scroll::PageDown),
                        WKey::Named(NamedKey::Home) => Some(Scroll::Top),
                        WKey::Named(NamedKey::End) => Some(Scroll::Bottom),
                        _ => None,
                    };
                    if let Some(scroll) = page {
                        self.scroll_focused(scroll);
                        return;
                    }
                }

                let text = event.text.as_ref().map(|s| s.as_str());
                if let Some(key) = translate(&event.logical_key, text)
                    && let Some(bytes) = input::encode(&key, self.mods)
                {
                    if let Some(pane) = self.panes.focused_mut() {
                        pane.term.scroll(Scroll::Bottom);
                        pane.term.clear_selection();
                    }
                    self.restart_blink();
                    self.send(&bytes);
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                let previous = self.mouse;
                self.mouse = (position.x, position.y);

                if self.moving.is_some() {
                    let area = self.content();
                    let current = self.preview.as_ref().map(|p| p.target);
                    // move_pane rejects a pane landing next to itself, so
                    // previewing it would promise a drop that never happens.
                    let next = self
                        .panes
                        .drop_target(position.x, position.y, area, current)
                        .filter(|(target, _, _)| Some(*target) != self.moving);
                    if next.map(|(p, z, _)| (p, z)) != current {
                        self.preview = match (next, self.preview.take()) {
                            (Some((pane, zone, rect)), Some(mut p)) => {
                                p.retarget((pane, zone), rect);
                                Some(p)
                            }
                            (Some((pane, zone, rect)), None) => {
                                Some(DropPreview::new((pane, zone), rect))
                            }
                            (None, _) => None,
                        };
                        self.redraw_soon();
                    }
                    return;
                }

                if let Some(id) = self.dragging {
                    let area = self.content();
                    let divider = self.panes.dividers(area).into_iter().find(|d| d.id == id);
                    let delta = match divider.map(|d| d.orientation) {
                        Some(Orientation::Horizontal) => position.x - previous.0,
                        Some(Orientation::Vertical) => position.y - previous.1,
                        None => 0.0,
                    };
                    if delta != 0.0 && self.panes.drag(id, delta as f32, area) {
                        self.relayout();
                        self.redraw_soon();
                    }
                    return;
                }

                let over_close = self.close_button_at(position.x, position.y);
                if over_close != self.hover_close {
                    self.hover_close = over_close;
                    self.redraw_soon();
                }

                let icon = match self.divider_at(position.x, position.y) {
                    Some(d) if d.orientation == Orientation::Horizontal => CursorIcon::ColResize,
                    Some(_) => CursorIcon::RowResize,
                    None if over_close.is_some() => CursorIcon::Pointer,
                    None if self.header_at(position.x, position.y).is_some() => CursorIcon::Grab,
                    None => CursorIcon::Default,
                };
                self.set_cursor(icon);

                if self.selecting
                    && let Some((id, row, col, right)) = self.hit(position.x, position.y)
                    && id == self.panes.focus
                    && let Some(pane) = self.panes.focused_mut()
                {
                    let side = if right { Side::Right } else { Side::Left };
                    pane.term.update_selection(row, col, side);
                    self.redraw_soon();
                }
            }

            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    // A divider takes precedence: its hit area overlaps the panes.
                    if let Some(d) = self.divider_at(self.mouse.0, self.mouse.1) {
                        self.dragging = Some(d.id);
                        return;
                    }

                    // The close button sits inside the header: check it first.
                    if let Some(id) = self.close_button_at(self.mouse.0, self.mouse.1) {
                        if self.panes.len() <= 1 {
                            event_loop.exit();
                        } else {
                            self.end_pane(id);
                            self.redraw_soon();
                        }
                        return;
                    }

                    if let Some(id) = self.header_at(self.mouse.0, self.mouse.1) {
                        self.panes.focus = id;
                        self.moving = Some(id);
                        self.preview = None;
                        self.set_cursor(CursorIcon::Grabbing);
                        self.restart_blink();
                        self.redraw_soon();
                        return;
                    }

                    let Some((id, row, col, right)) = self.hit(self.mouse.0, self.mouse.1) else {
                        return;
                    };
                    if self.panes.focus != id {
                        self.panes.focus = id;
                        self.restart_blink();
                    }

                    let now = Instant::now();
                    let same_spot = self.last_click.is_some_and(|(t, r, c)| {
                        (r, c) == (row, col) && now.duration_since(t) < Duration::from_millis(400)
                    });
                    self.click_streak = if same_spot {
                        (self.click_streak + 1).min(2)
                    } else {
                        0
                    };
                    self.last_click = Some((now, row, col));

                    let ty = match self.click_streak {
                        0 => SelectionType::Simple,
                        1 => SelectionType::Semantic,
                        _ => SelectionType::Lines,
                    };
                    let side = if right { Side::Right } else { Side::Left };
                    if let Some(pane) = self.panes.focused_mut() {
                        pane.term.start_selection(row, col, side, ty);
                    }
                    self.selecting = self.click_streak == 0;
                    self.redraw_soon();
                }
                ElementState::Released => {
                    self.selecting = false;
                    self.dragging = None;

                    if let Some(pane) = self.moving.take() {
                        // The landing slot comes from the target, not from the
                        // rectangle on screen, which may still be sliding.
                        if let Some((target, zone)) = self.preview.take().map(|p| p.target)
                            && self.panes.move_pane(pane, target, zone)
                        {
                            self.relayout();
                        }
                        self.set_cursor(CursorIcon::Default);
                        self.redraw_soon();
                    }
                }
            },

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                self.paste();
                self.redraw_soon();
            }

            WindowEvent::MouseWheel { delta, .. } if self.mods.ctrl => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                    MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
                };
                self.zoom(if up { 1 } else { -1 });
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * LINES_PER_NOTCH as f32) as i32,
                    MouseScrollDelta::PixelDelta(p) => {
                        (p.y / self.renderer.cell_size().1 as f64) as i32
                    }
                };
                if lines == 0 {
                    return;
                }
                let target = self
                    .hit(self.mouse.0, self.mouse.1)
                    .map(|(id, ..)| id)
                    .unwrap_or(self.panes.focus);
                if let Some(pane) = self.panes.panes.get_mut(&target) {
                    pane.term.scroll(Scroll::Delta(lines));
                    self.redraw_soon();
                }
            }

            WindowEvent::RedrawRequested => self.redraw(),

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.panes.panes.is_empty() {
            event_loop.exit();
            return;
        }

        // A pane in its grace period needs one more visit even if the child has
        // gone quiet, or it would never be reaped.
        if let Some(deadline) = self.ending_deadline() {
            self.reap_ending();
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            return;
        }

        if self.preview.as_ref().is_some_and(DropPreview::animating) {
            let now = Instant::now();
            if now >= self.last_frame + PREVIEW_FRAME {
                self.last_frame = now;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_frame + PREVIEW_FRAME));
            return;
        }

        // With no blink nothing is scheduled: the loop sleeps until the next real
        // event, which is what keeps idle CPU at zero.
        let Some(period) = self.blink.filter(|_| self.focused) else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };

        if self.blink_since.elapsed() >= period {
            self.blink_on = !self.blink_on;
            self.blink_since = Instant::now();
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + period));
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.blink_since + period));
        }
    }
}

fn main() -> anyhow::Result<()> {
    let event_loop = EventLoop::<()>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy())?;
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_and_an_unparsable_font_both_fall_back() {
        let bad = std::env::temp_dir().join("foothy-not-a-font.ttf");
        std::fs::write(&bad, b"not a font at all").unwrap();
        let built = build_renderer(bad.to_str(), 16.0, &config::default_palette());
        let _ = std::fs::remove_file(&bad);
        assert!(built.is_ok(), "{:?}", built.err());

        assert!(
            build_renderer(
                Some(r"Z:\does-not-exist.ttf"),
                16.0,
                &config::default_palette()
            )
            .is_ok()
        );
    }

    #[test]
    fn the_preview_starts_where_it_is_and_ends_where_it_goes() {
        let from = Rect::new(0.0, 0.0, 100.0, 100.0);
        let to = Rect::new(50.0, 20.0, 50.0, 80.0);

        let start = lerp_rect(from, to, ease_out(0.0));
        assert_eq!(
            (start.x, start.y, start.w, start.h),
            (0.0, 0.0, 100.0, 100.0)
        );

        let end = lerp_rect(from, to, ease_out(1.0));
        assert_eq!((end.x, end.y, end.w, end.h), (50.0, 20.0, 50.0, 80.0));
    }

    #[test]
    fn the_easing_never_goes_backwards_and_never_overshoots() {
        let mut previous = 0.0;
        for step in 0..=20 {
            let t = ease_out(step as f32 / 20.0);
            assert!(t >= previous, "eased backwards at {step}");
            assert!((0.0..=1.0).contains(&t), "out of range at {step}: {t}");
            previous = t;
        }
        assert_eq!(ease_out(4.0), 1.0);
        assert_eq!(ease_out(-1.0), 0.0);
    }

    #[test]
    fn a_fresh_preview_is_still_and_only_fades() {
        let rect = Rect::new(10.0, 10.0, 40.0, 40.0);
        let p = DropPreview::new((0, DropZone::Left), rect);
        let now = p.rect();
        assert_eq!((now.x, now.y, now.w, now.h), (10.0, 10.0, 40.0, 40.0));
        assert!(p.alpha() < DROP_ALPHA, "the tint must ramp, not appear");
        assert!(p.outline_alpha() < 255, "so must the outline");
        assert!(p.animating());
    }

    #[test]
    fn retargeting_slides_from_where_the_preview_already_is() {
        let mut p = DropPreview::new((0, DropZone::Left), Rect::new(0.0, 0.0, 50.0, 100.0));
        let shown = p.shown;
        p.retarget((0, DropZone::Right), Rect::new(50.0, 0.0, 50.0, 100.0));

        let now = p.rect();
        assert!(now.x < 50.0, "jumped straight to the new slot: {}", now.x);
        assert_eq!(p.shown, shown);
    }

    #[test]
    fn the_preview_stops_animating_and_the_loop_can_sleep_again() {
        let mut p = DropPreview::new((0, DropZone::Left), Rect::new(0.0, 0.0, 50.0, 50.0));
        p.retarget((0, DropZone::Right), Rect::new(50.0, 0.0, 50.0, 50.0));

        std::thread::sleep(PREVIEW_GLIDE.max(PREVIEW_FADE) + Duration::from_millis(20));

        assert!(!p.animating());
        assert_eq!(p.alpha(), DROP_ALPHA);
        assert_eq!(p.outline_alpha(), 255);
        let settled = p.rect();
        assert_eq!((settled.x, settled.w), (50.0, 50.0));
    }

    #[test]
    fn safe_paste_asks_nothing() {
        assert_eq!(warning_for(Risk::Safe, "cargo build"), None);
    }

    #[test]
    fn multiline_warning_names_the_line_count_and_shows_the_start() {
        let text = "rm -rf /tmp/rubbish\ncargo build\n";
        let msg = warning_for(Risk::WouldExecute { lines: 3 }, text).unwrap();
        assert!(msg.contains("3 lines"), "{msg}");
        assert!(msg.contains("rm -rf /tmp/rubbish"), "{msg}");
    }

    #[test]
    fn large_warning_names_the_size() {
        let msg = warning_for(Risk::Large { bytes: 9000 }, "x").unwrap();
        assert!(msg.contains("9000 bytes"), "{msg}");
    }

    #[test]
    fn preview_is_first_line_only_and_bounded() {
        assert_eq!(preview("  hello  \nworld"), "hello");
        let long = "a".repeat(200);
        let shown = preview(&long);
        assert!(shown.chars().count() <= 71, "{}", shown.chars().count());
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn ctrl_shift_arrows_split_towards_that_side() {
        assert_eq!(
            arrow_zone(&WKey::Named(NamedKey::ArrowLeft)),
            Some(DropZone::Left)
        );
        assert_eq!(
            arrow_zone(&WKey::Named(NamedKey::ArrowRight)),
            Some(DropZone::Right)
        );
        assert_eq!(
            arrow_zone(&WKey::Named(NamedKey::ArrowUp)),
            Some(DropZone::Top)
        );
        assert_eq!(
            arrow_zone(&WKey::Named(NamedKey::ArrowDown)),
            Some(DropZone::Bottom)
        );
        assert_eq!(arrow_zone(&WKey::Named(NamedKey::Enter)), None);
    }

    #[test]
    fn the_palette_takes_its_keys_and_turns_the_rest_into_filter_text() {
        let named = |n: NamedKey| palette_key(&WKey::Named(n), None);
        assert_eq!(named(NamedKey::ArrowUp), Some(palette::Key::Up));
        assert_eq!(named(NamedKey::ArrowDown), Some(palette::Key::Down));
        assert_eq!(named(NamedKey::Home), Some(palette::Key::Home));
        assert_eq!(named(NamedKey::End), Some(palette::Key::End));
        assert_eq!(named(NamedKey::Enter), Some(palette::Key::Enter));
        assert_eq!(named(NamedKey::Escape), Some(palette::Key::Esc));
        assert_eq!(named(NamedKey::Backspace), Some(palette::Key::Backspace));

        let typed = |s: &str| palette_key(&WKey::Character(s.into()), Some(s));
        assert_eq!(typed("n"), Some(palette::Key::Char('n')));
        assert_eq!(
            palette_key(&WKey::Named(NamedKey::Space), Some(" ")),
            Some(palette::Key::Char(' '))
        );
        // Tab and Ctrl+letter both arrive as a control character.
        assert_eq!(palette_key(&WKey::Named(NamedKey::Tab), Some("\t")), None);
        assert_eq!(typed("\u{1}"), None);
        assert_eq!(palette_key(&WKey::Named(NamedKey::F5), None), None);
    }

    #[test]
    fn alt_arrows_map_to_directions() {
        assert_eq!(
            arrow_direction(&WKey::Named(NamedKey::ArrowLeft)),
            Some(Direction::Left)
        );
        assert_eq!(
            arrow_direction(&WKey::Named(NamedKey::ArrowDown)),
            Some(Direction::Down)
        );
        assert_eq!(arrow_direction(&WKey::Named(NamedKey::Enter)), None);
    }
}
