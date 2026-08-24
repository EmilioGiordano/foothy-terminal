use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::TermSize;

pub enum PtyEvent {
    Output(Vec<u8>),
    Eof,
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<PtyEvent>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

fn pty_size(size: TermSize) -> PtySize {
    PtySize {
        rows: size.screen_lines as u16,
        cols: size.columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

pub type Waker = Box<dyn Fn() + Send + 'static>;

impl Pty {
    pub fn spawn(cmd: CommandBuilder, size: TermSize) -> anyhow::Result<Self> {
        Self::spawn_with_waker(cmd, size, None)
    }

    /// `waker` is called whenever bytes arrive, so the UI can stay event-driven
    /// instead of polling (docs/02 §3 rule 10: idle must cost 0% CPU).
    pub fn spawn_with_waker(
        cmd: CommandBuilder,
        size: TermSize,
        waker: Option<Waker>,
    ) -> anyhow::Result<Self> {
        let pair = native_pty_system().openpty(pty_size(size))?;
        let child = pair.slave.spawn_command(cmd)?;

        // Holding the slave open keeps the master from ever seeing EOF.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 24 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(PtyEvent::Eof);
                        if let Some(w) = &waker {
                            w();
                        }
                        break;
                    }
                    Ok(n) => {
                        if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                            break;
                        }
                        if let Some(w) = &waker {
                            w();
                        }
                    }
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            rx,
            child,
        })
    }

    pub fn spawn_default_shell(size: TermSize) -> anyhow::Result<Self> {
        let shell = if cfg!(windows) {
            std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".into())
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
        };
        Self::spawn(CommandBuilder::new(shell), size)
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&self, size: TermSize) -> anyhow::Result<()> {
        self.master.resize(pty_size(size))?;
        Ok(())
    }

    // Bounded per turn (docs/02 §3 rule 7): one noisy pane must not starve the others.
    pub fn drain(&self, max_reads: usize) -> (Vec<u8>, bool) {
        let mut out = Vec::new();
        let mut eof = false;
        for _ in 0..max_reads {
            match self.rx.try_recv() {
                Ok(PtyEvent::Output(mut b)) => out.append(&mut b),
                Ok(PtyEvent::Eof) | Err(TryRecvError::Disconnected) => {
                    eof = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        (out, eof)
    }

    // ConPTY keeps the master open after the child exits, so EOF never arrives.
    // Child status is the real end-of-session signal on Windows.
    pub fn child_exited(&mut self) -> Option<portable_pty::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    pub fn recv(&self) -> Option<PtyEvent> {
        self.rx.recv().ok()
    }
}
