//! Headless check of what actually reaches PSReadLine, with and without
//! win32-input-mode. Types a line, sends Ctrl+Backspace, dumps the grid.
//!
//!     cargo run -p platform-win --bin keytest -- legacy
//!     cargo run -p platform-win --bin keytest -- win32

use std::time::{Duration, Instant};

use input::win32::{self, KeyEvent, vk};
use input::{Key, Mods};
use portable_pty::CommandBuilder;
use vt_core::pty::Pty;
use vt_core::{TermSize, Terminal};

const LINE: &str = "hola mundo test";

fn vk_for(c: char) -> u16 {
    match c {
        ' ' => vk::SPACE,
        c => c.to_ascii_uppercase() as u16,
    }
}

const MAPVK_VK_TO_VSC: u32 = 0;

fn scan_code(virtual_key: u16) -> u16 {
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyW(
            virtual_key as u32,
            MAPVK_VK_TO_VSC,
        ) as u16
    }
}

fn win32_press(virtual_key: u16, unicode: u16, mods: Mods) -> Vec<u8> {
    let mut ev = KeyEvent::down(virtual_key, unicode, mods);
    ev.scan_code = scan_code(virtual_key);
    win32::press(&ev)
}

fn main() -> anyhow::Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "win32".into());
    let size = TermSize::new(100, 12);

    let mut term = Terminal::new(size);
    let mut cmd = CommandBuilder::new("powershell.exe");
    cmd.arg("-NoLogo");
    cmd.cwd(std::env::current_dir()?);
    let mut pty = Pty::spawn(cmd, size)?;

    let pump = |pty: &mut Pty, term: &mut Terminal, ms: u64| {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            let bytes = pty.drain(10).bytes;
            if !bytes.is_empty() {
                term.feed(&bytes);
                let replies = term.take_pty_replies();
                if !replies.is_empty() {
                    let _ = pty.write(&replies);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };

    pump(&mut pty, &mut term, 1500);

    if mode == "win32" || mode == "diag" {
        pty.write(win32::ENABLE)?;
        pump(&mut pty, &mut term, 300);
    }

    if mode == "diag" {
        println!("--- tras ENABLE, mando 'x' legacy ---");
        pty.write(b"x")?;
        pump(&mut pty, &mut term, 600);
        println!("linea: {:?}", term.line_text(0));

        println!("--- ahora 'y' en win32 ---");
        pty.write(&win32_press(b'Y' as u16, 'y' as u16, Mods::NONE))?;
        pump(&mut pty, &mut term, 600);
        println!("linea: {:?}", term.line_text(0));

        println!("--- pantalla completa ---");
        println!("{}", term.screen_text());
        return Ok(());
    }

    for c in LINE.chars() {
        let bytes = if mode == "win32" {
            win32_press(vk_for(c), c as u16, Mods::NONE)
        } else {
            input::encode(&Key::Char(c), Mods::NONE).unwrap()
        };
        pty.write(&bytes)?;
        pump(&mut pty, &mut term, 40);
    }
    pump(&mut pty, &mut term, 600);

    let before = term.line_text(0);

    let ctrl_bs = if mode == "win32" {
        win32_press(vk::BACK, 0, Mods::CTRL)
    } else {
        input::encode(&Key::Backspace, Mods::CTRL).unwrap()
    };
    pty.write(&ctrl_bs)?;
    pump(&mut pty, &mut term, 1200);

    let after = term.line_text(0);

    println!("modo     : {mode}");
    println!("antes    : {before}");
    println!("despues  : {after}");

    let strip = |s: &str| s.rsplit('>').next().unwrap_or("").trim().to_string();
    let (b, a) = (strip(&before), strip(&after));
    println!();
    println!("linea antes   : {b:?}");
    println!("linea despues : {a:?}");

    let verdict = if a == "hola mundo" {
        "BORRO LA PALABRA  <-- lo que queremos"
    } else if a == "hola mundo tes" {
        "borro un solo caracter"
    } else if a.contains('^') {
        "INSERTO BASURA en la linea"
    } else {
        "otro resultado"
    };
    println!("veredicto     : {verdict}");

    Ok(())
}
