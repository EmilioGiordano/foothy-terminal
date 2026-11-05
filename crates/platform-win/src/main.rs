// A terminal must not open a console window of its own. Kept in debug so
// eprintln diagnostics still have somewhere to go.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chrome;
mod clipboard;
mod config_file;
mod panes;

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
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{CursorIcon, Window, WindowId};

const FONT_PATH: &str = r"C:\Windows\Fonts\CascadiaMono.ttf";
const LINES_PER_NOTCH: i32 = 3;
/// How long to keep reading a pane after its child exits, so the last output
/// still arrives: ConPTY leaves bytes in flight past the exit.
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
/// Only ticks while the preview is still moving, so idle stays at zero.
const PREVIEW_FRAME: Duration = Duration::from_millis(8);
const CLOSE_HOVER: u32 = 0x00b3_4a4a;
/// Preference order for the close button. Picked at runtime because a font that
/// lacks the glyph draws tofu instead, and the fancier crosses are often absent.
const CLOSE_GLYPHS: [char; 4] = ['×', '╳', '⨯', 'x'];
/// Same reason as the close glyph: a font without it would draw tofu.
const FOCUS_GLYPHS: [char; 3] = ['▍', '▌', '|'];

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
            "Vas a pegar {lines} líneas.\n\n\
             El programa activo no pidió bracketed paste, así que cada salto de línea \
             se ejecuta igual que si apretaras Enter.\n\n\
             Empieza con:\n    {}\n\n¿Pegar de todos modos?",
            preview(text)
        )),
        Risk::Large { bytes } => Some(format!(
            "Vas a pegar {bytes} bytes en una sola línea.\n\n\
             Empieza con:\n    {}\n\n¿Pegar de todos modos?",
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

/// Ctrl+Shift+arrow splits towards that side: the arrow says where the new
/// pane goes, which is the whole point of picking arrows over letters.
fn arrow_zone(key: &WKey) -> Option<DropZone> {
    match key {
        WKey::Named(NamedKey::ArrowLeft) => Some(DropZone::Left),
        WKey::Named(NamedKey::ArrowRight) => Some(DropZone::Right),
        WKey::Named(NamedKey::ArrowUp) => Some(DropZone::Top),
        WKey::Named(NamedKey::ArrowDown) => Some(DropZone::Bottom),
        _ => None,
    }
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

/// Where a carried pane would land. The rectangle slides to its new slot and
/// the tint fades in rather than appearing at full strength, so brushing past a
/// pane no longer flashes. Only the overlay animates: the split itself stays
/// instant, because resizing a terminal means telling its shell about it.
struct DropPreview {
    target: (PaneId, DropZone),
    from: Rect,
    to: Rect,
    /// Restarted on every retarget, for the slide.
    moved: Instant,
    /// Kept across retargets, so the tint fades in once per drag, not per zone.
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

    /// Eased progress of the fade, 0 on appearance and 1 once settled.
    fn fade(&self) -> f32 {
        ease_out(self.shown.elapsed().div_duration_f32(PREVIEW_FADE))
    }

    fn alpha(&self) -> u8 {
        (DROP_ALPHA as f32 * self.fade()).round() as u8
    }

    /// The outline settles opaque. It is the line that says where the pane
    /// lands, and at the tint's own alpha it would not read as a boundary.
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
    /// Size the first pane asks for before the window exists.
    size: TermSize,
    dirty: bool,
    hwnd: isize,
    mouse: (f64, f64),
    selecting: bool,
    /// Rebuilt only when the layout changes. A mouse move used to recompute it
    /// half a dozen times, each allocating a map.
    rects: HashMap<PaneId, Rect>,
    dividers: Vec<Divider>,
    dragging: Option<DividerId>,
    /// Pane being carried by its header, and where it would land.
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
    cfg: config::Config,
}

/// A bad `font=` must not stop the terminal from opening, and a file that
/// exists is not a font: a truncated download or an unsupported container
/// reads fine and fails inside fontdue, so both errors fall back.
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
        Ok(Self {
            proxy,
            window: None,
            surface: None,
            _context: None,
            renderer,
            panes: Panes::new(header_h, cell_w),
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

    /// Headers only earn their line once there is more than one pane to tell apart.
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

    /// Reads every pane, not just the focused one: a background pane must keep
    /// producing output. Bounded per turn (docs/02 §3 rule 7) so a noisy pane
    /// cannot starve the others.
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

        // Work is capped per turn so one noisy pane cannot starve the others, but
        // the leftovers have to be claimed: the waker only fires for new bytes.
        if pending {
            let _ = self.proxy.send_event(());
        }
    }

    /// Marks a pane as ending. The tail is collected over the following turns
    /// rather than by sleeping here: this runs on the UI thread, and stalling it
    /// freezes input and redraw for every other pane.
    fn end_pane(&mut self, id: PaneId) {
        if let Some(pane) = self.panes.panes.get_mut(&id)
            && pane.ending.is_none()
        {
            pane.ending = Some(Instant::now() + TAIL_GRACE);
        }
    }

    /// Finishes panes whose grace period expired. The tail they were kept alive
    /// for arrived through the ordinary reads in [`Self::pump`].
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
                // The last pane: nothing left to show.
                self.panes.panes.remove(&id);
            } else {
                self.relayout();
            }
        }
        // about_to_wait reaches here too, and it schedules no frame of its own:
        // without this the closed pane stays painted until the next input.
        if reaped {
            self.redraw_soon();
        }
    }

    fn ending_deadline(&self) -> Option<Instant> {
        self.panes.panes.values().filter_map(|p| p.ending).min()
    }

    /// Resizes every terminal and its shell to the rectangle the tree gives it,
    /// and refreshes the cached geometry the mouse and the renderer read.
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

    /// Which pane, and which cell inside it, a window point lands on.
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

    fn paste(&mut self) {
        let Some(text) = clipboard::get_text() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let bracketed = self
            .panes
            .focused()
            .is_some_and(|p| p.term.bracketed_paste());

        if self.cfg.paste_warning
            && let Some(message) = warning_for(paste::assess(&text, bracketed), &text)
            && !chrome::confirm(self.hwnd as _, "Confirmar pegado", &message)
        {
            return;
        }

        let text = paste::normalize(&text);
        if bracketed {
            self.send(&paste::bracket(&text));
        } else {
            self.send(text.as_bytes());
        }
        if let Some(pane) = self.panes.focused_mut() {
            pane.term.scroll(Scroll::Bottom);
        }
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

    /// Keeps the cursor solid while you are typing, like every text field.
    fn restart_blink(&mut self) {
        self.blink_on = true;
        self.blink_since = Instant::now();
    }

    /// The pointer shape is the only hint that a divider can be grabbed.
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

        // Sorted so panes always paint in the same order; the cache itself is a
        // map and must not be cloned every frame.
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
                // A marker, not colour alone, says which pane has the keyboard
                // (docs/05 §3.2 row 2).
                let text = if is_focus {
                    format!("{focus_glyph}{label}")
                } else {
                    format!(" {label}")
                };

                let (bx, by, bw, bh) = self.panes.close_button(header);
                // The label stops where the button starts.
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
            // Physical, not logical: the cell metrics come from a font rasterised
            // at a pixel size, so scaling them again by the DPI factor would ask
            // for a window 1.5x too large at 150%.
            .with_inner_size(winit::dpi::PhysicalSize::new(
                (cw * self.size.columns) as u32,
                (ch * self.size.screen_lines) as u32,
            ));
        let window = Rc::new(event_loop.create_window(attrs).expect("crear ventana"));

        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface =
            softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");

        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(w) = handle.as_raw()
        {
            self.hwnd = w.hwnd.get();
            let hwnd = self.hwnd as _;
            chrome::set_dark_titlebar(hwnd, self.cfg.variant == config::Variant::Dark);
            chrome::set_opacity(hwnd, self.cfg.alpha);
        }

        // Blink::System means the system decides, and it already honours the
        // accessibility preference.
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Focused(focused) => {
                self.focused = focused;
                // The button release lands on whoever stole the focus, so a drag
                // left armed here would resume on the next mouse move.
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
                // Minimising reports 0x0. Resizing to it would clamp every grid to
                // one column and reflow the scrollback through it, destroying it.
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

                // Alt+arrows move the focus between panes.
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
                            let base = self.cfg.font_size;
                            self.apply_font_size(base);
                        } else {
                            self.zoom(steps);
                        }
                        return;
                    }

                    let has_selection =
                        self.panes.focused().is_some_and(|p| p.term.has_selection());
                    match &event.logical_key {
                        WKey::Character(c) if c.eq_ignore_ascii_case("c") => {
                            // Ctrl+C only copies when there is a screen selection;
                            // otherwise it must stay SIGINT (docs/05 §1.3).
                            if (self.mods.shift || has_selection) && self.copy_selection() {
                                if let Some(pane) = self.panes.focused_mut() {
                                    pane.term.clear_selection();
                                }
                                self.redraw_soon();
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

                // Shift+PgUp/PgDn navigate the scrollback instead of reaching the shell.
                if self.mods.shift {
                    let page = match &event.logical_key {
                        WKey::Named(NamedKey::PageUp) => Some(Scroll::PageUp),
                        WKey::Named(NamedKey::PageDown) => Some(Scroll::PageDown),
                        WKey::Named(NamedKey::Home) => Some(Scroll::Top),
                        WKey::Named(NamedKey::End) => Some(Scroll::Bottom),
                        _ => None,
                    };
                    if let Some(scroll) = page {
                        if let Some(pane) = self.panes.focused_mut() {
                            pane.term.scroll(scroll);
                        }
                        self.redraw_soon();
                        return;
                    }
                }

                let text = event.text.as_ref().map(|s| s.as_str());
                if let Some(key) = translate(&event.logical_key, text)
                    && let Some(bytes) = input::encode(&key, self.mods)
                {
                    if let Some(pane) = self.panes.focused_mut() {
                        // Typing always returns to the live view, like every terminal.
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

                // Carrying a pane by its header: keep showing where it would land.
                if self.moving.is_some() {
                    let area = self.content();
                    let current = self.preview.as_ref().map(|p| p.target);
                    // A pane cannot land next to itself: move_pane rejects it on
                    // release, so previewing it would promise a drop that never
                    // happens.
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

                    // The close button sits inside the header, so it is checked first.
                    if let Some(id) = self.close_button_at(self.mouse.0, self.mouse.1) {
                        if self.panes.len() <= 1 {
                            event_loop.exit();
                        } else {
                            self.end_pane(id);
                            self.redraw_soon();
                        }
                        return;
                    }

                    // Grabbing a header carries that pane to another slot.
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

                    // Repeated clicks on the same cell widen the selection:
                    // char, then word, then whole line.
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

            // foot binds BTN_MIDDLE to paste (doc/foot.ini.5.scd). There is no
            // primary selection on Windows, so it pastes the clipboard.
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
                // The wheel scrolls the pane under the pointer, not the focused one.
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

        // Without a blinking cursor nothing is scheduled and the loop sleeps
        // until the next real event, which keeps idle CPU at zero.
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
        // Overshoot would put the preview outside the pane it is previewing.
        let mut previous = 0.0;
        for step in 0..=20 {
            let t = ease_out(step as f32 / 20.0);
            assert!(t >= previous, "eased backwards at {step}");
            assert!((0.0..=1.0).contains(&t), "out of range at {step}: {t}");
            previous = t;
        }
        // Past the end it saturates rather than running away.
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

        // It leaves from the old slot, so the move reads as one rectangle.
        let now = p.rect();
        assert!(now.x < 50.0, "jumped straight to the new slot: {}", now.x);
        // And the fade does not restart, or every zone change would blink.
        assert_eq!(p.shown, shown);
    }

    #[test]
    fn the_preview_stops_animating_and_the_loop_can_sleep_again() {
        // This is what keeps idle at 0%: once it settles, about_to_wait stops
        // scheduling frames. A preview that never latches would tick forever.
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
        let text = "rm -rf /tmp/basura\ncargo build\n";
        let msg = warning_for(Risk::WouldExecute { lines: 3 }, text).unwrap();
        assert!(msg.contains("3 líneas"), "{msg}");
        assert!(msg.contains("rm -rf /tmp/basura"), "{msg}");
    }

    #[test]
    fn large_warning_names_the_size() {
        let msg = warning_for(Risk::Large { bytes: 9000 }, "x").unwrap();
        assert!(msg.contains("9000 bytes"), "{msg}");
    }

    #[test]
    fn preview_is_first_line_only_and_bounded() {
        assert_eq!(preview("  hola  \nmundo"), "hola");
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
