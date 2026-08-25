mod chrome;
mod clipboard;

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use input::paste::{self, Risk};
use input::{Key, Mods};
use portable_pty::CommandBuilder;
use render::{CursorLook, Renderer};
use vt_core::pty::Pty;
use vt_core::{CursorShape, Scroll, SelectionType, Side, TermEvent, TermSize, Terminal};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId};

const FONT_PATH: &str = r"C:\Windows\Fonts\CascadiaMono.ttf";
const FONT_PX: f32 = 16.0;
// foot's own default is pad=0x0 (doc/foot.ini.5.scd).
const PAD: usize = 0;
const BG: u32 = 0x0016_2a33;
const FG: u32 = 0x00cf_ecf7;
const ALPHA: u8 = 235;

fn env_hex(key: &str, fallback: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| u32::from_str_radix(v.trim_start_matches('#'), 16).ok())
        .unwrap_or(fallback)
}

fn env_or<T: std::str::FromStr>(key: &str, fallback: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}
const LINES_PER_NOTCH: i32 = 3;

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

struct App {
    proxy: EventLoopProxy<()>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    _context: Option<softbuffer::Context<Rc<Window>>>,
    renderer: Renderer,
    term: Terminal,
    pty: Option<Pty>,
    mods: Mods,
    size: TermSize,
    dirty: bool,
    hwnd: isize,
    mouse: (f64, f64),
    selecting: bool,
    last_click: Option<(Instant, usize, usize)>,
    click_streak: u8,
    focused: bool,
    blink: Option<Duration>,
    blink_on: bool,
    blink_since: Instant,
}

impl App {
    fn new(proxy: EventLoopProxy<()>) -> anyhow::Result<Self> {
        let font_path = std::env::var("FOOTWIN_FONT").unwrap_or_else(|_| FONT_PATH.into());
        let font = std::fs::read(&font_path)?;
        let mut renderer = Renderer::new(&font, env_or("FOOTWIN_FONT_PX", FONT_PX))?;
        renderer.set_padding(env_or("FOOTWIN_PAD", PAD));
        renderer.set_colors(env_hex("FOOTWIN_FG", FG), env_hex("FOOTWIN_BG", BG));
        let size = TermSize::new(100, 30);
        Ok(Self {
            proxy,
            window: None,
            surface: None,
            _context: None,
            renderer,
            term: Terminal::with_cursor(size, CursorShape::Beam),
            pty: None,
            mods: Mods::default(),
            size,
            dirty: true,
            hwnd: 0,
            mouse: (0.0, 0.0),
            selecting: false,
            last_click: None,
            click_streak: 0,
            focused: true,
            blink: None,
            blink_on: true,
            blink_since: Instant::now(),
        })
    }

    fn send(&mut self, bytes: &[u8]) {
        if let Some(pty) = self.pty.as_mut() {
            let _ = pty.write(bytes);
        }
    }

    fn pump_pty(&mut self) {
        let Some(pty) = self.pty.as_mut() else { return };
        let (bytes, eof) = pty.drain(10);
        let exited = pty.child_exited().is_some();
        if !bytes.is_empty() {
            self.term.feed(&bytes);
            self.dirty = true;
            let replies = self.term.take_pty_replies();
            if !replies.is_empty() {
                self.send(&replies);
            }
            // Whatever is not consumed here accumulates for the whole session.
            for event in self.term.drain_events() {
                if let TermEvent::Title(title) = event
                    && let Some(w) = self.window.as_ref()
                {
                    w.set_title(&title);
                }
            }
        }
        if eof || exited {
            self.end_session();
        }
    }

