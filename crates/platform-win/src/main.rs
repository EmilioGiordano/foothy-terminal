use std::num::NonZeroU32;
use std::rc::Rc;

use input::{Key, Mods};
use portable_pty::CommandBuilder;
use render::Renderer;
use vt_core::pty::Pty;
use vt_core::{TermSize, Terminal};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::window::{Window, WindowId};

const FONT_PATH: &str = r"C:\Windows\Fonts\CascadiaMono.ttf";
const FONT_PX: f32 = 16.0;

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
}

impl App {
    fn new(proxy: EventLoopProxy<()>) -> anyhow::Result<Self> {
        let font = std::fs::read(FONT_PATH)?;
        let renderer = Renderer::new(&font, FONT_PX)?;
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
                let text = event.text.as_ref().map(|s| s.as_str());
                if let Some(key) = translate(&event.logical_key, text)
                    && let Some(bytes) = input::encode(&key, self.mods)
                {
                    self.send(&bytes);
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
