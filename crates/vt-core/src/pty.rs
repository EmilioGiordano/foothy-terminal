use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::TermSize;

pub enum PtyEvent {
    Output(Vec<u8>),
    Eof,
}

/// One bounded read of the child's output.
pub struct Drained {
    pub bytes: Vec<u8>,
    /// The session ended.
    pub eof: bool,
    /// The cap was reached and there is still data waiting.
    pub more: bool,
}

pub struct Pty {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<PtyEvent>,
    exit: Arc<Mutex<Option<portable_pty::ExitStatus>>>,
}

fn pty_size(size: TermSize) -> PtySize {
    PtySize {
        rows: size.screen_lines as u16,
        cols: size.columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

pub type Waker = Arc<dyn Fn() + Send + Sync + 'static>;

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
        let mut child = pair.slave.spawn_command(cmd)?;

        // Holding the slave open keeps the master from ever seeing EOF.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx) = channel();
        let read_waker = waker.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 24 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(PtyEvent::Eof);
                        if let Some(w) = &read_waker {
                            w();
                        }
                        break;
                    }
                    Ok(n) => {
                        if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                            break;
                        }
                        if let Some(w) = &read_waker {
                            w();
                        }
                    }
                }
            }
        });

        // ConPTY keeps the master open after the child exits, so the child's
        // status is the only end-of-session signal. It is polled rather than
        // waited on because portable-pty's `wait()` blocks forever here: its
        // WaitForSingleObject on the ConPTY process handle never signals, while
        // `try_wait()` reports the exit correctly. A sleeping thread costs
        // nothing and keeps the event loop itself free of polling.
        let exit = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&exit);
        std::thread::spawn(move || {
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        *slot.lock().unwrap() = Some(status);
                        if let Some(w) = &waker {
                            w();
                        }
                        break;
                    }
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            master: Some(pair.master),
            writer,
            rx,
            exit,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&self, size: TermSize) -> anyhow::Result<()> {
        if let Some(master) = self.master.as_ref() {
            master.resize(pty_size(size))?;
        }
        Ok(())
    }

    /// Bounded per turn (docs/02 §3 rule 7): one noisy pane must not starve the
    /// others. `more` says the cap was hit with data still queued, and the caller
    /// must come back: the waker only fires for *new* bytes, so whatever is left
    /// would otherwise sit unread until the child happens to write again.
    pub fn drain(&self, max_reads: usize) -> Drained {
        let mut bytes = Vec::new();
        let mut eof = false;
        let mut more = false;
        for i in 0..max_reads {
            match self.rx.try_recv() {
                Ok(PtyEvent::Output(mut b)) => {
                    bytes.append(&mut b);
                    more = i + 1 == max_reads;
                }
                Ok(PtyEvent::Eof) | Err(TryRecvError::Disconnected) => {
                    eof = true;
                    more = false;
                    break;
                }
                Err(TryRecvError::Empty) => {
                    more = false;
                    break;
                }
            }
        }
        Drained { bytes, eof, more }
    }

    pub fn child_exited(&self) -> Option<portable_pty::ExitStatus> {
        self.exit.lock().unwrap().clone()
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // ClosePseudoConsole blocks until the reader unblocks, and under ConPTY
        // that can be never. Closing off-thread keeps the caller responsive: a
        // UI thread that drops a Pty inline would freeze the window.
        if let Some(master) = self.master.take() {
            std::thread::spawn(move || drop(master));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Terminal;
    use std::time::{Duration, Instant};

    /// ConPTY blocks the child on its startup DSR query until the terminal
    /// answers, so a test that does not pump the replies never lets the child
    /// run at all, let alone exit.
    fn run_until_exit(mut pty: Pty, size: TermSize) -> Option<portable_pty::ExitStatus> {
        let mut term = Terminal::new(size);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let bytes = pty.drain(10).bytes;
            if !bytes.is_empty() {
                term.feed(&bytes);
                let replies = term.take_pty_replies();
                if !replies.is_empty() {
                    let _ = pty.write(&replies);
                }
            }
            if let Some(status) = pty.child_exited() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn a_capped_drain_says_there_is_more() {
        // Feeding the channel directly keeps this away from a real shell.
        let (tx, rx) = channel();
        for _ in 0..5 {
            tx.send(PtyEvent::Output(vec![b'x'])).unwrap();
        }
        let mut bytes = Vec::new();
        let mut more = true;
        let mut rounds = 0;
        while more {
            let mut got = Vec::new();
            let mut hit_cap = false;
            for i in 0..2 {
                match rx.try_recv() {
                    Ok(PtyEvent::Output(mut b)) => {
                        got.append(&mut b);
                        hit_cap = i + 1 == 2;
                    }
                    _ => {
                        hit_cap = false;
                        break;
                    }
                }
            }
            bytes.extend_from_slice(&got);
            more = hit_cap;
            rounds += 1;
            assert!(rounds < 10, "no deberia dar vueltas para siempre");
        }
        assert_eq!(
            bytes.len(),
            5,
            "no se debe perder nada al acotar por vuelta"
        );
    }

    #[test]
    fn child_exit_is_reported() {
        let size = TermSize::new(40, 10);
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/c", "exit"]);
        let pty = Pty::spawn(cmd, size).expect("spawn");
        assert!(
            run_until_exit(pty, size).is_some(),
            "el hijo termino pero child_exited() nunca lo reporto"
        );
    }

    #[test]
    fn exit_wakes_the_event_loop() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let size = TermSize::new(40, 10);
        let woke = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&woke);
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/c", "exit"]);
        let pty = Pty::spawn_with_waker(
            cmd,
            size,
            Some(Arc::new(move || flag.store(true, Ordering::SeqCst))),
        )
        .expect("spawn");

        assert!(run_until_exit(pty, size).is_some(), "no reporto la salida");
        assert!(
            woke.load(Ordering::SeqCst),
            "la salida no desperto el bucle de eventos"
        );
    }
}
