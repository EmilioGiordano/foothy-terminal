mod chrome;
mod clipboard;

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use input::paste::{self, Risk};
use input::{Key, Mods};
use portable_pty::CommandBuilder;
use render::Renderer;
use vt_core::pty::Pty;
use vt_core::{Scroll, SelectionType, Side, TermSize, Terminal};
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
            term: Terminal::new(size),
            pty: None,
            mods: Mods::default(),
            size,
            dirty: true,
            hwnd: 0,
            mouse: (0.0, 0.0),
            selecting: false,
            last_click: None,
            click_streak: 0,
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
        if !bytes.is_empty() {
            self.term.feed(&bytes);
            self.dirty = true;
            let replies = self.term.take_pty_replies();
            if !replies.is_empty() {
                self.send(&replies);
            }
        }
        if eof {
            self.pty = None;
        }
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

    fn redraw_soon(&mut self) {
        self.dirty = true;
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn resize_grid(&mut self, width: u32, height: u32) {
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
            .with_inner_size(winit::dpi::LogicalSize::new(
                (cw * self.size.columns) as f64,
                (ch * self.size.screen_lines) as f64,
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
            Some(Box::new(move || {
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