    /// The child exiting says nothing about the pipe being empty, so the tail is
    /// collected before the PTY goes away or the last output is lost.
    fn end_session(&mut self) {
        let Some(pty) = self.pty.as_ref() else { return };
        let quiet_until = Instant::now() + Duration::from_millis(50);
        while Instant::now() < quiet_until {
            let (rest, _) = pty.drain(100);
            if rest.is_empty() {
                std::thread::sleep(Duration::from_millis(2));
            } else {
                self.term.feed(&rest);
            }
        }
        self.pty = None;
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

    fn cell_under_mouse(&self) -> (usize, usize, bool) {
        let (w, h) = self.window_px();
        self.renderer
            .cell_at(self.mouse.0, self.mouse.1, w, h, self.size)
    }

    fn copy_selection(&mut self) -> bool {
        match self.term.selection_text() {
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

        let bracketed = self.term.bracketed_paste();
        if env_or("FOOTWIN_PASTE_WARN", 1) != 0
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
        self.term.scroll(Scroll::Bottom);
    }

    fn zoom(&mut self, steps: i32) {
        let target = render::step_font_size(self.renderer.font_size(), steps);
        self.apply_font_size(target);
    }

    fn apply_font_size(&mut self, px: f32) {
        if !self.renderer.set_font_size(px) {
            return;
        }
        let (w, h) = self.window_px();
        let new = self.renderer.grid_size(w, h);
        if new != self.size {
            self.size = new;
            self.term.resize(new);
            if let Some(pty) = self.pty.as_ref() {
                let _ = pty.resize(new);
            }
        }
        self.redraw_soon();
    }

    fn cursor_look(&self) -> CursorLook {
        if !self.focused {
            return CursorLook::Hollow;
        }
        if !self.blink_on {
            return CursorLook::Hidden;
        }
        match self.term.cursor_style().shape {
            CursorShape::Block => CursorLook::Block,
            CursorShape::Underline => CursorLook::Underline,
            CursorShape::Beam => CursorLook::Beam,
            CursorShape::HollowBlock => CursorLook::Hollow,
            CursorShape::Hidden => CursorLook::Hidden,
        }
    }

    /// Keeps the cursor solid while you are typing, like every text field.
    fn restart_blink(&mut self) {
        self.blink_on = true;
        self.blink_since = Instant::now();
    }

    fn redraw_soon(&mut self) {
        self.dirty = true;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn resize_grid(&mut self, width: u32, height: u32) {
        // Minimising reports 0x0. Resizing to it would clamp the grid to one
        // column and reflow the whole scrollback through it, which destroys it.
        if width == 0 || height == 0 {
            return;
        }
        let new = self.renderer.grid_size(width as usize, height as usize);
        if new != self.size {
            self.size = new;
            self.term.resize(new);
            if let Some(pty) = self.pty.as_ref() {
                let _ = pty.resize(new);
            }
            self.dirty = true;
        }
    }

    fn redraw(&mut self) {
        let look = self.cursor_look();
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else {
            return;
        };
        let phys = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(phys.width), NonZeroU32::new(phys.height)) else {
            return;
        };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        self.renderer.draw(
            &self.term,
            &mut buffer,
            phys.width as usize,
            phys.height as usize,
            look,
        );
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
            .with_title("foot-win — fase 0")
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
            chrome::set_dark_titlebar(hwnd, true);
            chrome::set_opacity(hwnd, env_or("FOOTWIN_ALPHA", ALPHA));
        }

        // Sin variable, manda el sistema (que ya respeta accesibilidad).
        // FOOTWIN_BLINK=0 lo apaga; cualquier otro numero es el periodo en ms.
        self.blink = match std::env::var("FOOTWIN_BLINK")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            None => chrome::caret_blink(),
            Some(0) => None,
            Some(ms) => Some(Duration::from_millis(ms)),
        };
        if self.blink.is_none() {
            self.blink_on = true;
        }

        let phys = window.inner_size();
        self.window = Some(window);
        self._context = Some(context);
        self.surface = Some(surface);
        self.resize_grid(phys.width, phys.height);

        let proxy = self.proxy.clone();
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        match Pty::spawn_with_waker(
            cmd,
            self.size,
            Some(std::sync::Arc::new(move || {
                let _ = proxy.send_event(());
            })),
        ) {
            Ok(pty) => self.pty = Some(pty),
            Err(e) => eprintln!("no se pudo abrir el PTY: {e}"),
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _: ()) {
        self.pump_pty();
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
                self.restart_blink();
                self.redraw_soon();
            }

            WindowEvent::Resized(size) => {
                self.resize_grid(size.width, size.height);
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
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
                if self.mods.ctrl {
                    let zoom = match &event.logical_key {
                        WKey::Character(c) if c == "+" || c == "=" => Some(1),
                        WKey::Character(c) if c == "-" => Some(-1),
                        WKey::Character(c) if c == "0" => Some(0),
                        _ => None,
                    };
                    if let Some(steps) = zoom {
                        if steps == 0 {
                            let base = env_or("FOOTWIN_FONT_PX", FONT_PX);
                            self.apply_font_size(base);
                        } else {
                            self.zoom(steps);
                        }
                        return;
                    }

                    match &event.logical_key {
                        WKey::Character(c) if c.eq_ignore_ascii_case("c") => {
                            // Ctrl+C only copies when there is a screen selection;
                            // otherwise it must stay SIGINT (docs/05 §1.3).
                            if (self.mods.shift || self.term.has_selection())
                                && self.copy_selection()
                            {
                                self.term.clear_selection();
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
                        self.term.scroll(scroll);
                        self.dirty = true;
                        if let Some(w) = self.window.as_ref() {
                            w.request_redraw();
                        }
                        return;
                    }
                }

                let text = event.text.as_ref().map(|s| s.as_str());
                if let Some(key) = translate(&event.logical_key, text)
                    && let Some(bytes) = input::encode(&key, self.mods)
                {
                    // Typing always returns to the live view, like every terminal.
                    self.term.scroll(Scroll::Bottom);
                    self.term.clear_selection();
                    self.restart_blink();
                    self.send(&bytes);
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x, position.y);
                if self.selecting {
                    let (row, col, right) = self.cell_under_mouse();
                    let side = if right { Side::Right } else { Side::Left };
                    self.term.update_selection(row, col, side);
                    self.redraw_soon();
                }
            }

            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (row, col, right) = self.cell_under_mouse();
                match state {
                    ElementState::Pressed => {
                        // Repeated clicks on the same cell widen the selection:
                        // char, then word, then whole line.
                        let now = Instant::now();
                        let same_spot = self.last_click.is_some_and(|(t, r, c)| {
                            (r, c) == (row, col)
                                && now.duration_since(t) < Duration::from_millis(400)
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
                        self.term.start_selection(row, col, side, ty);
                        self.selecting = self.click_streak == 0;
                        self.redraw_soon();
                    }
                    ElementState::Released => self.selecting = false,
                }
            }

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
                if lines != 0 {
                    self.term.scroll(Scroll::Delta(lines));
                    self.dirty = true;
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }

            WindowEvent::RedrawRequested => self.redraw(),

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.pty.is_none() {
            event_loop.exit();
            return;
        }

        // Without a blinking cursor nothing is scheduled and the loop sleeps
        // until the next real event, which keeps idle CPU at zero.
        let Some(period) = self.blink.filter(|_| self.focused) else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };

        let elapsed = self.blink_since.elapsed();
        if elapsed >= period {
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
}
